//! LLM translation through a loopback llama-server (OpenAI-compatible chat
//! API), the same out-of-process boundary Nova's catalog reasoner uses: the
//! GGUF runtime never shares the app's address space.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::SegmentTranslator;

fn language_name(code: &str) -> &'static str {
    match code {
        "en" => "English",
        "fr" => "French",
        "ar" => "Arabic",
        _ => "the target language",
    }
}

pub struct LlamaTranslator {
    child: Child,
    port: u16,
    source: String,
    target: String,
    pub load_sec: f64,
}

impl Drop for LlamaTranslator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn post(port: u16, body: &Value) -> Result<Value> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "--max-time",
            "120",
            "-H",
            "Content-Type: application/json",
            "-d",
        ])
        .arg(body.to_string())
        .arg(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!("llama-server request failed");
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

impl LlamaTranslator {
    /// Start llama-server on `model` (GGUF) and wait until it is ready.
    pub fn start(model: &Path) -> Result<Self> {
        let started = Instant::now();
        // A free port, so a stale server can never answer for this one.
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let mut child = Command::new("llama-server")
            .arg("-m")
            .arg(model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["-c", "4096", "-ngl", "99", "--no-webui"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting llama-server")?;
        // Ready once /health answers 200: the server listens before the
        // model has loaded (it answers 503 meanwhile). Bounded wait.
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if let Some(status) = child.try_wait()? {
                bail!("llama-server exited early ({status})");
            }
            let ok = Command::new("curl")
                .args(["-sf", "--max-time", "2"])
                .arg(format!("http://127.0.0.1:{port}/health"))
                .stdout(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                // Healthy and still ours (not a bind failure racing a stale server).
                if child.try_wait()?.is_some() {
                    bail!("llama-server exited after its health check");
                }
                break;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                bail!("llama-server did not become ready");
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Ok(Self {
            child,
            port,
            source: "en".into(),
            target: "fr".into(),
            load_sec: started.elapsed().as_secs_f64(),
        })
    }
}

impl LlamaTranslator {
    /// Switch direction without restarting the server.
    pub fn set_direction(&mut self, source: &str, target: &str) {
        self.source = source.into();
        self.target = target.into();
    }
}

impl SegmentTranslator for LlamaTranslator {
    fn translate(&mut self, text: &str, context: &[String]) -> Result<String> {
        let (src, tgt) = (language_name(&self.source), language_name(&self.target));
        let mut prompt = format!(
            "Translate the following {src} subtitle line into {tgt}. Reply with the translation only.\n"
        );
        if !context.is_empty() {
            prompt.push_str(&format!(
                "Previous lines, for context only (do not translate them):\n{}\n",
                context.join("\n")
            ));
        }
        prompt.push_str(&format!("Line: {text}"));
        let body = json!({
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0,
            "max_tokens": 256,
            "cache_prompt": true,
        });
        let reply = post(self.port, &body)?;
        // A missing translation is an error, never an empty string to score.
        let text = reply["choices"][0]["message"]["content"]
            .as_str()
            .context("llama-server reply has no translation")?;
        Ok(text.trim().to_string())
    }
}
