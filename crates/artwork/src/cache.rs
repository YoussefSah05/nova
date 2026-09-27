//! On-disk artwork cache: memory (the webview) → disk → network only if needed.
//!
//! Files live at `<dir>/<ab>/<sha256(url)>`, holding the validated body. A
//! failed download leaves a `<hash>.miss` marker so a dead link is not retried
//! for [`MISS_TTL`]. Network fetches are bounded (concurrency, time, size) and
//! concurrent requests for one URL share a single download. The cache is
//! disposable: [`ArtworkCache::evict`] trims it to its byte budget, least
//! recently used first, and [`ArtworkCache::clear`] empties it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::validate::{self, ImageFormat, Rejection, MAX_BYTES};

/// How long a failed URL is not retried.
pub const MISS_TTL: Duration = Duration::from_secs(12 * 60 * 60);
/// Default byte budget of the cache.
pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Simultaneous network downloads.
const FETCH_CONCURRENCY: usize = 8;
/// A hit refreshes the file's recency at most this often (fewer writes).
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// New files between automatic evictions.
const EVICT_EVERY: u32 = 256;
const USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Nova";

#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error("not an artwork URL")]
    InvalidUrl,
    #[error("artwork unavailable: {0}")]
    Unavailable(String),
    #[error("artwork rejected: {0}")]
    Rejected(Rejection),
    #[error("artwork cache I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// A servable image.
#[derive(Debug, Clone)]
pub struct CachedImage {
    pub bytes: Vec<u8>,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub files: u64,
    pub bytes: u64,
}

pub struct ArtworkCache {
    dir: PathBuf,
    client: reqwest::Client,
    max_bytes: u64,
    fetch_permits: Semaphore,
    inflight: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    writes_since_evict: AtomicU32,
    tmp_counter: AtomicU64,
}

impl ArtworkCache {
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap_or_default();
        Self {
            dir,
            client,
            max_bytes,
            fetch_permits: Semaphore::new(FETCH_CONCURRENCY),
            inflight: Mutex::new(HashMap::new()),
            writes_since_evict: AtomicU32::new(0),
            tmp_counter: AtomicU64::new(0),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn paths(&self, url: &str) -> (String, PathBuf, PathBuf) {
        let hash = format!("{:x}", Sha256::digest(url.as_bytes()));
        let shard = self.dir.join(&hash[..2]);
        let file = shard.join(&hash);
        let miss = shard.join(format!("{hash}.miss"));
        (hash, file, miss)
    }

    /// The image behind `url`, from disk when cached, otherwise downloaded,
    /// validated and stored.
    pub async fn get(&self, url: &str) -> Result<CachedImage, ArtworkError> {
        let url = nova_core::artwork::plausible_image_url(url).ok_or(ArtworkError::InvalidUrl)?;
        let (hash, file, miss) = self.paths(&url);
        if let Some(hit) = read_hit(&file).await {
            return Ok(hit);
        }
        if fresh_marker(&miss).await {
            return Err(ArtworkError::Unavailable("recently failed".into()));
        }

        // One download per URL: later callers wait, then find the file.
        let lock = {
            let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            map.entry(hash.clone()).or_default().clone()
        };
        let result = {
            let _guard = lock.lock().await;
            match read_hit(&file).await {
                Some(hit) => Ok(hit),
                None if fresh_marker(&miss).await => {
                    Err(ArtworkError::Unavailable("recently failed".into()))
                }
                None => self.download(&url, &file, &miss).await,
            }
        };
        {
            let mut map = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
            if map.get(&hash).is_some_and(|l| Arc::strong_count(l) <= 2) {
                map.remove(&hash);
            }
        }
        result
    }

    async fn download(
        &self,
        url: &str,
        file: &Path,
        miss: &Path,
    ) -> Result<CachedImage, ArtworkError> {
        let fetched = {
            let _permit = self
                .fetch_permits
                .acquire()
                .await
                .map_err(|_| ArtworkError::Unavailable("cache closed".into()))?;
            self.fetch(url).await
        };
        let outcome = fetched.and_then(|bytes| {
            validate::validate(&bytes)
                .map(|v| CachedImage {
                    bytes,
                    format: v.format,
                })
                .map_err(ArtworkError::Rejected)
        });
        match outcome {
            Ok(image) => {
                self.store(file, &image.bytes).await?;
                let _ = tokio::fs::remove_file(miss).await;
                if self.writes_since_evict.fetch_add(1, Ordering::Relaxed) + 1 >= EVICT_EVERY {
                    self.writes_since_evict.store(0, Ordering::Relaxed);
                    if let Err(e) = self.evict().await {
                        tracing::warn!(error = %e, "artwork cache eviction failed");
                    }
                }
                Ok(image)
            }
            Err(e) => {
                if let Some(parent) = miss.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(miss, b"").await?;
                Err(e)
            }
        }
    }

    async fn fetch(&self, url: &str) -> Result<Vec<u8>, ArtworkError> {
        let unavailable =
            |e: reqwest::Error| ArtworkError::Unavailable(e.without_url().to_string());
        let mut resp = self.client.get(url).send().await.map_err(unavailable)?;
        if !resp.status().is_success() {
            return Err(ArtworkError::Unavailable(format!("HTTP {}", resp.status())));
        }
        if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
            return Err(ArtworkError::Rejected(Rejection::TooLarge));
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(unavailable)? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_BYTES {
                return Err(ArtworkError::Rejected(Rejection::TooLarge));
            }
        }
        Ok(body)
    }

    /// Write atomically (temp file + rename) so a reader never sees a partial
    /// body.
    async fn store(&self, file: &Path, bytes: &[u8]) -> Result<(), ArtworkError> {
        let parent = file.parent().expect("cache files live in a shard");
        tokio::fs::create_dir_all(parent).await?;
        let n = self.tmp_counter.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".tmp-{}-{n}", std::process::id()));
        tokio::fs::write(&tmp, bytes).await?;
        if let Err(e) = tokio::fs::rename(&tmp, file).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.into());
        }
        Ok(())
    }

    /// Files and bytes currently cached (markers excluded).
    pub async fn stats(&self) -> Result<CacheStats, ArtworkError> {
        let dir = self.dir.clone();
        let entries = tokio::task::spawn_blocking(move || scan(&dir))
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))??;
        Ok(entries
            .iter()
            .filter(|e| !e.marker)
            .fold(CacheStats::default(), |s, e| CacheStats {
                files: s.files + 1,
                bytes: s.bytes + e.len,
            }))
    }

    /// Remove every cached file and marker.
    pub async fn clear(&self) -> Result<(), ArtworkError> {
        match tokio::fs::remove_dir_all(&self.dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Trim the cache to 90% of its budget, least recently used first, and
    /// drop expired markers and stray temp files. Returns the files removed.
    pub async fn evict(&self) -> Result<u64, ArtworkError> {
        let dir = self.dir.clone();
        let budget = self.max_bytes;
        let removed = tokio::task::spawn_blocking(move || -> std::io::Result<u64> {
            let entries = scan(&dir)?;
            let now = SystemTime::now();
            let mut removed = 0;
            let mut files = Vec::new();
            for e in entries {
                let age = now.duration_since(e.modified).unwrap_or_default();
                if e.marker && age > MISS_TTL || e.temp && age > Duration::from_secs(3600) {
                    if std::fs::remove_file(&e.path).is_ok() {
                        removed += 1;
                    }
                } else if !e.marker && !e.temp {
                    files.push(e);
                }
            }
            let mut total: u64 = files.iter().map(|e| e.len).sum();
            if total <= budget {
                return Ok(removed);
            }
            let target = budget / 10 * 9;
            files.sort_by_key(|e| e.modified);
            for e in files {
                if total <= target {
                    break;
                }
                if std::fs::remove_file(&e.path).is_ok() {
                    total -= e.len;
                    removed += 1;
                }
            }
            Ok(removed)
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;
        Ok(removed)
    }
}

/// A cached, still-valid file. A corrupt file is deleted and treated as a miss.
async fn read_hit(file: &Path) -> Option<CachedImage> {
    let bytes = tokio::fs::read(file).await.ok()?;
    let Some(format) = validate::sniff(&bytes) else {
        let _ = tokio::fs::remove_file(file).await;
        return None;
    };
    touch_if_stale(file).await;
    Some(CachedImage { bytes, format })
}

async fn touch_if_stale(file: &Path) {
    let Ok(meta) = tokio::fs::metadata(file).await else {
        return;
    };
    let stale = meta
        .modified()
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|age| age > TOUCH_AFTER);
    if stale {
        let path = file.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || {
            std::fs::File::options()
                .write(true)
                .open(path)
                .and_then(|f| f.set_modified(SystemTime::now()))
        })
        .await;
    }
}

async fn fresh_marker(miss: &Path) -> bool {
    tokio::fs::metadata(miss)
        .await
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .is_some_and(|age| age < MISS_TTL)
}

struct Entry {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    marker: bool,
    temp: bool,
}

fn scan(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    let shards = match std::fs::read_dir(dir) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for shard in shards.flatten() {
        if !shard.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for f in std::fs::read_dir(shard.path())?.flatten() {
            let Ok(meta) = f.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let name = f.file_name().to_string_lossy().into_owned();
            out.push(Entry {
                path: f.path(),
                len: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                marker: name.ends_with(".miss"),
                temp: name.starts_with(".tmp-"),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::fixtures::png;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A tiny HTTP server: `/ok` answers a PNG, `/page` an HTML page, anything
    /// else 404. Counts requests.
    async fn server() -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    counter.fetch_add(1, Ordering::SeqCst);
                    let (status, body): (&str, Vec<u8>) = if req.starts_with("GET /ok") {
                        ("200 OK", png(64, 64))
                    } else if req.starts_with("GET /page") {
                        ("200 OK", b"<html><body>gone</body></html>".to_vec())
                    } else {
                        ("404 Not Found", Vec::new())
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&body).await;
                });
            }
        });
        (base, hits)
    }

    #[tokio::test]
    async fn downloads_once_then_serves_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let (base, hits) = server().await;
        let cache = ArtworkCache::new(dir.path().join("art"), DEFAULT_MAX_BYTES);
        let url = format!("{base}/ok/logo");
        let first = cache.get(&url).await.unwrap();
        assert_eq!(first.format, ImageFormat::Png);
        let again = cache.get(&url).await.unwrap();
        assert_eq!(again.bytes, first.bytes);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "second read hits the disk");
        assert_eq!(
            cache.stats().await.unwrap(),
            CacheStats {
                files: 1,
                bytes: first.bytes.len() as u64
            }
        );

        // A fresh cache over the same directory (next launch) still hits.
        let reopened = ArtworkCache::new(dir.path().join("art"), DEFAULT_MAX_BYTES);
        reopened.get(&url).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_download() {
        let dir = tempfile::tempdir().unwrap();
        let (base, hits) = server().await;
        let cache = Arc::new(ArtworkCache::new(
            dir.path().to_path_buf(),
            DEFAULT_MAX_BYTES,
        ));
        let url = format!("{base}/ok/poster.png");
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (cache, url) = (cache.clone(), url.clone());
                tokio::spawn(async move { cache.get(&url).await.is_ok() })
            })
            .collect();
        for t in tasks {
            assert!(t.await.unwrap());
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_are_remembered_and_never_cached_as_images() {
        let dir = tempfile::tempdir().unwrap();
        let (base, hits) = server().await;
        let cache = ArtworkCache::new(dir.path().to_path_buf(), DEFAULT_MAX_BYTES);

        let page = format!("{base}/page/logo");
        assert!(matches!(
            cache.get(&page).await,
            Err(ArtworkError::Rejected(Rejection::NotAnImage))
        ));
        let missing = format!("{base}/missing.png");
        assert!(matches!(
            cache.get(&missing).await,
            Err(ArtworkError::Unavailable(_))
        ));
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // Both are negative-cached: no new requests.
        assert!(cache.get(&page).await.is_err());
        assert!(cache.get(&missing).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(cache.stats().await.unwrap().files, 0);

        assert!(matches!(
            cache.get("not a url").await,
            Err(ArtworkError::InvalidUrl)
        ));
    }

    #[tokio::test]
    async fn corrupt_files_are_refetched() {
        let dir = tempfile::tempdir().unwrap();
        let (base, hits) = server().await;
        let cache = ArtworkCache::new(dir.path().to_path_buf(), DEFAULT_MAX_BYTES);
        let url = format!("{base}/ok/a");
        cache.get(&url).await.unwrap();
        let (_, file, _) = cache.paths(&url);
        std::fs::write(&file, b"garbage").unwrap();
        assert_eq!(cache.get(&url).await.unwrap().format, ImageFormat::Png);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn eviction_keeps_the_most_recently_used_within_budget() {
        let dir = tempfile::tempdir().unwrap();
        let (base, _) = server().await;
        let size = png(64, 64).len() as u64;
        // Room for two files (90% target of a 2.5-file budget = 2.25 files).
        let cache = ArtworkCache::new(dir.path().to_path_buf(), size * 5 / 2);
        let urls: Vec<String> = (0..4).map(|i| format!("{base}/ok/{i}")).collect();
        for (i, url) in urls.iter().enumerate() {
            cache.get(url).await.unwrap();
            let (_, file, _) = cache.paths(url);
            let age = Duration::from_secs(1000 * (4 - i as u64));
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
        }
        assert_eq!(cache.evict().await.unwrap(), 2);
        let kept: Vec<bool> = urls.iter().map(|u| cache.paths(u).1.exists()).collect();
        assert_eq!(kept, vec![false, false, true, true], "oldest evicted first");

        cache.clear().await.unwrap();
        assert_eq!(cache.stats().await.unwrap(), CacheStats::default());
    }
}
