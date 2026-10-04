//! Durable ownership markers for interrupted, unregistered copies.
use super::ConversionError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{collections::HashSet, path::Path};
use tokio::io::AsyncWriteExt;

const MARKER: &str = ".nova-recovery-v1.json";
pub(super) const PREFIX: &str = ".nova-recovery-v1-";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    version: u8,
    target: String,
    source: String,
}
impl Receipt {
    pub(super) fn new(target: String, source: String) -> Self {
        Self {
            version: 1,
            target,
            source,
        }
    }
    fn valid(&self) -> bool {
        self.version == 1
            && self.target != self.source
            && [&self.target, &self.source].iter().all(|s| {
                uuid::Uuid::parse_str(s)
                    .map(|id| id.to_string() == **s)
                    .unwrap_or(false)
            })
    }
    pub(super) async fn write(&self, directory: &Path) -> Result<(), ConversionError> {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(directory.join(MARKER)).await?;
        let bytes = serde_json::to_vec(self).map_err(|_| ConversionError::Filesystem)?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        sync_directory(directory).await
    }
}

pub(super) async fn sync_directory(path: &Path) -> Result<(), ConversionError> {
    #[cfg(unix)]
    tokio::fs::File::open(path).await?.sync_all().await?;
    Ok(())
}

pub(super) async fn lock(root: &Path) -> Result<std::fs::File, ConversionError> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(root.join(".nova-profile-recovery.lock"))
        .await?
        .into_std()
        .await;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ConversionError::Filesystem);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ConversionError::Filesystem);
        }
    }
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(ConversionError::Busy),
        // e.g. a filesystem without flock support: not "another recovery".
        Err(std::fs::TryLockError::Error(_)) => Err(ConversionError::Filesystem),
    }
}

async fn read_receipt(directory: &Path) -> Option<Receipt> {
    let metadata = tokio::fs::symlink_metadata(directory).await.ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let marker = directory.join(MARKER);
    let metadata = tokio::fs::symlink_metadata(&marker).await.ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 {
        return None;
    }
    let receipt: Receipt = serde_json::from_slice(&tokio::fs::read(marker).await.ok()?).ok()?;
    receipt.valid().then_some(receipt)
}

/// Remove a committed profile's ownership receipt. Once registered (and
/// whenever a profile is opened) it carries no receipt, so cleanup can never
/// remove it later, even after its app.db row is deleted or lost.
pub(super) async fn release(directory: &Path) -> Result<(), ConversionError> {
    match tokio::fs::symlink_metadata(directory).await {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(ConversionError::Filesystem),
    }
    match tokio::fs::remove_file(directory.join(MARKER)).await {
        Ok(()) => sync_directory(directory).await,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ConversionError::Filesystem),
    }
}

/// Called only with the process-wide recovery filesystem lock held. Never infer
/// ownership from a prefix alone, and never remove a registered profile directory.
/// Entries are reconciled independently: one stuck leftover is retained and
/// does not block later recoveries, which always use a fresh UUID.
pub(super) async fn cleanup(root: &Path, app_pool: &SqlitePool) -> Result<(), ConversionError> {
    let registered: HashSet<_> = crate::ProfileRepository::new(app_pool)
        .list()
        .await
        .map_err(|_| ConversionError::Database)?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(PREFIX) {
            continue;
        }
        if reconcile(root, &entry.path(), name, &registered)
            .await
            .is_err()
        {
            tracing::warn!("an interrupted profile recovery could not be removed; it is retained");
        }
    }
    sync_directory(root).await
}

async fn reconcile(
    root: &Path,
    staging: &Path,
    name: &str,
    registered: &HashSet<String>,
) -> Result<(), ConversionError> {
    let Some(receipt) = read_receipt(staging).await else {
        return discard_unwritten(staging).await;
    };
    if !name.starts_with(&format!("{PREFIX}{}-", receipt.target)) {
        return Ok(());
    }
    let target = root.join(&receipt.target);
    if registered.contains(&receipt.target) {
        release(&target).await?;
    } else if read_receipt(&target).await.as_ref() == Some(&receipt) {
        // A matching receipt proves this attempt published but never committed.
        // Move it inside the staging directory first: a crash during removal
        // then never leaves a partial directory under the profile's UUID.
        tokio::fs::rename(&target, staging.join("discarded")).await?;
        sync_directory(root).await?;
    }
    // Anything else under the target name is unproven and left untouched. The
    // staging directory holds only this attempt's receipt and private copy.
    remove_staging(staging).await
}

/// Remove a staging directory with its receipt last, so an interrupted or
/// failed removal still carries the proof a later cleanup needs to finish it.
pub(super) async fn remove_staging(staging: &Path) -> Result<(), ConversionError> {
    let mut entries = tokio::fs::read_dir(staging).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name() == MARKER {
            continue;
        }
        if entry.file_type().await?.is_dir() {
            tokio::fs::remove_dir_all(entry.path()).await?;
        } else {
            tokio::fs::remove_file(entry.path()).await?;
        }
    }
    sync_directory(staging).await?;
    match tokio::fs::remove_file(staging.join(MARKER)).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(ConversionError::Filesystem),
    }
    tokio::fs::remove_dir(staging).await?;
    Ok(())
}

/// A crash between creating a staging directory and completing its receipt
/// leaves it empty or holding only an incomplete marker; nothing else is written
/// before the receipt. Any other content, or a complete marker, is retained.
async fn discard_unwritten(staging: &Path) -> Result<(), ConversionError> {
    let metadata = tokio::fs::symlink_metadata(staging).await?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(staging).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = tokio::fs::symlink_metadata(entry.path()).await?;
        if entry.file_name() != MARKER || !metadata.is_file() || metadata.len() > 1024 {
            return Ok(());
        }
        let bytes = tokio::fs::read(entry.path()).await?;
        if serde_json::from_slice::<Receipt>(&bytes).is_ok() {
            return Ok(());
        }
        tokio::fs::remove_file(entry.path()).await?;
    }
    tokio::fs::remove_dir(staging).await?;
    Ok(())
}

#[cfg(test)]
pub(super) async fn checkpoint(stage: &str) {
    // Test builds only: the parent kills this child while it is paused at an
    // actual durable boundary. No fault-injection controls ship in the app.
    if std::env::var("NOVA_RECOVERY_TEST_STAGE").ok().as_deref() == Some(stage) {
        let marker = std::env::var_os("NOVA_RECOVERY_TEST_READY").expect("test ready path");
        tokio::fs::write(marker, b"ready")
            .await
            .expect("test ready marker");
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    async fn seed(root: &Path) -> (SqlitePool, String) {
        let app = crate::init_app_db(root.join("app.db").to_str().unwrap())
            .await
            .unwrap();
        let profile = crate::ProfileRepository::new(&app)
            .create("Crash fixture", "#e0a33e")
            .await
            .unwrap();
        let path = root.join("profiles").join(&profile.id);
        tokio::fs::create_dir_all(&path).await.unwrap();
        let pool = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path.join("profile.db"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
        crate::profile_recovery::historical_migrator()
            .run(&pool)
            .await
            .unwrap();
        pool.close().await;
        (app, profile.id)
    }

    #[test]
    #[ignore = "child process for abrupt termination tests"]
    fn recovery_kill_child() {
        let root = std::path::PathBuf::from(std::env::var_os("NOVA_RECOVERY_TEST_ROOT").unwrap());
        let id = std::env::var("NOVA_RECOVERY_TEST_ID").unwrap();
        let ready = std::path::PathBuf::from(std::env::var_os("NOVA_RECOVERY_TEST_READY").unwrap());
        let cancel = std::env::var_os("NOVA_RECOVERY_TEST_CANCEL").is_some();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let app = crate::init_app_db(root.join("app.db").to_str().unwrap())
                .await
                .unwrap();
            let profiles = root.join("profiles");
            let recovery = crate::profile_recovery::recover_as_profile(&app, &profiles, &id);
            if !cancel {
                recovery.await.unwrap();
                return;
            }
            // Drop the future at the checkpoint, as an aborted caller would,
            // then exit normally so destructors (not a kill) decide what stays.
            tokio::select! {
                _ = recovery => panic!("recovery should pause at the checkpoint"),
                _ = async {
                    while !ready.exists() {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => {}
            }
            app.close().await;
        });
    }

    /// Run a child recovery to `stage`, then kill it (or let it cancel itself).
    fn interrupt(root: &Path, id: &str, stage: &str, cancel: bool) {
        let ready = root.join("ready");
        let _ = std::fs::remove_file(&ready);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "profile_recovery::interruption::tests::recovery_kill_child",
            ])
            .env("NOVA_RECOVERY_TEST_ROOT", root)
            .env("NOVA_RECOVERY_TEST_ID", id)
            .env("NOVA_RECOVERY_TEST_READY", &ready)
            .env("NOVA_RECOVERY_TEST_STAGE", stage)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if cancel {
            command.env("NOVA_RECOVERY_TEST_CANCEL", "1");
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let reached = ready.exists();
        if cancel {
            let exit = child.wait().unwrap();
            assert!(exit.success(), "cancelled child exits cleanly at {stage}");
        } else {
            let _ = child.kill();
            child.wait().unwrap();
        }
        assert!(reached, "child reached {stage}");
    }

    fn directories(profiles: &Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(profiles)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .map(|e| e.path())
            .collect()
    }

    #[test]
    fn interrupted_recovery_cleans_only_unregistered_owned_copies() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let stages = [
            "unwritten",
            "staging",
            "converted",
            "assembling",
            "published",
            "registered",
        ];
        for (stage, cancel) in stages
            .iter()
            .map(|s| (*s, false))
            .chain(["converted", "published", "registered"].map(|s| (s, true)))
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, id) = runtime.block_on(seed(root));
            let source = root.join("profiles").join(&id).join("profile.db");
            let before = std::fs::read(&source).unwrap();
            runtime.block_on(app.close());
            interrupt(root, &id, stage, cancel);
            runtime.block_on(async {
                let app = crate::init_app_db(root.join("app.db").to_str().unwrap())
                    .await
                    .unwrap();
                let profiles = root.join("profiles");
                crate::profile_recovery::cleanup_interrupted_profiles(&app, &profiles)
                    .await
                    .unwrap();
                let rows = crate::ProfileRepository::new(&app).list().await.unwrap();
                assert_eq!(rows.len(), if stage == "registered" { 2 } else { 1 });
                assert_eq!(
                    directories(&profiles).len(),
                    rows.len(),
                    "no owned orphan remains after {stage} (cancel: {cancel})"
                );
                for profile in rows.iter().filter(|p| p.id != id) {
                    let copy = profiles.join(&profile.id);
                    assert!(
                        !copy.join(MARKER).exists(),
                        "committed copy keeps no receipt"
                    );
                    crate::init_profile_db(copy.join("profile.db").to_str().unwrap())
                        .await
                        .unwrap()
                        .close()
                        .await;
                }
                if stage != "registered" {
                    crate::profile_recovery::recover_as_profile(&app, &profiles, &id)
                        .await
                        .unwrap();
                }
                app.close().await;
            });
            assert_eq!(
                before,
                std::fs::read(&source).unwrap(),
                "source unchanged after {stage}"
            );
        }
    }

    #[test]
    fn opened_then_deleted_recovered_profile_is_never_cleaned_up() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (app, id) = runtime.block_on(seed(root));
        runtime.block_on(app.close());
        // Exit after registration, before the receipt is released or staging removed.
        interrupt(root, &id, "registered", false);
        runtime.block_on(async {
            let app = crate::init_app_db(root.join("app.db").to_str().unwrap())
                .await
                .unwrap();
            let profiles = root.join("profiles");
            let repo = crate::ProfileRepository::new(&app);
            let copy = repo
                .list()
                .await
                .unwrap()
                .into_iter()
                .find(|p| p.id != id)
                .unwrap();
            let copy_dir = profiles.join(&copy.id);
            // The app opens it (releasing the receipt), uses it, then deletes the row;
            // the directory intentionally stays on disk.
            crate::profile_recovery::release_recovery_receipt(&copy_dir)
                .await
                .unwrap();
            repo.delete(&copy.id).await.unwrap();
            crate::profile_recovery::cleanup_interrupted_profiles(&app, &profiles)
                .await
                .unwrap();
            assert!(copy_dir.join("profile.db").exists(), "user's copy retained");
            assert_eq!(directories(&profiles).len(), 2, "only staging was removed");
            app.close().await;
        });
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stuck_leftover_does_not_block_later_recoveries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (app, source) = seed(dir.path()).await;
        let root = dir.path().join("profiles");
        let target = uuid::Uuid::new_v4().to_string();
        let stuck = root.join(format!("{PREFIX}{target}-stuck"));
        tokio::fs::create_dir(&stuck).await.unwrap();
        Receipt::new(target, source.clone())
            .write(&stuck)
            .await
            .unwrap();
        // A valid leftover whose removal fails partway: a locked subdirectory.
        let locked = stuck.join("discarded");
        tokio::fs::create_dir(&locked).await.unwrap();
        tokio::fs::write(locked.join("profile.db"), b"copy")
            .await
            .unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let recovered = crate::profile_recovery::recover_as_profile(&app, &root, &source).await;
        assert!(recovered.is_ok(), "a stuck leftover is retained, not fatal");
        assert!(stuck.join(MARKER).exists(), "the receipt is removed last");
        // Once removable, a later cleanup finishes the interrupted removal.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        crate::profile_recovery::cleanup_interrupted_profiles(&app, &root)
            .await
            .unwrap();
        assert!(!stuck.exists());
        app.close().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lock_refuses_shared_or_redirected_lock_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".nova-profile-recovery.lock");
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            lock(dir.path()).await,
            Err(ConversionError::Filesystem)
        ));
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), &path).unwrap();
        assert!(matches!(
            lock(dir.path()).await,
            Err(ConversionError::Filesystem)
        ));
        assert!(!dir.path().join("elsewhere").exists());
    }

    #[tokio::test]
    async fn cleanup_refuses_contention_unknown_markers_and_redirected_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (app, source) = seed(dir.path()).await;
        let root = dir.path().join("profiles");
        let lease = lock(&root).await.unwrap();
        assert!(matches!(
            crate::profile_recovery::cleanup_interrupted_profiles(&app, &root).await,
            Err(ConversionError::Busy)
        ));
        drop(lease);
        let unknown = root.join(".nova-recovery-v1-unknown");
        tokio::fs::create_dir(&unknown).await.unwrap();
        tokio::fs::write(unknown.join("keep"), b"unknown data")
            .await
            .unwrap();
        let malicious = Receipt::new(source.clone(), source.clone());
        let redirect = root.join(format!(".nova-recovery-v1-{source}-fixture"));
        tokio::fs::create_dir(&redirect).await.unwrap();
        malicious.write(&redirect).await.unwrap();
        #[cfg(unix)]
        {
            let link = root.join(format!(".nova-recovery-v1-{}-linked", uuid::Uuid::new_v4()));
            std::os::unix::fs::symlink(&unknown, link).unwrap();
        }
        crate::profile_recovery::cleanup_interrupted_profiles(&app, &root)
            .await
            .unwrap();
        assert!(unknown.join("keep").exists());
        assert!(redirect.exists());
        assert!(root.join(source).join("profile.db").exists());
        app.close().await;
    }
}
