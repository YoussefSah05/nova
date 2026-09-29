//! Secret-safe logs (M25): every log line passes through
//! [`nova_core::redact_urls`] before it is written, so a URL in an error
//! (ffprobe's stderr naming an Xtream stream, a reqwest error carrying a TMDB
//! `api_key` query) never reaches the log with its credentials, whichever call
//! site formatted it.

use std::io::Write;

use tracing_subscriber::fmt::MakeWriter;

/// Wraps a writer factory: each event is buffered, redacted, then written.
pub struct Redacting<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for Redacting<M> {
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            inner: self.0.make_writer(),
            buf: Vec::new(),
        }
    }
}

/// One event's output, written (redacted) when the formatter drops it — a
/// URL split across several `write` calls is still redacted whole.
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&self.buf);
        let _ = self
            .inner
            .write_all(nova_core::redact_urls(&text).as_bytes());
        let _ = self.inner.flush();
    }
}

/// The app's log subscriber: `RUST_LOG` filtering (as `fmt::init`), stdout,
/// redacted.
pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(Redacting(std::io::stdout))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn logged_urls_lose_their_credentials() {
        let sink = Sink::default();
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(Redacting(move || writer.clone()))
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let error = "ffprobe failed: http://h.tv:80/live/alice/s3cret/1.ts: 403";
            tracing::warn!(%error, url = "https://api.example/3?api_key=k3y", "probe failed");
        });
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("probe failed"), "{out}");
        assert!(out.contains("http://h.tv:80/…"), "{out}");
        for secret in ["alice", "s3cret", "k3y"] {
            assert!(!out.contains(secret), "{secret} logged: {out}");
        }
    }
}

/// The webview's Content-Security-Policy (M25) stays strict: its own scripts
/// only, network only to the local proxy and Tauri IPC, no plugins or frames.
#[cfg(test)]
mod csp_tests {
    fn policy(key: &str) -> serde_json::Map<String, serde_json::Value> {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        config["app"]["security"][key]
            .as_object()
            .unwrap_or_else(|| panic!("{key} must be set"))
            .clone()
    }

    #[test]
    fn the_webview_runs_under_a_strict_policy() {
        for key in ["csp", "devCsp"] {
            let csp = policy(key);
            let directive = |name: &str| csp[name].as_str().unwrap().to_string();
            assert_eq!(directive("default-src"), "'self'", "{key}");
            assert_eq!(directive("object-src"), "'none'", "{key}");
            assert_eq!(directive("frame-src"), "'none'", "{key}");
            let all = serde_json::to_string(&csp).unwrap();
            assert!(!all.contains("unsafe-eval"), "{key}");
            // Media and fetches: the local proxy and IPC, never the internet.
            for name in ["connect-src", "media-src"] {
                for source in directive(name).split_whitespace() {
                    assert!(
                        !matches!(source, "*" | "https:" | "http:"),
                        "{key} {name} allows {source}"
                    );
                }
            }
        }
        assert_eq!(
            policy("csp")["script-src"].as_str(),
            Some("'self'"),
            "no inline script in production"
        );
    }
}
