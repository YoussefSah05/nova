//! V2 streaming evidence with authenticated frames and a mandatory final receipt.
//! Replay output is provisional until the returned receipt succeeds. Consumers
//! must stage output and discard it on error; this module never publishes catalogs.
use super::*;
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

const MAGIC_V2: &[u8; 8] = b"NOVASNP2";
pub const CHUNK_BYTES: usize = 64 * 1024;
const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const LOCK_NAME: &str = ".store.lock";

#[derive(Clone, Copy)]
pub struct StreamLimits {
    pub max_payload_bytes: u64,
    /// Logical file bytes in streams-v2, including interrupted staging/unknown files.
    pub max_stored_bytes: u64,
    /// Every regular file except the store lock consumes a slot, including orphans.
    pub max_files: u32,
}
struct StreamInner {
    root: PathBuf,
    profile: Uuid,
    key: ProfileKey,
    limits: StreamLimits,
}
#[derive(Clone)]
pub struct StreamingSnapshotStore(Arc<StreamInner>);

pub struct StreamReceipt {
    metadata: Metadata,
}
impl StreamReceipt {
    pub fn descriptor(&self) -> SnapshotDescriptor {
        SnapshotDescriptor {
            id: self.metadata.id.clone(),
            source_id: self.metadata.source.clone(),
            scope: self.metadata.scope.clone(),
            parser_revision: self.metadata.parser,
        }
    }
    pub fn byte_count(&self) -> u64 {
        self.metadata.bytes
    }
    pub fn record_count(&self) -> u64 {
        self.metadata.records
    }
    pub fn completeness(&self) -> Completeness {
        if self.metadata.complete {
            Completeness::Complete
        } else {
            Completeness::Partial
        }
    }
}

enum Command {
    Abort(oneshot::Sender<Result<()>>),
    Append(Zeroizing<Vec<u8>>, oneshot::Sender<Result<()>>),
    Seal(SourceSnapshot, oneshot::Sender<Result<()>>),
}
/// Dropping the writer closes its channel; the worker then removes only its own
/// staging file and releases its lock. No final pathname exists until seal succeeds.
pub struct StreamWriter {
    tx: mpsc::Sender<Command>,
}
impl StreamWriter {
    /// Wait for removal of this writer's unpublished staging and lock release.
    pub async fn abort(self) -> Result<()> {
        let (done, result) = oneshot::channel();
        self.tx
            .send(Command::Abort(done))
            .await
            .map_err(|_| SnapshotStoreError::Worker)?;
        result.await.map_err(|_| SnapshotStoreError::Worker)?
    }
    pub async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        for chunk in bytes.chunks(CHUNK_BYTES) {
            let (done, result) = oneshot::channel();
            self.tx
                .send(Command::Append(Zeroizing::new(chunk.to_vec()), done))
                .await
                .map_err(|_| SnapshotStoreError::Worker)?;
            result.await.map_err(|_| SnapshotStoreError::Worker)??;
        }
        Ok(())
    }
    pub async fn seal(self, receipt: SourceSnapshot) -> Result<()> {
        let (done, result) = oneshot::channel();
        self.tx
            .send(Command::Seal(receipt, done))
            .await
            .map_err(|_| SnapshotStoreError::Worker)?;
        result.await.map_err(|_| SnapshotStoreError::Worker)?
    }
}

enum ReplayEvent {
    Bytes(Zeroizing<Vec<u8>>),
    Done(Result<StreamReceipt>),
}
impl StreamingSnapshotStore {
    pub async fn open(
        parent: PathBuf,
        profile: Uuid,
        key: ProfileKey,
        limits: StreamLimits,
    ) -> Result<Self> {
        if limits.max_payload_bytes == 0
            || limits.max_payload_bytes > MAX_STREAM_BYTES
            || limits.max_stored_bytes < 40
            || limits.max_files == 0
            || limits.max_files > 10_000
        {
            return Err(SnapshotStoreError::Limit);
        }
        tokio::task::spawn_blocking(move || {
            let profile_root = parent.join(profile.to_string());
            private_directory(&profile_root)?;
            let root = profile_root.join("streams-v2");
            private_directory(&root)?;
            for path in [&parent, &profile_root] {
                File::open(path)
                    .and_then(|f| f.sync_all())
                    .map_err(|_| SnapshotStoreError::Io)?;
            }
            Ok(Self(Arc::new(StreamInner {
                root,
                profile,
                key,
                limits,
            })))
        })
        .await
        .map_err(|_| SnapshotStoreError::Worker)?
    }

    pub async fn begin(&self, descriptor: SnapshotDescriptor) -> Result<StreamWriter> {
        let inner = self.0.clone();
        let (tx, mut rx) = mpsc::channel(1);
        let (ready, started) = oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let mut writer = match WriterState::new(inner, descriptor) {
                Ok(writer) => writer,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            if ready.send(Ok(())).is_err() {
                return;
            }
            while let Some(command) = rx.blocking_recv() {
                match command {
                    Command::Abort(done) => {
                        let result = writer.abort();
                        let _ = done.send(result);
                        return;
                    }
                    Command::Append(bytes, done) => match writer.append(&bytes) {
                        Ok(()) => {
                            if done.send(Ok(())).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            drop(writer);
                            let _ = done.send(Err(error));
                            return;
                        }
                    },
                    Command::Seal(receipt, done) => {
                        let result = writer.seal(receipt);
                        let _ = done.send(result);
                        return;
                    }
                }
            }
        });
        started.await.map_err(|_| SnapshotStoreError::Worker)??;
        Ok(StreamWriter { tx })
    }

    /// Authenticated chunks may reach the sink before a later failure. Only Ok
    /// certifies endpoint evidence completeness/integrity. Sink staging is mandatory.
    /// Backpressure bounds the worker channel; caller controls sink timeouts.
    pub async fn replay_into<W: AsyncWrite + Unpin>(
        &self,
        id: SnapshotId,
        sink: &mut W,
    ) -> Result<StreamReceipt> {
        let inner = self.0.clone();
        let (tx, mut rx) = mpsc::channel(1);
        tokio::task::spawn_blocking(move || {
            let result = replay(&inner, id, &tx);
            let _ = tx.blocking_send(ReplayEvent::Done(result));
        });
        while let Some(event) = rx.recv().await {
            match event {
                ReplayEvent::Bytes(bytes) => sink
                    .write_all(&bytes)
                    .await
                    .map_err(|_| SnapshotStoreError::Io)?,
                ReplayEvent::Done(result) => {
                    let receipt = result?;
                    sink.flush().await.map_err(|_| SnapshotStoreError::Io)?;
                    return Ok(receipt);
                }
            }
        }
        Err(SnapshotStoreError::Worker)
    }
}

fn private_directory(path: &std::path::Path) -> Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(SnapshotStoreError::Io),
    }
    check_directory(path)
}
fn open_private(path: &std::path::Path, lock: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    if lock {
        options.write(true).create(true).truncate(false).mode(0o600);
    }
    let file = options.open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => SnapshotStoreError::NotFound,
        _ if e.raw_os_error() == Some(libc::ELOOP) => SnapshotStoreError::UnsafePath,
        _ => SnapshotStoreError::Io,
    })?;
    let metadata = file.metadata().map_err(|_| SnapshotStoreError::Io)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(SnapshotStoreError::UnsafePath);
    }
    Ok(file)
}
fn lease(inner: &StreamInner, exclusive: bool) -> Result<File> {
    check_directory(&inner.root)?;
    let file = open_private(&inner.root.join(LOCK_NAME), true)?;
    let result = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    result.map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => SnapshotStoreError::Busy,
        std::fs::TryLockError::Error(_) => SnapshotStoreError::Io,
    })?;
    Ok(file)
}
fn occupied(inner: &StreamInner) -> Result<u64> {
    let mut bytes = 0u64;
    let mut count = 0u32;
    for entry in fs::read_dir(&inner.root).map_err(|_| SnapshotStoreError::Io)? {
        let entry = entry.map_err(|_| SnapshotStoreError::Io)?;
        if entry.file_name() == LOCK_NAME {
            continue;
        }
        let info = fs::symlink_metadata(entry.path()).map_err(|_| SnapshotStoreError::Io)?;
        if !info.is_file() {
            return Err(SnapshotStoreError::UnsafePath);
        }
        count = count.checked_add(1).ok_or(SnapshotStoreError::Quota)?;
        bytes = bytes
            .checked_add(info.len())
            .ok_or(SnapshotStoreError::Quota)?;
        if count >= inner.limits.max_files || bytes > inner.limits.max_stored_bytes {
            return Err(SnapshotStoreError::Quota);
        }
    }
    Ok(bytes)
}
fn binding(profile: Uuid, id: &SnapshotId) -> Vec<u8> {
    let mut bytes = b"nova.snapshot.aead.v2\0".to_vec();
    bytes.extend_from_slice(profile.as_bytes());
    bytes.extend_from_slice(id.as_str().as_bytes());
    bytes
}
fn nonce(index: u64) -> aead::Nonce {
    let mut bytes = [0u8; 12];
    bytes[4..].copy_from_slice(&index.to_be_bytes());
    aead::Nonce::assume_unique_for_key(bytes)
}
fn aad(context: &[u8], index: u64, kind: u8) -> Vec<u8> {
    let mut bytes = context.to_vec();
    bytes.extend_from_slice(&index.to_be_bytes());
    bytes.push(kind);
    bytes
}

struct WriterState {
    inner: Arc<StreamInner>,
    descriptor: SnapshotDescriptor,
    key: aead::LessSafeKey,
    context: Vec<u8>,
    index: u64,
    pending: Zeroizing<Vec<u8>>,
    hash: Sha256,
    bytes: u64,
    occupied: u64,
    staged: tempfile::NamedTempFile,
    // Drop after staged so the quota lock outlives temporary-file cleanup.
    _lease: File,
}
impl WriterState {
    fn abort(self) -> Result<()> {
        self.staged.close().map_err(|_| SnapshotStoreError::Io)
    }
    fn new(inner: Arc<StreamInner>, descriptor: SnapshotDescriptor) -> Result<Self> {
        let lease = lease(&inner, true)?;
        let target = inner.root.join(format!("{}.nvs2", descriptor.id.as_str()));
        match fs::symlink_metadata(target) {
            Ok(_) => return Err(SnapshotStoreError::AlreadyExists),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(SnapshotStoreError::Io),
        }
        let occupied = occupied(&inner)?
            .checked_add(40)
            .ok_or(SnapshotStoreError::Quota)?;
        if occupied > inner.limits.max_stored_bytes {
            return Err(SnapshotStoreError::Quota);
        }
        let mut salt = [0u8; 32];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| SnapshotStoreError::Crypto)?;
        let context = binding(inner.profile, &descriptor.id);
        let key = object_key(&inner.key, &salt, &context)?;
        let mut staged = tempfile::Builder::new()
            .prefix(".staging-")
            .tempfile_in(&inner.root)
            .map_err(|_| SnapshotStoreError::Io)?;
        staged
            .write_all(MAGIC_V2)
            .and_then(|_| staged.write_all(&salt))
            .map_err(|_| SnapshotStoreError::Io)?;
        Ok(Self {
            inner,
            descriptor,
            key,
            context,
            index: 0,
            pending: Zeroizing::new(Vec::with_capacity(CHUNK_BYTES)),
            hash: Sha256::new(),
            bytes: 0,
            occupied,
            staged,
            _lease: lease,
        })
    }
    fn frame(&mut self, kind: u8, mut body: Zeroizing<Vec<u8>>) -> Result<()> {
        self.key
            .seal_in_place_append_tag(
                nonce(self.index),
                aead::Aad::from(aad(&self.context, self.index, kind)),
                &mut *body,
            )
            .map_err(|_| SnapshotStoreError::Crypto)?;
        let next = self
            .occupied
            .checked_add(body.len() as u64 + 5)
            .ok_or(SnapshotStoreError::Quota)?;
        if next > self.inner.limits.max_stored_bytes {
            return Err(SnapshotStoreError::Quota);
        }
        self.staged
            .write_all(&[kind])
            .and_then(|_| self.staged.write_all(&(body.len() as u32).to_be_bytes()))
            .and_then(|_| self.staged.write_all(&body))
            .map_err(|_| SnapshotStoreError::Io)?;
        self.occupied = next;
        self.index = self.index.checked_add(1).ok_or(SnapshotStoreError::Limit)?;
        Ok(())
    }
    fn flush_pending(&mut self) -> Result<()> {
        if !self.pending.is_empty() {
            let body = std::mem::replace(
                &mut self.pending,
                Zeroizing::new(Vec::with_capacity(CHUNK_BYTES)),
            );
            self.frame(0, body)?;
        }
        Ok(())
    }
    fn append(&mut self, mut bytes: &[u8]) -> Result<()> {
        let count = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(SnapshotStoreError::Limit)?;
        if count > self.inner.limits.max_payload_bytes {
            return Err(SnapshotStoreError::Limit);
        }
        self.bytes = count;
        self.hash.update(bytes);
        while !bytes.is_empty() {
            let take = bytes.len().min(CHUNK_BYTES - self.pending.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() == CHUNK_BYTES {
                self.flush_pending()?;
            }
        }
        Ok(())
    }
    fn seal(mut self, receipt: SourceSnapshot) -> Result<()> {
        if receipt.descriptor() != &self.descriptor
            || receipt.byte_count() != self.bytes
            || receipt.payload_fingerprint().to_hex()
                != format!("{:x}", self.hash.clone().finalize())
        {
            return Err(SnapshotStoreError::Integrity);
        }
        self.flush_pending()?;
        let metadata = Metadata {
            id: self.descriptor.id.clone(),
            source: self.descriptor.source_id.clone(),
            scope: self.descriptor.scope.clone(),
            parser: self.descriptor.parser_revision,
            complete: receipt.completeness() == Completeness::Complete,
            bytes: self.bytes,
            records: receipt.record_count(),
            digest: receipt.payload_fingerprint().to_hex(),
        };
        let json = Zeroizing::new(
            serde_json::to_vec(&metadata).map_err(|_| SnapshotStoreError::Integrity)?,
        );
        if json.len() > METADATA_LIMIT {
            return Err(SnapshotStoreError::Limit);
        }
        self.frame(1, json)?;
        self.staged
            .as_file()
            .sync_all()
            .map_err(|_| SnapshotStoreError::Io)?;
        #[cfg(test)]
        super::crash_tests::interrupt_at("stream-staged");
        let target = self
            .inner
            .root
            .join(format!("{}.nvs2", self.descriptor.id.as_str()));
        self.staged.persist_noclobber(target).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                SnapshotStoreError::AlreadyExists
            } else {
                SnapshotStoreError::Io
            }
        })?;
        File::open(&self.inner.root)
            .and_then(|f| f.sync_all())
            .map_err(|_| SnapshotStoreError::Io)?;
        #[cfg(test)]
        super::crash_tests::interrupt_at("stream-committed");
        Ok(())
    }
}

fn exact(file: &mut File, bytes: &mut [u8]) -> Result<()> {
    file.read_exact(bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            SnapshotStoreError::Integrity
        } else {
            SnapshotStoreError::Io
        }
    })
}
fn replay(
    inner: &StreamInner,
    id: SnapshotId,
    tx: &mpsc::Sender<ReplayEvent>,
) -> Result<StreamReceipt> {
    let _lease = lease(inner, false)?;
    let mut file = open_private(&inner.root.join(format!("{}.nvs2", id.as_str())), false)?;
    if file.metadata().map_err(|_| SnapshotStoreError::Io)?.len() > inner.limits.max_stored_bytes {
        return Err(SnapshotStoreError::Quota);
    }
    let mut header = [0u8; 40];
    exact(&mut file, &mut header)?;
    if &header[..8] != MAGIC_V2 {
        return Err(SnapshotStoreError::Integrity);
    }
    let context = binding(inner.profile, &id);
    let key = object_key(&inner.key, &header[8..], &context)?;
    let mut index = 0u64;
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    let mut short_chunk = false;
    loop {
        let mut header = [0u8; 5];
        exact(&mut file, &mut header)?;
        let kind = header[0];
        let length = u32::from_be_bytes(
            header[1..]
                .try_into()
                .map_err(|_| SnapshotStoreError::Integrity)?,
        ) as usize;
        let maximum = match kind {
            0 => CHUNK_BYTES,
            1 => METADATA_LIMIT,
            _ => return Err(SnapshotStoreError::Integrity),
        };
        if length <= 16 || length > maximum + 16 {
            return Err(SnapshotStoreError::Integrity);
        }
        let mut body = Zeroizing::new(vec![0u8; length]);
        exact(&mut file, &mut body)?;
        let plain = key
            .open_in_place(
                nonce(index),
                aead::Aad::from(aad(&context, index, kind)),
                &mut body,
            )
            .map_err(|_| SnapshotStoreError::Integrity)?;
        index = index.checked_add(1).ok_or(SnapshotStoreError::Limit)?;
        if kind == 0 {
            if short_chunk {
                return Err(SnapshotStoreError::Integrity);
            }
            short_chunk = plain.len() < CHUNK_BYTES;
            bytes = bytes
                .checked_add(plain.len() as u64)
                .ok_or(SnapshotStoreError::Limit)?;
            if bytes > inner.limits.max_payload_bytes {
                return Err(SnapshotStoreError::Limit);
            }
            hash.update(&*plain);
            tx.blocking_send(ReplayEvent::Bytes(Zeroizing::new(plain.to_vec())))
                .map_err(|_| SnapshotStoreError::Worker)?;
        } else {
            let metadata: Metadata =
                serde_json::from_slice(plain).map_err(|_| SnapshotStoreError::Integrity)?;
            let mut extra = [0u8; 1];
            if file.read(&mut extra).map_err(|_| SnapshotStoreError::Io)? != 0
                || metadata.id != id
                || metadata.bytes != bytes
                || metadata.digest != format!("{:x}", hash.finalize())
            {
                return Err(SnapshotStoreError::Integrity);
            }
            return Ok(StreamReceipt { metadata });
        }
    }
}
