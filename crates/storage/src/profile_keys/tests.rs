use super::*;
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
struct MemoryBackend(Mutex<HashMap<Uuid, Vec<u8>>>);
impl KeyBackend for MemoryBackend {
    fn load(&self, profile: Uuid) -> Result<Zeroizing<Vec<u8>>> {
        self.0
            .lock()
            .unwrap()
            .get(&profile)
            .cloned()
            .map(Zeroizing::new)
            .ok_or(ProfileKeyError::Missing)
    }
    fn insert(&self, profile: Uuid, bytes: &[u8]) -> Result<()> {
        use std::collections::hash_map::Entry;
        match self.0.lock().unwrap().entry(profile) {
            Entry::Vacant(entry) => {
                entry.insert(bytes.to_vec());
                Ok(())
            }
            Entry::Occupied(_) => Err(ProfileKeyError::AlreadyExists),
        }
    }
}
fn vault(backend: Arc<MemoryBackend>) -> ProfileKeyVault {
    ProfileKeyVault { backend }
}

#[tokio::test]
async fn missing_load_does_not_create_a_replacement_key() {
    let backend = Arc::new(MemoryBackend::default());
    assert!(matches!(
        vault(backend.clone()).load(Uuid::new_v4()).await,
        Err(ProfileKeyError::Missing)
    ));
    assert!(backend.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn provisioning_is_random_scoped_and_never_overwrites() {
    let backend = Arc::new(MemoryBackend::default());
    let keys = vault(backend.clone());
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    keys.provision(first).await.unwrap();
    let original = backend.0.lock().unwrap()[&first].clone();
    assert_eq!(original.len(), 32);
    assert!(matches!(
        keys.provision(first).await,
        Err(ProfileKeyError::AlreadyExists)
    ));
    keys.provision(second).await.unwrap();
    let stored = backend.0.lock().unwrap();
    assert_eq!(stored[&first], original);
    assert_ne!(stored[&first], stored[&second]);
}

#[tokio::test]
async fn malformed_keys_are_rejected_without_repair() {
    let backend = Arc::new(MemoryBackend::default());
    let profile = Uuid::new_v4();
    for size in [0, 31, 33] {
        backend.0.lock().unwrap().insert(profile, vec![7; size]);
        assert!(matches!(
            vault(backend.clone()).load(profile).await,
            Err(ProfileKeyError::InvalidKey)
        ));
        assert_eq!(backend.0.lock().unwrap()[&profile].len(), size);
    }
}

#[tokio::test]
async fn simultaneous_provisioning_has_one_winner() {
    let backend = Arc::new(MemoryBackend::default());
    let a = vault(backend.clone());
    let b = vault(backend.clone());
    let profile = Uuid::new_v4();
    let (a, b) = tokio::join!(a.provision(profile), b.provision(profile));
    assert!(matches!(
        (&a, &b),
        (Ok(_), Err(ProfileKeyError::AlreadyExists)) | (Err(ProfileKeyError::AlreadyExists), Ok(_))
    ));
    assert_eq!(backend.0.lock().unwrap().len(), 1);
}

struct DeniedBackend;
impl KeyBackend for DeniedBackend {
    fn load(&self, _: Uuid) -> Result<Zeroizing<Vec<u8>>> {
        Err(ProfileKeyError::Unavailable)
    }
    fn insert(&self, _: Uuid, _: &[u8]) -> Result<()> {
        Err(ProfileKeyError::Unavailable)
    }
}
#[tokio::test]
async fn access_failure_is_not_missing_and_does_not_fallback() {
    let keys = ProfileKeyVault {
        backend: Arc::new(DeniedBackend),
    };
    assert!(matches!(
        keys.load(Uuid::nil()).await,
        Err(ProfileKeyError::Unavailable)
    ));
    assert!(matches!(
        keys.provision(Uuid::nil()).await,
        Err(ProfileKeyError::Unavailable)
    ));
}

#[tokio::test]
async fn loaded_key_reopens_encrypted_fixture_after_vault_recreation() {
    use crate::snapshots::SnapshotStore;
    use nova_core::catalog::{snapshot::*, EndpointScope, SnapshotId, SourceId};
    use std::num::{NonZeroU32, NonZeroU64};
    let backend = Arc::new(MemoryBackend::default());
    let profile = Uuid::new_v4();
    let dir = tempfile::tempdir().unwrap();
    let payload = include_bytes!("../../../../fixtures/iptv/v1/playlist.m3u").to_vec();
    let mut capture = SnapshotCapture::new(
        SnapshotDescriptor {
            id: SnapshotId::new("a").unwrap(),
            source_id: SourceId::new("source").unwrap(),
            scope: EndpointScope::new("playlist").unwrap(),
            parser_revision: NonZeroU32::new(1).unwrap(),
        },
        SnapshotLimits {
            max_bytes: NonZeroU64::new(8192).unwrap(),
            max_records: NonZeroU64::new(10).unwrap(),
        },
    );
    capture.push_bytes(&payload).unwrap();
    let key = vault(backend.clone()).provision(profile).await.unwrap();
    let store = SnapshotStore::open(dir.path().to_owned(), profile, key, 8192)
        .await
        .unwrap();
    store
        .persist(
            capture.seal(Completeness::Complete).unwrap(),
            payload.clone(),
        )
        .await
        .unwrap();
    drop(store);
    let key = vault(backend).load(profile).await.unwrap();
    let store = SnapshotStore::open(dir.path().to_owned(), profile, key, 8192)
        .await
        .unwrap();
    assert_eq!(
        store
            .replay(SnapshotId::new("a").unwrap())
            .await
            .unwrap()
            .payload(),
        payload
    );
}
