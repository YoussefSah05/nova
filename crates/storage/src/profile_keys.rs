//! Profile snapshot-key lifecycle. No replacement, deletion, silent regeneration,
//! plaintext fallback or production importer wiring. Native operations may require
//! an entitled, signed application and execute only on blocking workers.
use crate::snapshots::ProfileKey;
use ring::rand::{SecureRandom, SystemRandom};
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProfileKeyError {
    #[error("profile snapshot key is missing; recovery is required")]
    Missing,
    #[error("profile snapshot key already exists")]
    AlreadyExists,
    #[error("profile snapshot key has invalid material")]
    InvalidKey,
    #[error("keychain is unavailable or access was denied")]
    Unavailable,
    #[error("keychain access requires application signing entitlements")]
    MissingEntitlement,
    #[error("secure random generation failed")]
    Random,
    #[error("profile key worker failed")]
    Worker,
}
type Result<T> = std::result::Result<T, ProfileKeyError>;

/// The insert operation MUST atomically reject existing items. Deliberately
/// private: app callers cannot accidentally supply an overwriting implementation.
trait KeyBackend: Send + Sync {
    fn load(&self, profile: Uuid) -> Result<Zeroizing<Vec<u8>>>;
    fn insert(&self, profile: Uuid, bytes: &[u8]) -> Result<()>;
}

/// A fresh key is provisioned only for a new profile/key namespace. An existing
/// profile whose key was lost must enter recovery, never call provision as repair.
/// Do not log keys or store them in preferences. No Debug/serialization is provided.
#[derive(Clone)]
pub struct ProfileKeyVault {
    backend: Arc<dyn KeyBackend>,
}
impl ProfileKeyVault {
    #[cfg(target_os = "macos")]
    pub fn mac_os() -> Self {
        Self {
            backend: Arc::new(native::MacKeychain {
                service: native::SERVICE.into(),
            }),
        }
    }

    /// Read only. Missing, inaccessible and malformed items are distinct failures.
    pub async fn load(&self, profile: Uuid) -> Result<ProfileKey> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = backend.load(profile)?;
            let key: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| ProfileKeyError::InvalidKey)?;
            Ok(ProfileKey::new(key))
        })
        .await
        .map_err(|_| ProfileKeyError::Worker)?
    }

    /// Insert only; returns AlreadyExists even when another process wins the race.
    /// A cancelled await can still leave a provisioned key: load to resolve it.
    pub async fn provision(&self, profile: Uuid) -> Result<ProfileKey> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || {
            let mut bytes = Zeroizing::new([0u8; 32]);
            SystemRandom::new()
                .fill(bytes.as_mut())
                .map_err(|_| ProfileKeyError::Random)?;
            backend.insert(profile, bytes.as_ref())?;
            Ok(ProfileKey::new(*bytes))
        })
        .await
        .map_err(|_| ProfileKeyError::Worker)?
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use core_foundation::data::CFData;
    use security_framework::item::{
        ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, Limit, Location, SearchResult,
    };

    pub(super) const SERVICE: &str = "app.nova.desktop.snapshot-key.v1";
    pub(super) struct MacKeychain {
        pub(super) service: String,
    }

    fn query(service: &str, profile: Uuid) -> ItemSearchOptions {
        let mut options = ItemSearchOptions::new();
        options
            .class(ItemClass::generic_password())
            .service(service)
            .account(&profile.to_string())
            .cloud_sync(Some(false))
            .ignore_legacy_keychains();
        options
    }

    // Public OSStatus values; no OS error description/account/data is logged.
    pub(super) fn classify(code: i32) -> ProfileKeyError {
        match code {
            -25300 => ProfileKeyError::Missing,
            -25299 => ProfileKeyError::AlreadyExists,
            -34018 => ProfileKeyError::MissingEntitlement,
            _ => ProfileKeyError::Unavailable,
        }
    }
    impl KeyBackend for MacKeychain {
        fn load(&self, profile: Uuid) -> Result<Zeroizing<Vec<u8>>> {
            let mut query = query(&self.service, profile);
            let results = query
                .load_data(true)
                .limit(Limit::Max(1))
                .search()
                .map_err(|error| classify(error.code()))?;
            let mut results = results.into_iter();
            match results.next() {
                Some(SearchResult::Data(bytes)) => Ok(Zeroizing::new(bytes)),
                _ => Err(ProfileKeyError::InvalidKey),
            }
        }
        fn insert(&self, profile: Uuid, bytes: &[u8]) -> Result<()> {
            let mut options = ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(bytes),
            });
            // SecItemAdd, never the upserting password helper. Apple defaults are
            // non-synchronizable and WhenUnlocked; choose data protection explicitly.
            options
                .set_service(&self.service)
                .set_account_name(profile.to_string())
                .set_location(Location::DataProtectionKeychain);
            options.add().map_err(|error| classify(error.code()))
        }
    }

    #[test]
    fn statuses_never_confuse_access_failure_with_missing() {
        assert_eq!(classify(-25300), ProfileKeyError::Missing);
        assert_eq!(classify(-25299), ProfileKeyError::AlreadyExists);
        assert_eq!(classify(-34018), ProfileKeyError::MissingEntitlement);
        for code in [-25308, -25293, -128, -50] {
            assert_eq!(classify(code), ProfileKeyError::Unavailable);
        }
    }

    #[test]
    #[ignore = "requires native Keychain access/signing; uses only a unique synthetic namespace"]
    fn native_keychain_roundtrip() {
        let service = format!("app.nova.desktop.snapshot-key.test.{}", Uuid::new_v4());
        let profile = Uuid::new_v4();
        let backend = Arc::new(MacKeychain {
            service: service.clone(),
        });
        let vault = ProfileKeyVault {
            backend: backend.clone(),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        // Clean only the exact synthetic item created by this test, even on panic.
        struct Cleanup {
            service: String,
            profile: Uuid,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = query(&self.service, self.profile).delete();
            }
        }
        let cleanup = Cleanup { service, profile };
        runtime.block_on(async {
            match vault.load(profile).await {
                Err(ProfileKeyError::Missing) => {}
                Err(error) => panic!("native read failed: {error}"),
                Ok(_) => panic!("unexpected item in unique test namespace"),
            }
            vault
                .provision(profile)
                .await
                .expect("native provision failed");
        });
        let original = backend.load(profile).unwrap();
        runtime.block_on(async {
            vault.load(profile).await.unwrap();
            assert!(matches!(
                vault.provision(profile).await,
                Err(ProfileKeyError::AlreadyExists)
            ));
        });
        let reread = backend.load(profile).unwrap();
        assert!(
            original.as_slice() == reread.as_slice(),
            "duplicate provisioning changed material"
        );
        query(&cleanup.service, profile).delete().unwrap();
        assert!(matches!(
            backend.load(profile),
            Err(ProfileKeyError::Missing)
        ));
        drop(cleanup);
    }
}

#[cfg(test)]
mod tests;
