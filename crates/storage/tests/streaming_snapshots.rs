#![cfg(unix)]
use nova_core::catalog::{snapshot::*, EndpointScope, SnapshotId, SourceId};
use nova_storage::snapshots::{stream::*, ProfileKey, SnapshotStoreError};
use std::num::{NonZeroU32, NonZeroU64};
use uuid::Uuid;

fn descriptor(id: &str) -> SnapshotDescriptor {
    SnapshotDescriptor {
        id: SnapshotId::new(id).unwrap(),
        source_id: SourceId::new("source").unwrap(),
        scope: EndpointScope::new("playlist").unwrap(),
        parser_revision: NonZeroU32::new(1).unwrap(),
    }
}
fn capture(id: &str) -> SnapshotCapture {
    SnapshotCapture::new(
        descriptor(id),
        SnapshotLimits {
            max_bytes: NonZeroU64::new(32 * 1024 * 1024).unwrap(),
            max_records: NonZeroU64::new(10).unwrap(),
        },
    )
}
fn limits() -> StreamLimits {
    StreamLimits {
        max_payload_bytes: 32 * 1024 * 1024,
        max_stored_bytes: 40 * 1024 * 1024,
        max_files: 10,
    }
}
async fn store(
    dir: &std::path::Path,
    profile: Uuid,
    limits: StreamLimits,
) -> StreamingSnapshotStore {
    StreamingSnapshotStore::open(dir.to_owned(), profile, ProfileKey::new([7; 32]), limits)
        .await
        .unwrap()
}
async fn save(store: &StreamingSnapshotStore, id: &str, bytes: &[u8]) {
    let mut writer = store.begin(descriptor(id)).await.unwrap();
    let mut capture = capture(id);
    capture.push_bytes(bytes).unwrap();
    writer.append(bytes).await.unwrap();
    writer
        .seal(capture.seal(Completeness::Complete).unwrap())
        .await
        .unwrap();
}
fn file(dir: &std::path::Path, profile: Uuid, id: &str) -> std::path::PathBuf {
    dir.join(profile.to_string())
        .join("streams-v2")
        .join(format!("{id}.nvs2"))
}

#[tokio::test]
async fn fixture_stream_replays_after_reopen_with_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, limits()).await;
    let bytes = include_bytes!("../../../fixtures/iptv/v1/playlist.m3u");
    save(&db, "a", bytes).await;
    drop(db);
    let db = store(dir.path(), profile, limits()).await;
    let mut output = Vec::new();
    let receipt = db
        .replay_into(SnapshotId::new("a").unwrap(), &mut output)
        .await
        .unwrap();
    assert_eq!(output, bytes);
    assert_eq!(receipt.byte_count(), bytes.len() as u64);
    assert_eq!(receipt.completeness(), Completeness::Complete);
    assert_eq!(receipt.descriptor().source_id.as_str(), "source");
}

#[tokio::test]
async fn streams_beyond_v1_ceiling_without_whole_payload_buffers() {
    use sha2::{Digest, Sha256};
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    struct Sink {
        hash: Sha256,
        bytes: u64,
    }
    impl tokio::io::AsyncWrite for Sink {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            assert!(bytes.len() <= CHUNK_BYTES);
            self.hash.update(bytes);
            self.bytes += bytes.len() as u64;
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let db = store(dir.path(), Uuid::new_v4(), limits()).await;
    let chunk = vec![0x5a; CHUNK_BYTES];
    let mut cap = capture("a");
    let mut writer = db.begin(descriptor("a")).await.unwrap();
    for _ in 0..273 {
        cap.push_bytes(&chunk).unwrap();
        writer.append(&chunk).await.unwrap();
    }
    let receipt = cap.seal(Completeness::Complete).unwrap();
    let expected = receipt.payload_fingerprint().to_hex();
    writer.seal(receipt).await.unwrap();
    let mut sink = Sink {
        hash: Sha256::new(),
        bytes: 0,
    };
    db.replay_into(SnapshotId::new("a").unwrap(), &mut sink)
        .await
        .unwrap();
    assert_eq!(sink.bytes, (273 * CHUNK_BYTES) as u64);
    assert_eq!(format!("{:x}", sink.hash.finalize()), expected);
}

#[tokio::test]
async fn cross_handle_lock_prevents_quota_race_and_cancellation_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let a = store(dir.path(), profile, limits()).await;
    let b = store(dir.path(), profile, limits()).await;
    let mut writer = a.begin(descriptor("a")).await.unwrap();
    writer.append(b"abandoned").await.unwrap();
    assert!(matches!(
        b.begin(descriptor("b")).await,
        Err(SnapshotStoreError::Busy)
    ));
    drop(writer);
    let writer = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match b.begin(descriptor("b")).await {
                Ok(writer) => break writer,
                Err(SnapshotStoreError::Busy) => tokio::task::yield_now().await,
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .unwrap();
    let mut cap = capture("b");
    writer
        .seal(cap.seal(Completeness::Partial).unwrap())
        .await
        .unwrap();
    assert!(!file(dir.path(), profile, "a").exists());
    let mut out = Vec::new();
    assert_eq!(
        b.replay_into(SnapshotId::new("b").unwrap(), &mut out)
            .await
            .unwrap()
            .completeness(),
        Completeness::Partial
    );
    assert!(out.is_empty());
}

#[tokio::test]
async fn quota_counts_orphans_and_preserves_existing_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let mut budget = limits();
    budget.max_stored_bytes = 5000;
    let db = store(dir.path(), profile, budget).await;
    save(&db, "a", b"first").await;
    std::fs::write(
        file(dir.path(), profile, "a")
            .parent()
            .unwrap()
            .join(".staging-orphan"),
        vec![0; 4500],
    )
    .unwrap();
    let mut writer = db.begin(descriptor("b")).await.unwrap();
    assert!(matches!(
        writer.append(&vec![1; CHUNK_BYTES]).await,
        Err(SnapshotStoreError::Quota)
    ));
    assert!(!file(dir.path(), profile, "b").exists());
    let mut out = Vec::new();
    db.replay_into(SnapshotId::new("a").unwrap(), &mut out)
        .await
        .unwrap();
    assert_eq!(out, b"first");
}

#[tokio::test]
async fn file_count_and_payload_limits_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let mut budget = limits();
    budget.max_files = 1;
    budget.max_payload_bytes = 4;
    let db = store(dir.path(), profile, budget).await;
    let mut writer = db.begin(descriptor("bad")).await.unwrap();
    assert!(matches!(
        writer.append(b"12345").await,
        Err(SnapshotStoreError::Limit)
    ));
    save(&db, "a", b"1234").await;
    assert!(matches!(
        db.begin(descriptor("b")).await,
        Err(SnapshotStoreError::Quota)
    ));
}

#[tokio::test]
async fn corrupt_reordered_or_truncated_frames_never_return_a_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, limits()).await;
    save(&db, "a", &vec![9; CHUNK_BYTES * 2]).await;
    let path = file(dir.path(), profile, "a");
    let original = std::fs::read(&path).unwrap();
    let mut corrupt = original.clone();
    corrupt[60] ^= 1;
    let mut reordered = original.clone();
    let size = CHUNK_BYTES + 21;
    reordered[40..40 + size].copy_from_slice(&original[40 + size..40 + 2 * size]);
    let mut appended = original.clone();
    appended.push(0);
    for bytes in [
        corrupt,
        reordered,
        original[..40 + 2 * size].to_vec(),
        original[..original.len() - 1].to_vec(),
        appended,
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            db.replay_into(SnapshotId::new("a").unwrap(), &mut tokio::io::sink())
                .await,
            Err(SnapshotStoreError::Integrity)
        ));
    }
}

#[tokio::test]
async fn mismatched_receipt_never_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, limits()).await;
    let mut writer = db.begin(descriptor("a")).await.unwrap();
    writer.append(b"real").await.unwrap();
    let mut cap = capture("a");
    cap.push_bytes(b"fake").unwrap();
    assert!(matches!(
        writer.seal(cap.seal(Completeness::Complete).unwrap()).await,
        Err(SnapshotStoreError::Integrity)
    ));
    assert!(!file(dir.path(), profile, "a").exists());
}

#[tokio::test]
async fn stream_binding_rejects_other_profiles_ids_and_keys() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, limits()).await;
    save(&db, "a", b"synthetic evidence").await;
    let wrong = StreamingSnapshotStore::open(
        dir.path().to_owned(),
        profile,
        ProfileKey::new([8; 32]),
        limits(),
    )
    .await
    .unwrap();
    assert!(matches!(
        wrong
            .replay_into(SnapshotId::new("a").unwrap(), &mut tokio::io::sink())
            .await,
        Err(SnapshotStoreError::Integrity)
    ));
    let other_profile = Uuid::new_v4();
    let other = store(dir.path(), other_profile, limits()).await;
    std::fs::copy(
        file(dir.path(), profile, "a"),
        file(dir.path(), other_profile, "a"),
    )
    .unwrap();
    assert!(matches!(
        other
            .replay_into(SnapshotId::new("a").unwrap(), &mut tokio::io::sink())
            .await,
        Err(SnapshotStoreError::Integrity)
    ));
    std::fs::copy(
        file(dir.path(), profile, "a"),
        file(dir.path(), profile, "b"),
    )
    .unwrap();
    assert!(matches!(
        db.replay_into(SnapshotId::new("b").unwrap(), &mut tokio::io::sink())
            .await,
        Err(SnapshotStoreError::Integrity)
    ));
}

#[tokio::test]
async fn stream_paths_reject_symlinks_and_do_not_replace_committed_files() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, limits()).await;
    save(&db, "a", b"original").await;
    assert!(matches!(
        db.begin(descriptor("a")).await,
        Err(SnapshotStoreError::AlreadyExists)
    ));
    symlink(
        file(dir.path(), profile, "a"),
        file(dir.path(), profile, "b"),
    )
    .unwrap();
    assert!(matches!(
        db.replay_into(SnapshotId::new("b").unwrap(), &mut tokio::io::sink())
            .await,
        Err(SnapshotStoreError::UnsafePath)
    ));
    assert!(matches!(
        db.begin(descriptor("c")).await,
        Err(SnapshotStoreError::UnsafePath)
    ));
}
