//! Install pinned-file models from their manifest `source`.
//!
//! Every byte is checked against the pinned size and sha256 while it streams;
//! a file only appears under its real name once it matches, and the finished
//! directory is attested so the runtime loads it without hashing again.
//! Errors name files, never URLs, so they are safe to show and log.

use std::path::{Path, PathBuf};

use nova_core::model_manifest::{attest, is_attested, ManifestEntry, ManifestFile, VerifyError};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("model {0} has no download source")]
    NoSource(String),
    #[error("could not download {0}; check the connection and try again")]
    Fetch(String),
    #[error("download of {0} was refused (HTTP {1})")]
    Status(String, u16),
    #[error("{0} did not match its pinned checksum")]
    Mismatch(String),
    #[error("could not write model files: {0}")]
    Io(#[from] std::io::Error),
}

/// Bytes still to download: files whose size differs from the pin. Files of
/// the right size are hashed by [`attest`] instead of being fetched again.
pub fn pending_bytes(model_dir: &Path, entry: &ManifestEntry) -> u64 {
    if is_attested(model_dir, entry) {
        return 0;
    }
    entry
        .files
        .iter()
        .filter(|file| !has_pinned_size(model_dir, file))
        .map(|file| file.bytes)
        .sum()
}

fn has_pinned_size(model_dir: &Path, file: &ManifestFile) -> bool {
    model_dir
        .join(&file.path)
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() == file.bytes)
}

/// Download whatever is missing, then attest the whole directory. A file of
/// the right size but the wrong content is deleted and fetched once more.
/// `on_progress` receives the byte count of each written chunk.
pub async fn install_pinned(
    client: &reqwest::Client,
    model_dir: &Path,
    entry: &ManifestEntry,
    mut on_progress: impl FnMut(u64),
) -> Result<(), InstallError> {
    if is_attested(model_dir, entry) {
        return Ok(());
    }
    let source = entry
        .source
        .as_deref()
        .ok_or_else(|| InstallError::NoSource(entry.id.clone()))?;
    for attempt in 0..2 {
        for file in &entry.files {
            if !has_pinned_size(model_dir, file) {
                let url = format!("{source}/{}", file.path);
                download_verified(client, &url, model_dir, file, &mut on_progress).await?;
            }
        }
        let dir = model_dir.to_path_buf();
        let pinned = entry.clone();
        let verified = tokio::task::spawn_blocking(move || attest(&dir, &pinned))
            .await
            .map_err(|_| InstallError::Io(std::io::Error::other("verification stopped")))?;
        match verified {
            Ok(()) => return Ok(()),
            Err(VerifyError::Mismatch(path)) if attempt == 0 => {
                tokio::fs::remove_file(model_dir.join(&path)).await?;
            }
            Err(VerifyError::Mismatch(path)) => return Err(InstallError::Mismatch(path)),
            // Changed while hashing (or vanished): fetch whatever is now off.
            Err(VerifyError::Unreadable(_)) if attempt == 0 => {}
            Err(VerifyError::Unreadable(path)) => {
                return Err(InstallError::Io(std::io::Error::other(format!(
                    "{path} could not be verified"
                ))))
            }
        }
    }
    unreachable!("the second attempt always returns")
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    model_dir: &Path,
    file: &ManifestFile,
    on_progress: &mut impl FnMut(u64),
) -> Result<(), InstallError> {
    let dest = model_dir.join(&file.path);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let fetch = || InstallError::Fetch(file.path.clone());
    let mut response = client.get(url).send().await.map_err(|_| fetch())?;
    if !response.status().is_success() {
        return Err(InstallError::Status(
            file.path.clone(),
            response.status().as_u16(),
        ));
    }
    let partial = partial_path(&dest);
    let result = async {
        let mut out = tokio::fs::File::create(&partial).await?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| fetch())? {
            total += chunk.len() as u64;
            // Never write past the pin: a wrong or hostile body stops early.
            if total > file.bytes {
                return Err(InstallError::Mismatch(file.path.clone()));
            }
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            on_progress(chunk.len() as u64);
        }
        out.sync_all().await?;
        let digest = format!("{:x}", hasher.finalize());
        if total != file.bytes || digest != file.sha256 {
            return Err(InstallError::Mismatch(file.path.clone()));
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(tokio::fs::rename(&partial, &dest).await?),
        Err(error) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::model_manifest::ModelManifest;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    /// Serve `body` for every request; returns the base URL and a request counter.
    fn serve(body: &'static [u8]) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn entry(source: &str) -> ManifestEntry {
        let json = format!(
            r#"{{"schema":1,"models":[{{"id":"opus-mt-en-fr","revision":"r1",
            "capability":"translation","backend":"onnx-seq2seq","relative_path":"opus/en-fr",
            "probe":"files","files":[{{"path":"onnx/model.onnx","sha256":"{HELLO_SHA}","bytes":5}}],
            "pairs":[{{"source":"en","target":"fr"}}],"licence":"CC-BY-4.0",
            "latency_class":"streaming","source":"https://example.com/m"}}]}}"#
        );
        let mut entry = ModelManifest::parse(&json).unwrap().models.remove(0);
        // Tests serve plain HTTP on loopback; the manifest itself requires https.
        entry.source = Some(source.to_string());
        entry
    }

    #[tokio::test]
    async fn installs_verifies_and_attests_then_skips_the_network() {
        let (base, hits) = serve(b"hello");
        let dir = tempfile::tempdir().unwrap();
        let entry = entry(&base);
        assert_eq!(pending_bytes(dir.path(), &entry), 5);
        let mut seen = 0;
        install_pinned(&reqwest::Client::new(), dir.path(), &entry, |n| seen += n)
            .await
            .unwrap();
        assert_eq!(seen, 5);
        assert!(is_attested(dir.path(), &entry));
        assert_eq!(pending_bytes(dir.path(), &entry), 0);
        install_pinned(&reqwest::Client::new(), dir.path(), &entry, |_| {})
            .await
            .unwrap();
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn wrong_content_is_refused_and_leaves_nothing_behind() {
        let (base, _) = serve(b"HELLO");
        let dir = tempfile::tempdir().unwrap();
        let entry = entry(&base);
        let error = install_pinned(&reqwest::Client::new(), dir.path(), &entry, |_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Mismatch(ref p) if p == "onnx/model.onnx"));
        assert!(!dir.path().join("onnx/model.onnx").exists());
        assert!(!dir.path().join("onnx/model.onnx.part").exists());
        assert!(!is_attested(dir.path(), &entry));
    }

    #[tokio::test]
    async fn an_oversized_body_stops_at_the_pin() {
        let (base, _) = serve(b"hello, and much more than was pinned");
        let dir = tempfile::tempdir().unwrap();
        let error = install_pinned(&reqwest::Client::new(), dir.path(), &entry(&base), |_| {})
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Mismatch(_)));
    }

    #[tokio::test]
    async fn a_same_size_corrupt_file_is_replaced() {
        let (base, hits) = serve(b"hello");
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("onnx")).unwrap();
        std::fs::write(dir.path().join("onnx/model.onnx"), b"jello").unwrap();
        let entry = entry(&base);
        assert_eq!(pending_bytes(dir.path(), &entry), 0);
        install_pinned(&reqwest::Client::new(), dir.path(), &entry, |_| {})
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("onnx/model.onnx")).unwrap(),
            b"hello"
        );
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Real network: `NOVA_TEST_INSTALL_OPUS_TO=/path/to/opus` installs every
    /// bundled translation model there, laid out for `NOVA_TEST_OPUS_DIR`.
    #[tokio::test]
    async fn bundled_translation_models_install_from_their_pinned_sources() {
        let Some(root) = std::env::var_os("NOVA_TEST_INSTALL_OPUS_TO") else {
            return;
        };
        let manifest = crate::model_registry::bundled_manifest().unwrap();
        let client = reqwest::Client::new();
        for entry in manifest.models.iter().filter(|e| e.source.is_some()) {
            let dir = std::path::PathBuf::from(&root).join(&entry.id);
            install_pinned(&client, &dir, entry, |_| {}).await.unwrap();
            assert!(is_attested(&dir, entry), "{}", entry.id);
            assert_eq!(pending_bytes(&dir, entry), 0);
        }
    }

    #[tokio::test]
    async fn errors_never_carry_the_url() {
        let dir = tempfile::tempdir().unwrap();
        let entry = entry("http://127.0.0.1:9/secret-token");
        let error = install_pinned(&reqwest::Client::new(), dir.path(), &entry, |_| {})
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("secret-token"), "{error}");
    }
}
