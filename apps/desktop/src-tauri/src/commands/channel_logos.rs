//! Opt-in channel logos from the iptv-org directory (artwork v1).
//!
//! When the profile enabled it, `match_channel_logos` makes sure the compact
//! directory index is on device (downloading the public `channels.json` +
//! `logos.json` at most once a week), then matches the channels that have no
//! provider logo in the background (`nova_artwork::directory`) and stores the
//! result. Nothing leaves the device but those two downloads: channel names are
//! never sent anywhere. Progress: `artwork:channel-logos`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use nova_artwork::directory::{
    self, ChannelQuery, Directory, CHANNELS_URL, DIRECTORY_FORMAT, LOGOS_URL,
};
use nova_storage::{
    ChannelLogoRepository, DirectoryLogoWrite, ProfileSettingsRepository, SourceRepository,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use super::require_profile_pool;
use crate::ipc::ApiError;
use crate::state::AppState;

/// A directory index older than this is downloaded again.
const MAX_AGE_SECS: i64 = 7 * 24 * 3600;
/// Largest directory file accepted (the real ones are ~8 and ~6 MB).
const MAX_BODY: usize = 64 * 1024 * 1024;
const EVENT: &str = "artwork:channel-logos";

/// Requests accepted while a profile's pass runs; that pass drains them before
/// it ends, so a request made while it waits (e.g. for playback) is never lost.
#[derive(Debug, Default, PartialEq)]
struct Pending {
    all: bool,
    sources: BTreeSet<i64>,
    force: bool,
}

impl Pending {
    fn add(&mut self, source_id: Option<i64>, force: bool) {
        match source_id {
            None => self.all = true,
            Some(id) => {
                self.sources.insert(id);
            }
        }
        self.force |= force;
    }

    fn is_empty(&self) -> bool {
        !self.all && self.sources.is_empty()
    }
}

/// A profile's running pass: its identity and the requests not yet served.
#[derive(Debug, Default)]
struct Pass {
    id: u64,
    pending: Pending,
}

/// One pass per profile at a time.
static PASSES: LazyLock<Mutex<HashMap<String, Pass>>> = LazyLock::new(Default::default);
static NEXT_PASS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn passes() -> std::sync::MutexGuard<'static, HashMap<String, Pass>> {
    PASSES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Ends a pass however its task ends (a panic, or tokio dropping the task):
/// removes the profile's entry if it still belongs to this pass, so later
/// requests can start a new one. A newer pass's entry is never touched.
struct PassEnd {
    profile_id: String,
    id: u64,
}

impl Drop for PassEnd {
    fn drop(&mut self) {
        let mut passes = passes();
        if passes
            .get(&self.profile_id)
            .is_some_and(|p| p.id == self.id)
        {
            passes.remove(&self.profile_id);
        }
    }
}

/// Accept a request: `Some(pass id)` if a new pass must start, `None` if the
/// running pass of this profile will serve it.
fn accept(profile_id: &str, source_id: Option<i64>, force: bool) -> Option<u64> {
    let mut passes = passes();
    if let Some(pass) = passes.get_mut(profile_id) {
        pass.pending.add(source_id, force);
        return None;
    }
    let id = NEXT_PASS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut pending = Pending::default();
    pending.add(source_id, force);
    passes.insert(profile_id.to_owned(), Pass { id, pending });
    Some(id)
}

/// The next batch for pass `id`, or `None` (and the pass ends) when no
/// request is left.
fn next_batch(profile_id: &str, id: u64) -> Option<Pending> {
    let mut passes = passes();
    let pass = passes.get_mut(profile_id).filter(|p| p.id == id)?;
    let batch = std::mem::take(&mut pass.pending);
    if batch.is_empty() {
        passes.remove(profile_id);
        None
    } else {
        Some(batch)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLogosProgress {
    /// `downloading` | `matching` | `complete` | `failed`
    pub stage: &'static str,
    pub source_id: Option<i64>,
    pub matched: u64,
    pub candidates: u64,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoDirectoryStatus {
    pub downloaded: bool,
    pub entries: u64,
    pub built_at: Option<i64>,
}

fn directory_path(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .join("artwork-directory")
        .join("iptv-org.json"))
}

async fn load(path: &Path) -> Option<Directory> {
    let bytes = tokio::fs::read(path).await.ok()?;
    tokio::task::spawn_blocking(move || serde_json::from_slice::<Directory>(&bytes).ok())
        .await
        .ok()
        .flatten()
        .filter(|d| d.format == DIRECTORY_FORMAT)
}

async fn download(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BODY {
            return Err("directory file too large".into());
        }
    }
    String::from_utf8(body).map_err(|_| "directory file is not UTF-8".into())
}

/// The on-device index, refreshed when missing, stale or `force`d. A failed
/// refresh keeps using the previous index when there is one.
async fn ensure_directory(
    app: &AppHandle,
    force: bool,
    emit: &impl Fn(ChannelLogosProgress),
) -> Result<Arc<Directory>, String> {
    let path = directory_path(app).map_err(|e| format!("{e:?}"))?;
    let existing = load(&path).await;
    let fresh = existing
        .as_ref()
        .is_some_and(|d| nova_core::now_ts() - d.built_at < MAX_AGE_SECS);
    if let Some(d) = existing.as_ref().filter(|_| fresh && !force) {
        return Ok(Arc::new(d.clone()));
    }
    emit(progress("downloading", None, 0, 0, false, None));
    let fetched = async {
        let client = reqwest::Client::builder()
            .user_agent("Nova")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let channels = download(&client, CHANNELS_URL).await?;
        let logos = download(&client, LOGOS_URL).await?;
        let built_at = nova_core::now_ts();
        let dir =
            tokio::task::spawn_blocking(move || directory::build(&channels, &logos, built_at))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
        let json = serde_json::to_vec(&dir).map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, json)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<Directory, String>(dir)
    }
    .await;
    match (fetched, existing) {
        (Ok(dir), _) => Ok(Arc::new(dir)),
        (Err(e), Some(old)) => {
            tracing::warn!(error = %e, "logo directory refresh failed; using the previous index");
            Ok(Arc::new(old))
        }
        (Err(e), None) => Err(e),
    }
}

fn progress(
    stage: &'static str,
    source_id: Option<i64>,
    matched: u64,
    candidates: u64,
    done: bool,
    error: Option<String>,
) -> ChannelLogosProgress {
    ChannelLogosProgress {
        stage,
        source_id,
        matched,
        candidates,
        done,
        error,
    }
}

/// Match one source's logo-less channels and store the result.
async fn match_source(
    pool: &sqlx::SqlitePool,
    dir: Arc<Directory>,
    source_id: i64,
) -> Result<(u64, u64), ApiError> {
    let repo = ChannelLogoRepository::new(pool);
    let queries: Vec<ChannelQuery> = repo
        .candidates(source_id)
        .await?
        .into_iter()
        .map(|c| ChannelQuery {
            key: directory::channel_key(c.tvg_id.as_deref(), &c.name),
            tvg_id: c.tvg_id,
            name: c.name,
            country: c.country,
        })
        .collect();
    let candidates = queries.len() as u64;
    let matches = tokio::task::spawn_blocking(move || directory::match_channels(&dir, &queries))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    // Several channels can share a key (same tvg-id): one row per key.
    let mut seen = std::collections::HashSet::new();
    let writes: Vec<DirectoryLogoWrite> = matches
        .into_iter()
        .filter(|m| seen.insert(m.key.clone()))
        .map(|m| DirectoryLogoWrite {
            channel_key: m.key,
            directory_id: m.directory_id,
            url: m.logo_url,
            method: m.method.as_str(),
            confidence_bp: (m.confidence * 10_000.0).round() as i64,
        })
        .collect();
    repo.replace_directory_logos(source_id, &writes).await?;
    Ok((writes.len() as u64, candidates))
}

/// Start a background match for one source (or every source). `false` when the
/// feature is off; a request made while this profile's pass runs is served by it.
#[tauri::command]
pub async fn match_channel_logos(
    profile_id: String,
    source_id: Option<i64>,
    force_refresh: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    if !enabled(&pool).await? {
        return Ok(false);
    }
    let Some(pass) = accept(&profile_id, source_id, force_refresh.unwrap_or(false)) else {
        return Ok(true);
    };
    let admission = state.model_runtime.resource_admission();
    tauri::async_runtime::spawn(async move {
        let _end = PassEnd {
            profile_id: profile_id.clone(),
            id: pass,
        };
        while let Some(batch) = next_batch(&profile_id, pass) {
            run_pass(&pool, &app, &admission, batch).await;
        }
    });
    Ok(true)
}

async fn enabled(pool: &sqlx::SqlitePool) -> Result<bool, ApiError> {
    Ok(ProfileSettingsRepository::new(pool)
        .get()
        .await?
        .logo_directory_enabled)
}

/// One batch, as metadata-class work in the governor's job lane: it waits
/// while anything plays and yields to higher-priority jobs after the
/// directory download and before each source. The setting is re-checked
/// before each source, since a pass may wait a long time.
async fn run_pass(
    pool: &sqlx::SqlitePool,
    app: &AppHandle,
    admission: &Arc<crate::resource_policy::AdmissionController>,
    batch: Pending,
) {
    let emit = |p: ChannelLogosProgress| {
        let _ = app.emit(EVENT, p);
    };
    let single = (!batch.all && batch.sources.len() == 1)
        .then(|| batch.sources.first().copied())
        .flatten();
    let cancelled = || ApiError::Internal("channel logo matching was cancelled".into());
    let outcome = async {
        // Turned off while queued: no slot, no download.
        if !enabled(pool).await? {
            return Ok((0, 0));
        }
        let cancellation = crate::model_runtime::RuntimeCancellation::default();
        let mut slot = admission
            .acquire_job_slot(crate::resource_policy::WorkClass::Metadata, &cancellation)
            .await
            .map_err(|()| cancelled())?;
        let dir = ensure_directory(app, batch.force, &emit)
            .await
            .map_err(ApiError::Internal)?;
        let sources: Vec<i64> = if batch.all {
            SourceRepository::new(pool)
                .list()
                .await?
                .into_iter()
                .map(|s| s.id)
                .collect()
        } else {
            batch.sources.into_iter().collect()
        };
        let (mut matched, mut candidates) = (0, 0);
        for id in sources {
            slot.checkpoint(&cancellation).await.map_err(|()| cancelled())?;
            if !enabled(pool).await? {
                break;
            }
            emit(progress("matching", Some(id), matched, candidates, false, None));
            // One source failing (e.g. removed meanwhile) does not stop the rest.
            match match_source(pool, dir.clone(), id).await {
                Ok((m, c)) => {
                    matched += m;
                    candidates += c;
                }
                Err(e) => tracing::warn!(source_id = id, error = ?e, "channel logo matching skipped a source"),
            }
        }
        Ok::<_, ApiError>((matched, candidates))
    }
    .await;
    match outcome {
        Ok((matched, candidates)) => emit(progress(
            "complete", single, matched, candidates, true, None,
        )),
        Err(e) => {
            tracing::warn!(error = ?e, "channel logo matching failed");
            emit(progress(
                "failed",
                single,
                0,
                0,
                true,
                Some("Could not update channel logos".into()),
            ))
        }
    }
}

/// Forget every directory logo of the profile (the feature was turned off).
#[tauri::command]
pub async fn clear_channel_logo_matches(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelLogoRepository::new(&pool)
        .clear_directory_logos()
        .await?;
    Ok(())
}

/// Whether the directory index is on device, and how big and old it is.
#[tauri::command]
pub async fn get_logo_directory_status(app: AppHandle) -> Result<LogoDirectoryStatus, ApiError> {
    let dir = load(&directory_path(&app)?).await;
    Ok(LogoDirectoryStatus {
        downloaded: dir.is_some(),
        entries: dir.as_ref().map_or(0, |d| d.entries.len() as u64),
        built_at: dir.map(|d| d.built_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_artwork::directory::DirectoryEntry;
    use nova_core::ContentSort;
    use nova_storage::ChannelRepository;

    fn entry(id: &str, name: &str, country: &str) -> DirectoryEntry {
        DirectoryEntry {
            id: id.into(),
            name: name.into(),
            alt_names: vec![],
            country: Some(country.into()),
            closed: false,
            logo: format!("https://dir/{id}.png"),
        }
    }

    #[tokio::test]
    async fn fills_only_channels_without_a_provider_logo() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_profile_db(tmp.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let source = SourceRepository::new(&pool)
            .create("m3u", "fixture", "https://example.invalid/list")
            .await
            .unwrap()
            .id;
        for (tvg, name, country, logo) in [
            (Some("CNN.us@HD"), "CNN HD", Some("US"), None),
            (None, "TF1", Some("FR"), None),
            (None, "M6", Some("FR"), Some("https://provider/m6.png")),
            (None, "Unknown Local TV", Some("FR"), None),
        ] {
            sqlx::query(
                "INSERT INTO channels (source_id, tvg_id, name, country, logo_url, stream_url)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(source)
            .bind(tvg)
            .bind(name)
            .bind(country)
            .bind(logo)
            .bind(format!("https://example.invalid/{name}"))
            .execute(&pool)
            .await
            .unwrap();
        }
        let dir = Arc::new(Directory {
            format: DIRECTORY_FORMAT,
            built_at: 0,
            entries: vec![
                entry("CNN.us", "CNN", "US"),
                entry("TF1.fr", "TF1", "FR"),
                entry("M6.fr", "M6", "FR"),
            ],
        });
        let (matched, candidates) = match_source(&pool, dir, source).await.unwrap();
        assert_eq!((matched, candidates), (2, 3));

        let mut logos: Vec<(String, Option<String>)> = ChannelRepository::new(&pool)
            .list(
                source,
                10,
                0,
                None,
                None,
                None,
                None,
                false,
                false,
                ContentSort::Default,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|c| (c.name, c.logo_url))
            .collect();
        logos.sort();
        assert_eq!(
            logos,
            vec![
                ("CNN HD".into(), Some("https://dir/CNN.us.png".into())),
                ("M6".into(), Some("https://provider/m6.png".into())),
                ("TF1".into(), Some("https://dir/TF1.fr.png".into())),
                ("Unknown Local TV".into(), None),
            ]
        );
    }
}

#[cfg(test)]
mod pass_tests {
    use super::*;

    #[test]
    fn requests_during_a_pass_are_merged_and_drained_not_dropped() {
        let profile = "pass-test-profile";
        let pass = accept(profile, Some(1), false).expect("first request starts a pass");
        assert!(
            accept(profile, Some(2), true).is_none(),
            "later requests join it"
        );
        assert!(accept(profile, Some(1), false).is_none());
        let mut first = next_batch(profile, pass).unwrap();
        assert_eq!(std::mem::take(&mut first.sources), BTreeSet::from([1, 2]));
        assert!(first.force && !first.all);
        // A request arriving mid-batch is served by the next one.
        assert!(accept(profile, None, false).is_none());
        assert!(next_batch(profile, pass).unwrap().all);
        assert!(
            next_batch(profile, pass).is_none(),
            "drained: the pass ends"
        );
        let next = accept(profile, Some(3), false).expect("a new request starts a new pass");
        assert_ne!(next, pass);
        assert!(
            next_batch(profile, pass).is_none(),
            "an old pass never serves a new one"
        );
        drop(PassEnd {
            profile_id: profile.into(),
            id: pass,
        });
        assert!(
            next_batch(profile, next).is_some(),
            "an old pass's end leaves the new one"
        );
        assert!(next_batch(profile, next).is_none());
    }

    #[test]
    fn a_pass_that_dies_frees_its_profile() {
        let profile = "pass-test-dies";
        let pass = accept(profile, None, false).unwrap();
        drop(PassEnd {
            profile_id: profile.into(),
            id: pass,
        });
        let fresh = accept(profile, None, false).expect("a new pass can start");
        assert!(next_batch(profile, fresh).is_some());
        assert!(next_batch(profile, fresh).is_none());
    }
}
