//! The app side of the ASR worker: spawn it, wait for it to be ready, and
//! send it decode requests. A worker that dies, hangs or answers garbage is
//! killed and replaced on the next request; the caller sees one failed step.

use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
            // Decode-time failures; start-time ones are mapped to LoadFailed
            // where the worker is started.
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
///
/// The worker is owned by the runtime's model backend, which calls
/// [`WhisperWorker::shutdown`] when the model is unloaded or evicted. After
/// that no process runs and none is started, even if a session still holds
/// this worker: memory is only used while the runtime accounts for it.
pub struct WhisperWorker {
    spec: WorkerSpec,
    slot: Mutex<Slot>,
    /// The running worker's pid (0: none). Written only under `slot`.
    pid: AtomicU32,
    /// [`Accel`] of the latest decode, readable without waiting on a decode.
    accel: AtomicU8,
    closed: AtomicBool,
}

#[derive(Default)]
struct Slot {
    process: Option<WorkerProcess>,
    /// Consecutive failed starts, for backoff.
    failures: u32,
    retry_at: Option<Instant>,
}

/// Wait before restarting a worker that failed to start: 2, 4, 8 … 60 s.
/// A model that crashes whisper.cpp on load is not reloaded on every step.
fn restart_backoff(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(6)).min(60))
}

fn accel_code(accel: Option<Accel>) -> u8 {
    match accel {
        None => 0,
        Some(Accel::Metal) => 1,
        Some(Accel::Coreml) => 2,
        Some(Accel::Cpu) => 3,
    }
}

impl WhisperWorker {
    /// Start the worker and wait until it can decode (the Metal path; the
    /// Core ML upgrade continues inside the worker).
    pub fn start(spec: WorkerSpec) -> Result<Self, WorkerError> {
        let process = WorkerProcess::spawn(&spec)?;
        let worker = Self {
            spec,
            pid: AtomicU32::new(process.child.id()),
            accel: AtomicU8::new(accel_code(Some(process.accel))),
            slot: Mutex::new(Slot {
                process: Some(process),
                ..Slot::default()
            }),
            closed: AtomicBool::new(false),
        };
        Ok(worker)
    }

    /// The encoder the latest decode ran on, if a worker is running.
    pub fn accel(&self) -> Option<Accel> {
        match self.accel.load(Ordering::Acquire) {
            1 => Some(Accel::Metal),
            2 => Some(Accel::Coreml),
            3 => Some(Accel::Cpu),
            _ => None,
        }
    }

    /// Stop the worker for good. Safe while a decode is running: that
    /// decode fails, and no new worker is started afterwards.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        match self.slot.try_lock() {
            Ok(mut slot) => {
                // Dropping the process kills and reaps it.
                slot.process = None;
                self.pid.store(0, Ordering::Release);
                self.accel.store(0, Ordering::Release);
            }
            Err(_) => {
                // A decode holds the slot, so its child is alive and not yet
                // reaped: signalling the pid cannot hit a reused one. The
                // decode then sees `closed` and drops the process.
                let pid = self.pid.load(Ordering::Acquire);
                #[cfg(unix)]
                if pid != 0 {
                    // SAFETY: see above; the pid belongs to our live child.
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Slot> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clear(&self, slot: &mut Slot) {
        slot.process = None;
        self.pid.store(0, Ordering::Release);
        self.accel.store(0, Ordering::Release);
    }
}

impl BufferDecoder for WhisperWorker {
    fn decode(&self, request: &DecodeRequest, pcm: &[f32]) -> Result<Decoded, EngineError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(EngineError::LoadFailed);
        }
        let mut slot = self.lock();
        if slot.process.is_none() {
            if slot.retry_at.is_some_and(|at| Instant::now() < at) {
                return Err(EngineError::LoadFailed);
            }
            match WorkerProcess::spawn(&self.spec) {
                Ok(process) => {
                    self.pid.store(process.child.id(), Ordering::Release);
                    self.accel
                        .store(accel_code(Some(process.accel)), Ordering::Release);
                    slot.process = Some(process);
                    slot.failures = 0;
                    slot.retry_at = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "ASR worker restart failed");
                    slot.failures += 1;
                    slot.retry_at = Some(Instant::now() + restart_backoff(slot.failures));
                    return Err(EngineError::LoadFailed);
                }
            }
        }
        let result = match slot.process.as_mut() {
            Some(process) => process.decode(request, pcm),
            None => return Err(EngineError::LoadFailed),
        };
        if self.closed.load(Ordering::Acquire) {
            self.clear(&mut slot);
            return Err(EngineError::LoadFailed);
        }
        match result {
            Ok(decoded) => {
                self.accel
                    .store(accel_code(Some(decoded.accel)), Ordering::Release);
                Ok(decoded)
            }
            // A decode error reported by a healthy worker leaves it running.
            Err(WorkerError::Failed(_)) => Err(EngineError::InferenceFailed),
            Err(_) => {
                // Dead or hung: drop (and kill) it; the next step restarts it.
                self.clear(&mut slot);
                Err(EngineError::InferenceFailed)
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
            max_len: 0,
            timing: None,
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
    fn shutdown_stops_the_worker_and_prevents_restarts() {
        let (dir, spec) = script_worker(
            r#"echo $$ >> "$(dirname "$0")/spawns"
echo '{"ready":{"accel":"metal","load_ms":5}}'
while read header; do head -c 8 > /dev/null; echo '{"decoded":{"words":[],"language":null,"accel":"metal","decode_ms":1}}'; done"#,
        );
        let worker = WhisperWorker::start(spec).unwrap();
        assert!(worker.decode(&request(2), &[0.0, 0.0]).is_ok());
        let pid = std::fs::read_to_string(dir.path().join("spawns")).unwrap();
        worker.shutdown();
        assert!(worker.accel().is_none());
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(!alive, "the worker process is gone");
        assert_eq!(
            worker.decode(&request(2), &[0.0, 0.0]),
            Err(EngineError::LoadFailed)
        );
        let spawns = std::fs::read_to_string(dir.path().join("spawns")).unwrap();
        assert_eq!(spawns.lines().count(), 1, "no restart after shutdown");
    }

    #[test]
    fn failed_restarts_back_off() {
        // Ready once, then exits; every later start fails at load.
        let (dir, spec) = script_worker(
            r#"if [ -e "$(dirname "$0")/started" ]; then echo '{"failed":"bad model"}'; exit 0; fi
touch "$(dirname "$0")/started"
echo '{"ready":{"accel":"metal","load_ms":5}}'"#,
        );
        let worker = WhisperWorker::start(spec).unwrap();
        assert_eq!(
            worker.decode(&request(2), &[0.0, 0.0]),
            Err(EngineError::InferenceFailed),
            "the first worker exited"
        );
        assert_eq!(
            worker.decode(&request(2), &[0.0, 0.0]),
            Err(EngineError::LoadFailed),
            "its restart fails to load"
        );
        let before = std::fs::metadata(dir.path().join("started")).unwrap();
        assert_eq!(
            worker.decode(&request(2), &[0.0, 0.0]),
            Err(EngineError::LoadFailed),
            "backing off: no new start"
        );
        assert!(before.is_file());
        assert_eq!(restart_backoff(1), Duration::from_secs(2));
        assert_eq!(restart_backoff(30), Duration::from_secs(60));
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
        assert!(matches!(
            WhisperWorker::start(spec),
            Err(WorkerError::Spawn(_))
        ));
    }
}
