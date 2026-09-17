//! Bounded encrypted evidence store, deliberately not wired to production imports.
//! The caller supplies a profile-scoped key from its vault and a trusted parent
//! directory. No plaintext or key is persisted. Unix file permissions are required.

use nova_core::catalog::{
    snapshot::{Completeness, SnapshotDescriptor, SourceSnapshot},
    EndpointScope, SnapshotId, SourceId,
};
use ring::{
    aead, hkdf,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    num::NonZeroU32,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Semaphore;
use uuid::Uuid;
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"NOVASNP1";
pub mod stream;
const SALT_LEN: usize = 32;
const METADATA_LIMIT: usize = 4096;
/// Prototype ceiling: larger provider snapshots need a chunked format first.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotStoreError {
    #[error("snapshot store is busy")]
    Busy,
    #[error("snapshot storage quota exceeded")]
    Quota,
    #[error("snapshot size limit exceeded")]
    Limit,
    #[error("snapshot integrity check failed")]
    Integrity,
    #[error("snapshot already exists")]
    AlreadyExists,
    #[error("snapshot not found")]
    NotFound,
    #[error("snapshot directory or file is not private and regular")]
    UnsafePath,
    #[error("snapshot storage I/O failed")]
    Io,
    #[error("snapshot cryptography unavailable")]
    Crypto,
    #[error("snapshot worker failed")]
    Worker,
}
type Result<T> = std::result::Result<T, SnapshotStoreError>;

/// No Debug/Clone/serialization; owned input bytes are cleared on drop. The vault
/// remains responsible for its own input copies and for key backup/rotation.
pub struct ProfileKey(Zeroizing<[u8; 32]>);
impl ProfileKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    id: SnapshotId,
    source: SourceId,
    scope: EndpointScope,
    parser: NonZeroU32,
    complete: bool,
    bytes: u64,
    records: u64,
    digest: String,
}

/// Authenticated stored evidence, not a validated or published catalog snapshot.
/// No Debug or serialization: raw payload can contain credentials.
pub struct SnapshotReplay {
    descriptor: SnapshotDescriptor,
    complete: Completeness,
    records: u64,
    digest: String,
    payload: Zeroizing<Vec<u8>>,
}
impl SnapshotReplay {
    pub fn descriptor(&self) -> &SnapshotDescriptor {
        &self.descriptor
    }
    pub fn completeness(&self) -> Completeness {
        self.complete
    }
    pub fn record_count(&self) -> u64 {
        self.records
    }
    pub fn payload_fingerprint_hex(&self) -> &str {
        &self.digest
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

struct Inner {
    root: PathBuf,
    profile: Uuid,
    key: ProfileKey,
    limit: usize,
    gate: Arc<Semaphore>,
}
/// Filesystem and cryptographic work run on a blocking worker, one operation per
/// handle at a time. Reopened handles can race safely through no-clobber commits.
#[derive(Clone)]
pub struct SnapshotStore(Arc<Inner>);

impl SnapshotStore {
    pub async fn open(
        parent: PathBuf,
        profile: Uuid,
        key: ProfileKey,
        limit: usize,
    ) -> Result<Self> {
        if limit == 0 || limit > MAX_PAYLOAD_BYTES {
            return Err(SnapshotStoreError::Limit);
        }
        tokio::task::spawn_blocking(move || {
            let root = parent.join(profile.to_string());
            match fs::DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(SnapshotStoreError::Io),
            }
            check_directory(&root)?;
            File::open(&parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| SnapshotStoreError::Io)?;
            Ok(Self(Arc::new(Inner {
                root,
                profile,
                key,
                limit,
                gate: Arc::new(Semaphore::new(1)),
            })))
        })
        .await
        .map_err(|_| SnapshotStoreError::Worker)?
    }

    pub async fn persist(&self, receipt: SourceSnapshot, payload: Vec<u8>) -> Result<()> {
        let payload = Zeroizing::new(payload);
        if payload.len() > self.0.limit {
            return Err(SnapshotStoreError::Limit);
        }
        let permit = self
            .0
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SnapshotStoreError::Worker)?;
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            check_directory(&inner.root)?;
            if receipt.byte_count() != payload.len() as u64
                || receipt.payload_fingerprint().to_hex() != digest(&payload)
            {
                return Err(SnapshotStoreError::Integrity);
            }
            let descriptor = receipt.descriptor();
            let metadata = Metadata {
                id: descriptor.id.clone(),
                source: descriptor.source_id.clone(),
                scope: descriptor.scope.clone(),
                parser: descriptor.parser_revision,
                complete: receipt.completeness() == Completeness::Complete,
                bytes: receipt.byte_count(),
                records: receipt.record_count(),
                digest: receipt.payload_fingerprint().to_hex(),
            };
            let json = Zeroizing::new(
                serde_json::to_vec(&metadata).map_err(|_| SnapshotStoreError::Integrity)?,
            );
            if json.len() > METADATA_LIMIT {
                return Err(SnapshotStoreError::Limit);
            }
            let mut body = Zeroizing::new(Vec::with_capacity(4 + json.len() + payload.len() + 16));
            body.extend_from_slice(&(json.len() as u32).to_be_bytes());
            body.extend_from_slice(&json);
            body.extend_from_slice(&payload);
            let mut salt = [0u8; SALT_LEN];
            SystemRandom::new()
                .fill(&mut salt)
                .map_err(|_| SnapshotStoreError::Crypto)?;
            let context = context(inner.profile, &metadata.id);
            let key = object_key(&inner.key, &salt, &context)?;
            // Every object gets a fresh 256-bit random HKDF salt/subkey. The nonce
            // is used once for sealing with that subkey, including failed commits.
            key.seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key([0; 12]),
                aead::Aad::from(&context),
                &mut *body,
            )
            .map_err(|_| SnapshotStoreError::Crypto)?;
            let mut staged = tempfile::Builder::new()
                .prefix(".staging-")
                .tempfile_in(&inner.root)
                .map_err(|_| SnapshotStoreError::Io)?;
            staged
                .write_all(MAGIC)
                .and_then(|_| staged.write_all(&salt))
                .and_then(|_| staged.write_all(&body))
                .and_then(|_| staged.as_file().sync_all())
                .map_err(|_| SnapshotStoreError::Io)?;
            #[cfg(test)]
            crash_tests::interrupt_at("staged");
            let target = inner.root.join(format!("{}.nvs", metadata.id.as_str()));
            staged
                .persist_noclobber(target)
                .map_err(|e| match e.error.kind() {
                    std::io::ErrorKind::AlreadyExists => SnapshotStoreError::AlreadyExists,
                    _ => SnapshotStoreError::Io,
                })?;
            // If this fails, commit durability is uncertain: replay the ID before
            // retrying. Never overwrite a possibly committed snapshot.
            File::open(&inner.root)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| SnapshotStoreError::Io)?;
            #[cfg(test)]
            crash_tests::interrupt_at("committed");
            Ok(())
        })
        .await
        .map_err(|_| SnapshotStoreError::Worker)?
    }

    pub async fn replay(&self, id: SnapshotId) -> Result<SnapshotReplay> {
        let permit = self
            .0
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SnapshotStoreError::Worker)?;
        let inner = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            check_directory(&inner.root)?;
            let path = inner.root.join(format!("{}.nvs", id.as_str()));
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => SnapshotStoreError::NotFound,
                    _ if e.raw_os_error() == Some(libc::ELOOP) => SnapshotStoreError::UnsafePath,
                    _ => SnapshotStoreError::Io,
                })?;
            let info = file.metadata().map_err(|_| SnapshotStoreError::Io)?;
            if !info.is_file() || info.permissions().mode() & 0o077 != 0 {
                return Err(SnapshotStoreError::UnsafePath);
            }
            let maximum = inner.limit + METADATA_LIMIT + 4 + 8 + SALT_LEN + 16;
            if info.len() > maximum as u64 {
                return Err(SnapshotStoreError::Limit);
            }
            let mut bytes = Zeroizing::new(Vec::new());
            file.take(maximum as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| SnapshotStoreError::Io)?;
            if bytes.len() > maximum {
                return Err(SnapshotStoreError::Limit);
            }
            if bytes.len() < 8 + SALT_LEN + 16 || &bytes[..8] != MAGIC {
                return Err(SnapshotStoreError::Integrity);
            }
            let context = context(inner.profile, &id);
            let key = object_key(&inner.key, &bytes[8..8 + SALT_LEN], &context)?;
            let plain = key
                .open_in_place(
                    aead::Nonce::assume_unique_for_key([0; 12]),
                    aead::Aad::from(&context),
                    &mut bytes[8 + SALT_LEN..],
                )
                .map_err(|_| SnapshotStoreError::Integrity)?;
            let length = plain.get(..4).ok_or(SnapshotStoreError::Integrity)?;
            let length = u32::from_be_bytes(
                length
                    .try_into()
                    .map_err(|_| SnapshotStoreError::Integrity)?,
            ) as usize;
            if length > METADATA_LIMIT {
                return Err(SnapshotStoreError::Integrity);
            }
            let json = plain
                .get(4..4 + length)
                .ok_or(SnapshotStoreError::Integrity)?;
            let metadata: Metadata =
                serde_json::from_slice(json).map_err(|_| SnapshotStoreError::Integrity)?;
            let payload = &plain[4 + length..];
            if payload.len() > inner.limit {
                return Err(SnapshotStoreError::Limit);
            }
            if metadata.id != id
                || metadata.bytes != payload.len() as u64
                || metadata.digest != digest(payload)
            {
                return Err(SnapshotStoreError::Integrity);
            }
            Ok(SnapshotReplay {
                descriptor: SnapshotDescriptor {
                    id,
                    source_id: metadata.source,
                    scope: metadata.scope,
                    parser_revision: metadata.parser,
                },
                complete: if metadata.complete {
                    Completeness::Complete
                } else {
                    Completeness::Partial
                },
                records: metadata.records,
                digest: metadata.digest,
                payload: Zeroizing::new(payload.to_vec()),
            })
        })
        .await
        .map_err(|_| SnapshotStoreError::Worker)?
    }
}

fn check_directory(path: &std::path::Path) -> Result<()> {
    let info = fs::symlink_metadata(path).map_err(|_| SnapshotStoreError::Io)?;
    if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
        return Err(SnapshotStoreError::UnsafePath);
    }
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn context(profile: Uuid, id: &SnapshotId) -> Vec<u8> {
    let mut context = b"nova.snapshot.aead.v1\0".to_vec();
    context.extend_from_slice(profile.as_bytes());
    context.extend_from_slice(id.as_str().as_bytes());
    context
}
fn object_key(key: &ProfileKey, salt: &[u8], context: &[u8]) -> Result<aead::LessSafeKey> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, salt);
    let prk = salt.extract(key.0.as_ref());
    let info = [context];
    let okm = prk
        .expand(&info, &aead::AES_256_GCM)
        .map_err(|_| SnapshotStoreError::Crypto)?;
    let mut bytes = Zeroizing::new([0u8; 32]);
    okm.fill(bytes.as_mut())
        .map_err(|_| SnapshotStoreError::Crypto)?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, bytes.as_ref())
        .map_err(|_| SnapshotStoreError::Crypto)?;
    Ok(aead::LessSafeKey::new(key))
}

#[cfg(test)]
mod crash_tests {
    use super::*;
    use nova_core::catalog::snapshot::{SnapshotCapture, SnapshotLimits};
    use std::{
        num::NonZeroU64,
        process::Command,
        time::{Duration, Instant},
    };

    pub(super) fn interrupt_at(phase: &str) {
        if std::env::var("NOVA_SNAPSHOT_TEST_INTERRUPT").as_deref() == Ok(phase) {
            // Abrupt process termination: no Rust destructors or temp-file cleanup.
            // Compiled only into this unit-test executable, never the app/library.
            std::process::exit(86);
        }
    }

    #[test]
    #[ignore = "subprocess helper invoked by crash boundary tests"]
    fn subprocess_writer() {
        let parent = PathBuf::from(std::env::var_os("NOVA_SNAPSHOT_TEST_ROOT").unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let mut capture = SnapshotCapture::new(
                SnapshotDescriptor {
                    id: SnapshotId::new("crash").unwrap(),
                    source_id: SourceId::new("source").unwrap(),
                    scope: EndpointScope::new("vod").unwrap(),
                    parser_revision: NonZeroU32::new(1).unwrap(),
                },
                SnapshotLimits {
                    max_bytes: NonZeroU64::new(1024).unwrap(),
                    max_records: NonZeroU64::new(10).unwrap(),
                },
            );
            capture.push_bytes(b"synthetic secret").unwrap();
            let receipt = capture.seal(Completeness::Complete).unwrap();
            if std::env::var("NOVA_SNAPSHOT_TEST_INTERRUPT")
                .unwrap()
                .starts_with("stream-")
            {
                let store = stream::StreamingSnapshotStore::open(
                    parent,
                    Uuid::nil(),
                    ProfileKey::new([7; 32]),
                    stream_limits(),
                )
                .await
                .unwrap();
                let mut writer = store.begin(receipt.descriptor().clone()).await.unwrap();
                writer.append(b"synthetic secret").await.unwrap();
                writer.seal(receipt).await.unwrap();
                return;
            }
            let store = SnapshotStore::open(parent, Uuid::nil(), ProfileKey::new([7; 32]), 1024)
                .await
                .unwrap();
            store
                .persist(receipt, b"synthetic secret".to_vec())
                .await
                .unwrap();
        });
        panic!("child failed to reach interruption hook");
    }

    fn assert_boundary(phase: &str, published: bool) {
        let dir = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "snapshots::crash_tests::subprocess_writer",
                "--ignored",
            ])
            .env("NOVA_SNAPSHOT_TEST_INTERRUPT", phase)
            .env("NOVA_SNAPSHOT_TEST_ROOT", dir.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("snapshot subprocess exceeded deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(86));
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            if phase.starts_with("stream-") {
                let store = stream::StreamingSnapshotStore::open(
                    dir.path().to_owned(),
                    Uuid::nil(),
                    ProfileKey::new([7; 32]),
                    stream_limits(),
                )
                .await
                .unwrap();
                let mut output = Vec::new();
                let result = store
                    .replay_into(SnapshotId::new("crash").unwrap(), &mut output)
                    .await;
                if published {
                    assert!(result.is_ok());
                    assert_eq!(output, b"synthetic secret");
                } else {
                    assert!(matches!(result, Err(SnapshotStoreError::NotFound)));
                }
                // A committed file or abandoned staging consumes the only slot;
                // Quota (not Busy) also proves the crashed process released its lock.
                let descriptor = SnapshotDescriptor {
                    id: SnapshotId::new("next").unwrap(),
                    source_id: SourceId::new("source").unwrap(),
                    scope: EndpointScope::new("vod").unwrap(),
                    parser_revision: NonZeroU32::new(1).unwrap(),
                };
                assert!(matches!(
                    store.begin(descriptor).await,
                    Err(SnapshotStoreError::Quota)
                ));
                return;
            }
            let store = SnapshotStore::open(
                dir.path().to_owned(),
                Uuid::nil(),
                ProfileKey::new([7; 32]),
                1024,
            )
            .await
            .unwrap();
            let result = store.replay(SnapshotId::new("crash").unwrap()).await;
            if published {
                assert_eq!(result.unwrap().payload(), b"synthetic secret");
            } else {
                assert!(matches!(result, Err(SnapshotStoreError::NotFound)));
                let files: Vec<_> = fs::read_dir(dir.path().join(Uuid::nil().to_string()))
                    .unwrap()
                    .collect();
                assert_eq!(files.len(), 1, "only encrypted staging evidence remains");
            }
        });
    }

    #[test]
    fn interruption_before_commit_keeps_snapshot_invisible() {
        assert_boundary("staged", false);
    }
    #[test]
    fn interruption_after_commit_replays_complete_evidence() {
        assert_boundary("committed", true);
    }
    fn stream_limits() -> stream::StreamLimits {
        stream::StreamLimits {
            max_payload_bytes: 1024,
            max_stored_bytes: 8192,
            max_files: 1,
        }
    }
    #[test]
    fn stream_interruption_before_commit_charges_orphan_and_releases_lock() {
        assert_boundary("stream-staged", false);
    }
    #[test]
    fn stream_interruption_after_commit_replays_evidence_and_releases_lock() {
        assert_boundary("stream-committed", true);
    }
}
