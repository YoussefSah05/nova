//! Apple Translation (on-device, `TranslationSession`) through the Swift
//! helper, one persistent process per direction fed line by line.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{bail, Context, Result};

use super::SegmentTranslator;

pub struct AppleTranslator {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl AppleTranslator {
    /// Requires the language pair to be installed (System Settings →
    /// General → Language & Region → Translation Languages).
    pub fn start(helper: &Path, source: &str, target: &str) -> Result<Self> {
        let mut child = Command::new(helper)
            .args(["translate", source, target])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("starting nova-apple-helper")?;
        let stdin = child.stdin.take().context("helper stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("helper stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
}

impl Drop for AppleTranslator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl SegmentTranslator for AppleTranslator {
    fn translate(&mut self, text: &str, _context: &[String]) -> Result<String> {
        writeln!(self.stdin, "{}", text.replace('\n', " "))?;
        self.stdin.flush()?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            bail!("apple helper exited (is the language pair installed?)");
        }
        let v: serde_json::Value = serde_json::from_str(&line)?;
        if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
            bail!("apple translation: {e}");
        }
        Ok(v["text"].as_str().unwrap_or_default().to_string())
    }
}
