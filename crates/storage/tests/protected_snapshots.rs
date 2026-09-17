#![cfg(unix)]

use nova_core::catalog::{snapshot::*, EndpointScope, SnapshotId, SourceId};
use nova_storage::snapshots::{ProfileKey, SnapshotStore, SnapshotStoreError};
use std::num::{NonZeroU32, NonZeroU64};
use uuid::Uuid;

fn receipt(id: &str, bytes: &[u8], completeness: Completeness) -> SourceSnapshot {
    let mut capture = SnapshotCapture::new(
        SnapshotDescriptor {
            id: SnapshotId::new(id).unwrap(),
            source_id: SourceId::new("source1").unwrap(),
            scope: EndpointScope::new("playlist").unwrap(),
            parser_revision: NonZeroU32::new(1).unwrap(),
        },
        SnapshotLimits {
            max_bytes: NonZeroU64::new(1024 * 1024).unwrap(),
            max_records: NonZeroU64::new(100).unwrap(),
        },
    );
    capture.push_bytes(bytes).unwrap();
    capture.seal(completeness).unwrap()
}

async fn store(root: &std::path::Path, profile: Uuid, key: u8, limit: usize) -> SnapshotStore {
    SnapshotStore::open(root.to_owned(), profile, ProfileKey::new([key; 32]), limit)
        .await
        .unwrap()
}

#[tokio::test]
async fn encrypted_roundtrip_survives_reopen_and_keeps_partial_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let bytes = include_bytes!("../../../fixtures/iptv/v1/playlist.m3u").to_vec();
    let saved = receipt("snapshot1", &bytes, Completeness::Partial);
    let expected_hash = saved.payload_fingerprint();
    let db = store(dir.path(), profile, 7, 8192).await;
    db.persist(saved, bytes.clone()).await.unwrap();
    drop(db);
    let path = dir.path().join(profile.to_string()).join("snapshot1.nvs");
    let disk = std::fs::read(&path).unwrap();
    assert!(!disk.windows(7).any(|w| w == b"fixture"));
    let db = store(dir.path(), profile, 7, 8192).await;
    let replay = db
        .replay(SnapshotId::new("snapshot1").unwrap())
        .await
        .unwrap();
    assert_eq!(replay.payload(), bytes);
    assert_eq!(replay.descriptor().source_id.as_str(), "source1");
    assert_eq!(replay.completeness(), Completeness::Partial);
    assert_eq!(replay.record_count(), 0);
    assert_eq!(replay.payload_fingerprint_hex(), expected_hash.to_hex());
}

#[tokio::test]
async fn mismatch_and_quota_fail_without_publication() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, 7, 8).await;
    assert!(matches!(
        db.persist(receipt("a", b"a", Completeness::Complete), b"b".to_vec())
            .await,
        Err(SnapshotStoreError::Integrity)
    ));
    assert!(matches!(
        db.persist(
            receipt("b", b"123456789", Completeness::Complete),
            b"123456789".to_vec()
        )
        .await,
        Err(SnapshotStoreError::Limit)
    ));
    assert_eq!(
        std::fs::read_dir(dir.path().join(profile.to_string()))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn immutable_commit_rejects_duplicate_and_preserves_original() {
    let dir = tempfile::tempdir().unwrap();
    let db = store(dir.path(), Uuid::new_v4(), 7, 8192).await;
    db.persist(
        receipt("a", b"first", Completeness::Complete),
        b"first".to_vec(),
    )
    .await
    .unwrap();
    assert!(matches!(
        db.persist(
            receipt("a", b"second", Completeness::Complete),
            b"second".to_vec()
        )
        .await,
        Err(SnapshotStoreError::AlreadyExists)
    ));
    assert_eq!(
        db.replay(SnapshotId::new("a").unwrap())
            .await
            .unwrap()
            .payload(),
        b"first"
    );
}

#[tokio::test]
async fn wrong_key_profile_and_snapshot_substitution_fail_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, 7, 8192).await;
    db.persist(
        receipt("a", b"secret", Completeness::Complete),
        b"secret".to_vec(),
    )
    .await
    .unwrap();
    let wrong = store(dir.path(), profile, 8, 8192).await;
    assert!(matches!(
        wrong.replay(SnapshotId::new("a").unwrap()).await,
        Err(SnapshotStoreError::Integrity)
    ));
    let other_profile = Uuid::new_v4();
    let other = store(dir.path(), other_profile, 7, 8192).await;
    let source = dir.path().join(profile.to_string()).join("a.nvs");
    std::fs::copy(
        &source,
        dir.path().join(other_profile.to_string()).join("a.nvs"),
    )
    .unwrap();
    assert!(matches!(
        other.replay(SnapshotId::new("a").unwrap()).await,
        Err(SnapshotStoreError::Integrity)
    ));
    std::fs::copy(&source, dir.path().join(profile.to_string()).join("b.nvs")).unwrap();
    assert!(matches!(
        db.replay(SnapshotId::new("b").unwrap()).await,
        Err(SnapshotStoreError::Integrity)
    ));
}

#[tokio::test]
async fn corrupt_truncated_and_oversized_files_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, 7, 8192).await;
    db.persist(
        receipt("a", b"secret", Completeness::Complete),
        b"secret".to_vec(),
    )
    .await
    .unwrap();
    let path = dir.path().join(profile.to_string()).join("a.nvs");
    let original = std::fs::read(&path).unwrap();
    for index in [0, 8, original.len() - 1] {
        let mut bytes = original.clone();
        bytes[index] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            db.replay(SnapshotId::new("a").unwrap()).await,
            Err(SnapshotStoreError::Integrity)
        ));
    }
    std::fs::write(&path, &original[..original.len() - 1]).unwrap();
    assert!(matches!(
        db.replay(SnapshotId::new("a").unwrap()).await,
        Err(SnapshotStoreError::Integrity)
    ));
    std::fs::write(&path, vec![0; 32_768]).unwrap();
    assert!(matches!(
        db.replay(SnapshotId::new("a").unwrap()).await,
        Err(SnapshotStoreError::Limit)
    ));
}

#[tokio::test]
async fn orphaned_staging_file_is_not_replayed_or_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, 7, 8192).await;
    let orphan = dir.path().join(profile.to_string()).join(".staging-crash");
    std::fs::write(&orphan, b"incomplete ciphertext").unwrap();
    drop(db);
    let db = store(dir.path(), profile, 7, 8192).await;
    assert!(matches!(
        db.replay(SnapshotId::new("a").unwrap()).await,
        Err(SnapshotStoreError::NotFound)
    ));
    assert!(orphan.exists());
    db.persist(receipt("a", b"", Completeness::Complete), vec![])
        .await
        .unwrap();
    assert!(db
        .replay(SnapshotId::new("a").unwrap())
        .await
        .unwrap()
        .payload()
        .is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn permissions_are_private_and_symlinks_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile, 7, 8192).await;
    db.persist(
        receipt("a", b"secret", Completeness::Complete),
        b"secret".to_vec(),
    )
    .await
    .unwrap();
    let root = dir.path().join(profile.to_string());
    assert_eq!(root.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(
        root.join("a.nvs").metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    symlink(root.join("a.nvs"), root.join("b.nvs")).unwrap();
    assert!(matches!(
        db.replay(SnapshotId::new("b").unwrap()).await,
        Err(SnapshotStoreError::UnsafePath)
    ));
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        SnapshotStore::open(
            dir.path().to_owned(),
            profile,
            ProfileKey::new([7; 32]),
            8192
        )
        .await,
        Err(SnapshotStoreError::UnsafePath)
    ));
}

#[tokio::test]
async fn concurrent_handles_publish_exactly_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let first = store(dir.path(), profile, 7, 8192).await;
    let second = store(dir.path(), profile, 7, 8192).await;
    let (a, b) = tokio::join!(
        first.persist(
            receipt("a", b"first", Completeness::Complete),
            b"first".to_vec()
        ),
        second.persist(
            receipt("a", b"second", Completeness::Complete),
            b"second".to_vec()
        ),
    );
    assert!(matches!(
        (&a, &b),
        (Ok(()), Err(SnapshotStoreError::AlreadyExists))
            | (Err(SnapshotStoreError::AlreadyExists), Ok(()))
    ));
    let result = first.replay(SnapshotId::new("a").unwrap()).await.unwrap();
    assert_eq!(
        result.payload(),
        if a.is_ok() {
            b"first".as_slice()
        } else {
            b"second".as_slice()
        }
    );
}
