//! The app side of the ASR worker: spawn it, wait for it to be ready, and
//! send it decode requests. A worker that dies, hangs or answers garbage is
//! killed and replaced on the next request; the caller sees one failed step.

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use nova_core::intelligence::EngineError;
use thiserror::Error;

use super::protocol::{read_line, write_decode, Accel, DecodeRequest, Decoded, Response};
use super::worker::WORKER_FLAG;

/// Generous: a Metal load from a slow external disk. The Core ML compile
/// happens after ready, off this path.
const READY_DEADLINE: Duration = Duration::from_secs(90);
/// A decode of a full 30 s window takes well under a second; anything near
/// this is a hung worker.
const DECODE_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("could not start the ASR worker: {0}")]
    Spawn(std::io::Error),
    #[error("the ASR worker stopped responding")]
    Exited,
    #[error("the ASR worker failed: {0}")]
    Failed(String),
}

impl From<WorkerError> for EngineError {
    fn from(error: WorkerError) -> Self {
        match error {
            WorkerError::Spawn(_) => EngineError::LoadFailed,
            WorkerError::Exited | WorkerError::Failed(_) => EngineError::InferenceFailed,
        }
    }
}

/// How to start a worker: the program (normally the app's own executable,
/// which enters worker mode on [`WORKER_FLAG`]) and the GGML model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSpec {
    pub program: PathBuf,
    pub model: PathBuf,
}

impl WorkerSpec {
    /// A worker run by the current executable.
    pub fn current_exe(model: PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            program: std::env::current_exe()?,
            model,
        })
    }
}

pub struct WorkerProcess {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    pub accel: Accel,
    pub load_ms: u64,
}

/// Kill the worker if `work` has not finished within `deadline`: its pipe
/// then closes and the blocked read returns.
fn with_deadline<T>(pid: u32, deadline: Duration, work: impl FnOnce() -> T) -> T {
    let (done, finished) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if finished.recv_timeout(deadline) == Err(mpsc::RecvTimeoutError::Timeout) {
            #[cfg(unix)]
            // SAFETY: signalling our own child by pid; it has not been
            // reaped yet because the caller still owns its `Child`.
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    });
    let result = work();
    let _ = done.send(());
    let _ = watchdog.join();
    result
}

impl WorkerProcess {
    pub fn spawn(spec: &WorkerSpec) -> Result<Self, WorkerError> {
        let mut child = Command::new(&spec.program)
            .arg(WORKER_FLAG)
            .arg(&spec.model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(WorkerError::Spawn)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(WorkerError::Exited);
        };
        let mut worker = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            accel: Accel::Cpu,
            load_ms: 0,
        };
        let pid = worker.child.id();
        let stdout = &mut worker.stdout;
        match with_deadline(pid, READY_DEADLINE, || read_line::<Response>(stdout)) {
            Ok(Some(Response::Ready(ready))) => {
                worker.accel = ready.accel;
                worker.load_ms = ready.load_ms;
                Ok(worker)
            }
            Ok(Some(Response::Failed(message))) => Err(WorkerError::Failed(message)),
            _ => Err(WorkerError::Exited),
        }
    }

    pub fn decode(&mut self, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, WorkerError> {
        let pid = self.child.id();
        let (stdin, stdout) = (&mut self.stdin, &mut self.stdout);
        let response = with_deadline(pid, DECODE_DEADLINE, || {
            write_decode(stdin, request, pcm)?;
            read_line::<Response>(stdout)
        });
        match response {
            Ok(Some(Response::Decoded(decoded))) => {
                self.accel = decoded.accel;
                Ok(decoded)
            }
            Ok(Some(Response::Failed(message))) => Err(WorkerError::Failed(message)),
            _ => Err(WorkerError::Exited),
        }
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes one whole audio buffer: the seam between the streaming session
/// and whatever runs the model (the worker, or a fake in tests).
pub trait BufferDecoder: Send + Sync {
    fn decode(&self, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, EngineError>;
}

/// A resident whisper.cpp worker shared by every session of one model.
/// Requests are serialized: one decode at a time per model, which is also
/// what the runtime's per-model admission allows.
pub struct WhisperWorker {
    spec: WorkerSpec,
    process: Mutex<Option<WorkerProcess>>,
}

impl WhisperWorker {
    /// Start the worker and wait until it can decode (the Metal path; the
    /// Core ML upgrade continues inside the worker).
    pub fn start(spec: WorkerSpec) -> Result<Self, WorkerError> {
        let process = WorkerProcess::spawn(&spec)?;
        Ok(Self {
            spec,
            process: Mutex::new(Some(process)),
        })
    }

    /// The encoder the last decode ran on, if a worker is running.
    pub fn accel(&self) -> Option<Accel> {
        self.lock().as_ref().map(|p| p.accel)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<WorkerProcess>> {
        self.process.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl BufferDecoder for WhisperWorker {
    fn decode(&self, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, EngineError> {
        let mut slot = self.lock();
        if slot.is_none() {
            *slot = Some(WorkerProcess::spawn(&self.spec)?);
        }
        let Some(process) = slot.as_mut() else {
            return Err(EngineError::LoadFailed);
        };
        match process.decode(request, pcm) {
            Ok(decoded) => Ok(decoded),
            // A decode error reported by a healthy worker leaves it running.
            Err(WorkerError::Failed(_)) => Err(EngineError::InferenceFailed),
            Err(error) => {
                // Dead or hung: drop (and kill) it; the next step respawns.
                *slot = None;
                Err(error.into())
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A stand-in worker: a shell script that speaks the protocol.
    fn script_worker(body: &str) -> (tempfile::TempDir, WorkerSpec) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("worker.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let spec = WorkerSpec {
            program,
            model: dir.path().join("model.bin"),
        };
        (dir, spec)
    }

    fn request(samples: usize) -> DecodeRequest {
        DecodeRequest {
            samples,
            offset_sec: 0.0,
            prompt: String::new(),
            language: None,
        }
    }

    #[test]
    fn a_ready_worker_answers_and_a_dead_one_is_replaced() {
        // Answers one decode (reading its header line and 2 samples = 8
        // bytes), then exits.
        let (_dir, spec) = script_worker(
            r#"echo '{"ready":{"accel":"metal","load_ms":5}}'
read header
head -c 8 > /dev/null
echo '{"decoded":{"words":[{"text":"hi","start":0.0,"end":0.4}],"language":"en","accel":"coreml","decode_ms":3}}'"#,
        );
        let worker = WhisperWorker::start(spec).unwrap();
        assert_eq!(worker.accel(), Some(Accel::Metal));
        let decoded = worker.decode(&request(2), &[0.0, 0.0]).unwrap();
        assert_eq!(decoded.words[0].text, "hi");
        assert_eq!(worker.accel(), Some(Accel::Coreml));
        // The script has exited: this step fails and the worker is dropped...
        assert_eq!(
            worker.decode(&request(2), &[0.0, 0.0]),
            Err(EngineError::InferenceFailed)
        );
        assert!(worker.accel().is_none());
        // ...and the next step starts a fresh one.
        assert!(worker.decode(&request(2), &[0.0, 0.0]).is_ok());
    }

    #[test]
    fn a_worker_that_fails_to_load_reports_why() {
        let (_dir, spec) = script_worker(r#"echo '{"failed":"no such model"}'"#);
        assert!(matches!(
            WhisperWorker::start(spec),
            Err(WorkerError::Failed(message)) if message == "no such model"
        ));
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let spec = WorkerSpec {
            program: PathBuf::from("/nonexistent/nova-asr-worker"),
            model: PathBuf::from("model.bin"),
        };
        assert!(matches!(WhisperWorker::start(spec), Err(WorkerError::Spawn(_))));
    }
}
