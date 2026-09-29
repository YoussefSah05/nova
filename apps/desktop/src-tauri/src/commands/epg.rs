//! Phase 4 — EPG (Electronic Program Guide) commands.
//!
//! Sourcing: provider feeds first (Xtream `xmltv.php`, M3U `url-tvg`) are
//! auto-registered during `add_source`; community fallbacks (iptv-org, epg.pw,
//! …) get a manual `add_epg_url` entry point so users can paste any XMLTV URL.
//!
//! Refresh: a manual `epg_refresh` command per source. For each of the
//! source's feeds the worker spools the XMLTV body to a temp file (refusing
//! feeds over [`MAX_EPG_BYTES`]), parses it on a blocking thread into bounded
//! staged batches, then replaces *that feed's* programmes in one short
//! transaction — feeds covering the same channel never overwrite each other,
//! and a feed that parses to nothing keeps its previous guide — then prunes
//! anything older than one day. Memory stays flat whatever the feed size.
//! Progress is emitted on the `epg:refresh:progress` event channel.

use std::sync::Arc;
use std::time::Duration;

use nova_core::now_ts;
use nova_epg::Program;
use nova_storage::{
    ChannelGuide, ChannelNowAndNext, ChannelRepository, EpgFeedStatus, EpgSourcesRepository,
    LiveOverview, NowAndNext, ProgramsRepository,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

const REFRESH_BATCH: usize = 500;
/// Malformed programmes in a row after which a feed is not XMLTV at all.
const MAX_CONSECUTIVE_PARSE_ERRORS: usize = 1_000;
/// Parsed batches in flight between the parser thread and the inserter.
const BATCHES_IN_FLIGHT: usize = 4;
/// XMLTV feeds larger than this are refused (full multi-country guides run to
/// a few hundred MB; anything bigger is a mistake or hostile).
pub(crate) const MAX_EPG_BYTES: u64 = 512 * 1024 * 1024;
/// A gzip feed may inflate to at most this much XML (a bomb stops here).
const MAX_EPG_INFLATED_BYTES: u64 = 4 * MAX_EPG_BYTES;
/// Most channels one guide request reads.
const MAX_GUIDE_CHANNELS: usize = 200;
/// The widest guide window one request reads (13 hours).
const MAX_GUIDE_WINDOW_SECS: i64 = 13 * 3600;

/// Refuse oversized guide requests; a window is `[from, to)` with `from < to`.
fn check_guide_request(ids: usize, from_utc: i64, to_utc: i64) -> Result<(), ApiError> {
    if ids > MAX_GUIDE_CHANNELS {
        return Err(ApiError::InvalidInput(format!(
            "at most {MAX_GUIDE_CHANNELS} channels per guide request"
        )));
    }
    if to_utc <= from_utc || to_utc - from_utc > MAX_GUIDE_WINDOW_SECS {
        return Err(ApiError::InvalidInput(
            "a guide window is at most 13 hours".into(),
        ));
    }
    Ok(())
}

// ── Event payload ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpgRefreshProgress {
    pub source_id: i64,
    pub epg_source_id: i64,
    pub stage: EpgRefreshStage,
    pub inserted: u64,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EpgRefreshStage {
    Downloading,
    Parsing,
    Complete,
    Failed,
}

fn emit(app: &AppHandle, payload: EpgRefreshProgress) {
    let _ = app.emit("epg:refresh:progress", payload);
}

// ── Commands ──────────────────────────────────────────────────────────────────

/// Now-and-next for a list of channel TVG IDs. Returns one entry per ID —
/// entries without EPG data still come back with both fields `None` so the
/// frontend can render a stable list. With `source_id`, that source's feeds
/// win where several feeds cover a channel.
#[tauri::command]
pub async fn epg_now_and_next(
    profile_id: String,
    tvg_ids: Vec<String>,
    source_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<NowAndNext>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let now = now_ts();
    ProgramsRepository::new(&pool)
        .now_and_next_batch(&tvg_ids, now, source_id)
        .await
        .map_err(Into::into)
}

/// All programmes for a list of channel TVG IDs whose [start, stop) interval
/// overlaps `[from_utc, to_utc)`. Used by the full grid guide view. With
/// `source_id`, that source's feeds win where feeds overlap.
#[tauri::command]
pub async fn epg_range(
    profile_id: String,
    tvg_ids: Vec<String>,
    from_utc: i64,
    to_utc: i64,
    source_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<Program>, ApiError> {
    check_guide_request(tvg_ids.len(), from_utc, to_utc)?;
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProgramsRepository::new(&pool)
        .range(&tvg_ids, from_utc, to_utc, source_id)
        .await
        .map_err(Into::into)
}

/// The guide grid for channel rows: each channel's programmes overlapping
/// `[from_utc, to_utc)`, from its own source's feed where it has one. One
/// entry per requested id, in order (empty without a guide). At most
/// [`MAX_GUIDE_CHANNELS`] channels and a 13-hour window per request.
#[tauri::command]
pub async fn epg_guide(
    profile_id: String,
    channel_ids: Vec<i64>,
    from_utc: i64,
    to_utc: i64,
    state: State<'_, AppState>,
) -> Result<Vec<ChannelGuide>, ApiError> {
    check_guide_request(channel_ids.len(), from_utc, to_utc)?;
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProgramsRepository::new(&pool)
        .guide_for_channels(&channel_ids, from_utc, to_utc)
        .await
        .map_err(Into::into)
}

/// The Live section's overview of a playlist: channel count, how many have
/// guide data, and channels per category.
#[tauri::command]
pub async fn list_live_overview(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<LiveOverview, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ChannelRepository::new(&pool)
        .live_overview(source_id, now_ts())
        .await
        .map_err(Into::into)
}

/// Manually register an XMLTV URL for a source. Use this to wire up community
/// EPG feeds (iptv-org, epg.pw, EPGTalk, etc.) for sources that don't ship
/// their own EPG, or to supplement provider EPG with extra channel coverage.
/// The feed is refreshed straight away (a queued EPG job); a failure to
/// queue it is logged — the feed is saved and the next refresh fetches it.
#[tauri::command]
pub async fn add_epg_url(
    profile_id: String,
    source_id: i64,
    url: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    EpgSourcesRepository::new(&pool)
        .upsert(source_id, url.trim(), "community")
        .await?;
    if let Err(error) = state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .epg_refresh(source_id)
        .await
    {
        tracing::warn!(%error, source_id, "could not queue EPG refresh for a new feed");
    }
    Ok(())
}

/// Now-and-next for channel rows (what the browse cards and the live player
/// hold): each channel reads the guide of its own source where it has one.
/// One entry per requested id, empty when the channel has no guide.
#[tauri::command]
pub async fn epg_channels_now_and_next(
    profile_id: String,
    channel_ids: Vec<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<ChannelNowAndNext>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProgramsRepository::new(&pool)
        .now_and_next_for_channels(&channel_ids, now_ts())
        .await
        .map_err(Into::into)
}

/// A source's guide feeds: kind, last refresh, last error and programme
/// count. Never the feed URLs (an Xtream feed URL carries credentials).
#[tauri::command]
pub async fn list_epg_feeds(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
) -> Result<Vec<EpgFeedStatus>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    EpgSourcesRepository::new(&pool)
        .feed_status(source_id)
        .await
        .map_err(Into::into)
}

/// Trigger an EPG refresh for every registered XMLTV URL for the source.
/// Runs in a spawned task and emits progress events; returns immediately so
/// the UI doesn't block on a multi-minute download.
#[tauri::command]
pub async fn epg_refresh(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .epg_refresh(source_id)
        .await
        .map_err(ApiError::Internal)
}

/// How old a feed's last refresh (success or failure) may be before the
/// guide is refreshed on its own: feeds cover a day or more ahead, and a
/// failing feed is retried at most twice a day.
const GUIDE_STALE_SECS: i64 = 12 * 3600;

/// Whether any of a source's feeds is due: never refreshed, or not within
/// [`GUIDE_STALE_SECS`].
fn guide_due(feeds: &[nova_storage::EpgSource], now: i64) -> bool {
    feeds.iter().any(|f| {
        f.last_refresh_utc
            .is_none_or(|at| now - at >= GUIDE_STALE_SECS)
    })
}

/// Refresh the source's guide when it is due and no refresh is already
/// queued or running (the Live section asks when it opens). Returns whether
/// a refresh was queued.
#[tauri::command]
pub async fn epg_refresh_if_stale(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<bool, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let feeds = EpgSourcesRepository::new(&pool)
        .list_for_source(source_id)
        .await?;
    if !guide_due(&feeds, now_ts()) {
        return Ok(false);
    }
    // A refresh already queued or running (after an import) covers it; a
    // second one would download every feed again.
    if nova_storage::JobRepository::new(&pool)
        .has_active(crate::desktop_jobs::EPG_KIND, &source_id.to_string())
        .await?
    {
        return Ok(false);
    }
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .epg_refresh(source_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(true)
}

// ── Refresh worker ────────────────────────────────────────────────────────────

pub(crate) async fn run_epg_refresh_job(
    source_id: i64,
    pool: Arc<SqlitePool>,
    app: AppHandle,
    cancellation: &crate::model_runtime::RuntimeCancellation,
) -> Result<(), String> {
    let epg_sources = EpgSourcesRepository::new(&pool)
        .list_for_source(source_id)
        .await
        .map_err(|_| "could not list EPG sources".to_string())?;
    if epg_sources.is_empty() {
        return Ok(());
    }
    let mut complete = true;
    for es in epg_sources {
        // Each feed commits on its own; stop between feeds once cancelled.
        if cancellation.is_cancelled() {
            return Err("EPG refresh cancelled".into());
        }
        complete &= run_refresh(source_id, es.id, &es.url, &pool, app.clone()).await;
    }
    // Keep one day of history after all source transactions complete.
    let cutoff = now_ts() - 86_400;
    ProgramsRepository::new(&pool)
        .prune_before(cutoff)
        .await
        .map_err(|_| "EPG prune failed".to_string())?;
    if complete {
        Ok(())
    } else {
        Err("EPG refresh failed".into())
    }
}

async fn run_refresh(
    source_id: i64,
    epg_source_id: i64,
    url: &str,
    pool: &Arc<SqlitePool>,
    app: AppHandle,
) -> bool {
    let started_at = std::time::Instant::now();
    emit(
        &app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Downloading,
            inserted: 0,
            elapsed_ms: 0,
            error: None,
        },
    );

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .user_agent("Mozilla/5.0 (compatible; nova-iptv/1.0)")
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                e.to_string(),
                pool,
            )
            .await
        }
    };

    let response = match client.get(url).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                format!("HTTP {}", r.status()),
                pool,
            )
            .await;
        }
        Err(e) => {
            return fail(
                &app,
                source_id,
                epg_source_id,
                started_at,
                e.without_url().to_string(),
                pool,
            )
            .await
        }
    };
    let spooled = match spool_capped(response, MAX_EPG_BYTES).await {
        Ok(file) => file,
        Err(e) => return fail(&app, source_id, epg_source_id, started_at, e, pool).await,
    };

    emit(
        &app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Parsing,
            inserted: 0,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: None,
        },
    );

    // Channel logos this feed declares are matched to the source's channels
    // the same way programmes are (tvg-id). A read failure only costs logos.
    let source_tvg_ids = match EpgSourcesRepository::new(pool)
        .channel_tvg_ids(source_id)
        .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "epg_refresh: channel tvg-ids unavailable");
            Vec::new()
        }
    };
    let insert_result = ingest_feed(pool, source_id, epg_source_id, spooled, source_tvg_ids).await;

    match insert_result {
        Ok(inserted) => {
            if let Err(e) = EpgSourcesRepository::new(pool)
                .touch_success(epg_source_id)
                .await
            {
                tracing::warn!(error = %e, "epg_refresh: touch_success failed");
            }
            emit(
                &app,
                EpgRefreshProgress {
                    source_id,
                    epg_source_id,
                    stage: EpgRefreshStage::Complete,
                    inserted,
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    error: None,
                },
            );
            tracing::info!(
                source_id,
                epg_source_id,
                inserted,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "EPG refresh complete"
            );
            true
        }
        Err(e) => fail(&app, source_id, epg_source_id, started_at, e, pool).await,
    }
}

/// Write a response body to a temp file, refusing it once it passes `cap`
/// bytes (checked while streaming, and up front when the length is declared).
pub(crate) async fn spool_capped(
    mut response: reqwest::Response,
    cap: u64,
) -> Result<std::fs::File, String> {
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
    let too_large = || format!("EPG feed is larger than {} MB", cap / (1024 * 1024));
    if response.content_length().is_some_and(|len| len > cap) {
        return Err(too_large());
    }
    let mut file =
        tokio::fs::File::from_std(tempfile::tempfile().map_err(|e| format!("EPG spool: {e}"))?);
    let mut written: u64 = 0;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                written += chunk.len() as u64;
                if written > cap {
                    return Err(too_large());
                }
                file.write_all(&chunk)
                    .await
                    .map_err(|e| format!("EPG spool: {e}"))?;
            }
            Ok(None) => break,
            Err(e) => return Err(e.without_url().to_string()),
        }
    }
    file.flush().await.map_err(|e| format!("EPG spool: {e}"))?;
    file.rewind().await.map_err(|e| format!("EPG spool: {e}"))?;
    Ok(file.into_std().await)
}

/// The feed's XML: gzip feeds (`.xml.gz`, served as-is by many community
/// guides) are inflated as they are parsed; past [`MAX_EPG_INFLATED_BYTES`]
/// the read fails and so does the feed.
fn xml_reader(mut feed: std::fs::File) -> std::io::Result<Box<dyn std::io::Read + Send>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut magic = [0u8; 2];
    let read = feed.read(&mut magic)?;
    feed.seek(SeekFrom::Start(0))?;
    if read == 2 && magic == [0x1f, 0x8b] {
        Ok(Box::new(Capped {
            inner: flate2::read::MultiGzDecoder::new(feed),
            left: MAX_EPG_INFLATED_BYTES,
        }))
    } else {
        Ok(Box::new(feed))
    }
}

/// A reader that fails (rather than ending quietly) once it has produced
/// `left` bytes, so an oversized feed is refused instead of published cut.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: std::io::Read> std::io::Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n as u64 > self.left {
            return Err(std::io::Error::other(
                "EPG feed inflates past the size limit",
            ));
        }
        self.left -= n as u64;
        Ok(n)
    }
}

enum ParsedChunk {
    Programs(Vec<Program>),
    /// The feed can't be read to its end; nothing is published.
    Failed(String),
    /// The feed ended; its `<channel>` icons.
    Done(Vec<nova_epg::ChannelIcon>),
}

/// Parse a spooled XMLTV feed on a blocking thread, stage its programmes
/// batch by batch (at most [`BATCHES_IN_FLIGHT`] batches are ever held in
/// memory), then replace the feed's programmes and logos in one short
/// transaction. A feed that parses to no programmes is refused.
pub(crate) async fn ingest_feed(
    pool: &SqlitePool,
    source_id: i64,
    epg_source_id: i64,
    feed: std::fs::File,
    source_tvg_ids: Vec<String>,
) -> Result<u64, String> {
    let (tx_batches, mut rx_batches) = tokio::sync::mpsc::channel(BATCHES_IN_FLIGHT);
    let parser = tokio::task::spawn_blocking(move || {
        let reader = match xml_reader(feed) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "EPG feed unreadable");
                return;
            }
        };
        let mut programs = nova_epg::parse(std::io::BufReader::new(reader));
        let mut batch = Vec::with_capacity(REFRESH_BATCH);
        let mut consecutive_errors = 0usize;
        for p in programs.by_ref() {
            match p {
                // A stream that fails part-way would publish half a guide
                // over the whole one: fail the feed and keep its guide.
                Err(e) if e.is_io() => {
                    let _ = tx_batches.blocking_send(ParsedChunk::Failed(format!(
                        "EPG feed is truncated or corrupt: {e}"
                    )));
                    return;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_PARSE_ERRORS {
                        let _ = tx_batches.blocking_send(ParsedChunk::Failed(
                            "EPG feed is not readable XMLTV".to_string(),
                        ));
                        return;
                    }
                    // A broken element shouldn't discard the entire feed.
                    tracing::warn!(error = %e, "EPG parse: skipped malformed programme");
                }
                Ok(p) => {
                    consecutive_errors = 0;
                    batch.push(p);
                    if batch.len() == REFRESH_BATCH {
                        let full = std::mem::replace(&mut batch, Vec::with_capacity(REFRESH_BATCH));
                        if tx_batches
                            .blocking_send(ParsedChunk::Programs(full))
                            .is_err()
                        {
                            return; // the inserter gave up
                        }
                    }
                }
            }
        }
        if !batch.is_empty()
            && tx_batches
                .blocking_send(ParsedChunk::Programs(batch))
                .is_err()
        {
            return;
        }
        let _ = tx_batches.blocking_send(ParsedChunk::Done(programs.take_channel_icons()));
    });

    let programs = ProgramsRepository::new(pool);
    let result: Result<u64, String> = async {
        programs
            .clear_staged(epg_source_id)
            .await
            .map_err(|e| e.to_string())?;
        // Batches are staged in short writes while the feed is still being
        // parsed; the database write lock is only held to publish them.
        let mut staged: u64 = 0;
        let icons = loop {
            match rx_batches.recv().await {
                Some(ParsedChunk::Programs(batch)) => {
                    staged += programs
                        .stage_batch(epg_source_id, &batch)
                        .await
                        .map_err(|e| e.to_string())?;
                }
                Some(ParsedChunk::Done(icons)) => break icons,
                Some(ParsedChunk::Failed(error)) => return Err(error),
                None => return Err("EPG parser stopped unexpectedly".to_string()),
            }
        };
        // An empty body or an HTML error page parses to nothing: keep the
        // guide the feed had.
        if staged == 0 {
            return Err("EPG feed contained no programmes".to_string());
        }
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let published = ProgramsRepository::publish_staged_tx(&mut tx, epg_source_id)
            .await
            .map_err(|e| e.to_string())?;
        let logos = nova_epg::match_channel_icons(&source_tvg_ids, &icons);
        EpgSourcesRepository::replace_channel_logos_tx(&mut tx, epg_source_id, source_id, &logos)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(published)
    }
    .await;
    if result.is_err() {
        if let Err(e) = programs.clear_staged(epg_source_id).await {
            tracing::warn!(error = %e, "epg_refresh: clearing staged programmes failed");
        }
    }
    // On failure the receiver is dropped: the parser stops at its next send.
    drop(rx_batches);
    let _ = parser.await;
    result
}

async fn fail(
    app: &AppHandle,
    source_id: i64,
    epg_source_id: i64,
    started_at: std::time::Instant,
    error: String,
    pool: &Arc<SqlitePool>,
) -> bool {
    tracing::warn!(source_id, epg_source_id, error = %error, "EPG refresh failed");
    if let Err(e) = EpgSourcesRepository::new(pool)
        .touch_failure(epg_source_id, &error)
        .await
    {
        tracing::warn!(error = %e, "epg_refresh: touch_failure failed");
    }
    emit(
        app,
        EpgRefreshProgress {
            source_id,
            epg_source_id,
            stage: EpgRefreshStage::Failed,
            inserted: 0,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: Some(error),
        },
    );
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn profile_with_feed() -> (tempfile::TempDir, SqlitePool, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let source = nova_storage::SourceRepository::new(&pool)
            .create("xtream", "t", "http://h/get.php")
            .await
            .unwrap()
            .id;
        let feeds = EpgSourcesRepository::new(&pool);
        feeds
            .upsert(source, "http://h/xmltv.php", "xtream")
            .await
            .unwrap();
        feeds
            .upsert(source, "http://h/community.xml", "community")
            .await
            .unwrap();
        let feed = feeds.list_for_source(source).await.unwrap()[0].id;
        (dir, pool, source, feed)
    }

    fn xmltv(programmes: usize) -> std::fs::File {
        let mut file = tempfile::tempfile().unwrap();
        let mut body = String::from(
            "<?xml version=\"1.0\"?><tv><channel id=\"cnn.us\"><icon src=\"https://img.example/cnn.png\"/></channel>",
        );
        for n in 0..programmes {
            let channel = if n % 2 == 0 { "cnn.us" } else { "bbc.uk" };
            body.push_str(&format!(
                "<programme channel=\"{channel}\" start=\"20260101{:02}0000 +0000\" stop=\"20260101{:02}3000 +0000\"><title>Show {n}</title></programme>",
                n % 24,
                n % 24
            ));
        }
        body.push_str("</tv>");
        file.write_all(body.as_bytes()).unwrap();
        std::io::Seek::rewind(&mut file).unwrap();
        file
    }

    #[tokio::test]
    async fn a_feed_is_ingested_in_bounded_batches_and_replaces_only_itself() {
        let (_dir, pool, source, feed) = profile_with_feed().await;
        let other = EpgSourcesRepository::new(&pool)
            .list_for_source(source)
            .await
            .unwrap()[1]
            .id;
        // Another feed's programme for the same channel.
        let programs = ProgramsRepository::new(&pool);
        programs
            .stage_batch(
                other,
                &[Program {
                    channel_tvg_id: "cnn.us".into(),
                    title: "community".into(),
                    sub_title: None,
                    description: None,
                    start_utc: 0,
                    stop_utc: 10,
                    category: None,
                    icon_url: None,
                }],
            )
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        ProgramsRepository::publish_staged_tx(&mut tx, other)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let tvg_ids = vec!["cnn.us".to_string()];
        let n = 3 * REFRESH_BATCH + 7; // several batches through the channel
        let inserted = ingest_feed(&pool, source, feed, xmltv(n), tvg_ids.clone())
            .await
            .unwrap();
        assert_eq!(inserted, n as u64);
        let again = ingest_feed(&pool, source, feed, xmltv(10), tvg_ids)
            .await
            .unwrap();
        assert_eq!(again, 10);
        let counts: Vec<(Option<i64>, i64)> = sqlx::query_as(
            "SELECT epg_source_id, COUNT(*) FROM programs GROUP BY epg_source_id ORDER BY epg_source_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            counts,
            [(Some(feed), 10), (Some(other), 1)],
            "replaced itself only"
        );
        let logos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM epg_channel_logos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(logos, 1, "the feed's channel icon is kept");
    }

    #[tokio::test]
    async fn a_feed_that_parses_to_nothing_keeps_its_guide() {
        let (_dir, pool, source, feed) = profile_with_feed().await;
        let tvg_ids = vec!["cnn.us".to_string()];
        ingest_feed(&pool, source, feed, xmltv(4), tvg_ids.clone())
            .await
            .unwrap();
        for body in ["<tv></tv>", "<html><body>Please log in</body></html>", ""] {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(body.as_bytes()).unwrap();
            std::io::Seek::rewind(&mut file).unwrap();
            let refused = ingest_feed(&pool, source, feed, file, tvg_ids.clone())
                .await
                .unwrap_err();
            assert!(refused.contains("no programmes"), "{refused}");
        }
        let (kept, staged): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM programs), (SELECT COUNT(*) FROM program_staging)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            (kept, staged),
            (4, 0),
            "the previous guide stays, staging is clean"
        );
    }

    #[tokio::test]
    async fn gzip_feeds_are_inflated_while_parsing() {
        use std::io::Read;
        let (_dir, pool, source, feed) = profile_with_feed().await;
        let mut xml = Vec::new();
        xmltv(6).read_to_end(&mut xml).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&xml).unwrap();
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&gz.finish().unwrap()).unwrap();
        std::io::Seek::rewind(&mut file).unwrap();
        let inserted = ingest_feed(&pool, source, feed, file, vec!["cnn.us".into()])
            .await
            .unwrap();
        assert_eq!(inserted, 6);
    }

    fn gzip_of(programmes: usize) -> Vec<u8> {
        use std::io::Read;
        let mut xml = Vec::new();
        xmltv(programmes).read_to_end(&mut xml).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&xml).unwrap();
        gz.finish().unwrap()
    }

    fn file_of(bytes: &[u8]) -> std::fs::File {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        std::io::Seek::rewind(&mut file).unwrap();
        file
    }

    #[tokio::test]
    async fn a_damaged_gzip_feed_fails_and_keeps_the_guide_it_had() {
        let (_dir, pool, source, feed) = profile_with_feed().await;
        let tvg = vec!["cnn.us".to_string()];
        ingest_feed(&pool, source, feed, xmltv(4), tvg.clone())
            .await
            .unwrap();
        let whole = gzip_of(2_000);
        let mut trailing = whole.clone();
        trailing.extend_from_slice(b"garbage");
        for damaged in [
            whole[..whole.len() / 2].to_vec(),
            whole[..whole.len() - 4].to_vec(),
            trailing,
        ] {
            // Bounded: a stuck parser would hang here.
            let result = tokio::time::timeout(
                Duration::from_secs(20),
                ingest_feed(&pool, source, feed, file_of(&damaged), tvg.clone()),
            )
            .await
            .expect("a damaged feed must not hang the refresh");
            let error = result.unwrap_err();
            assert!(error.contains("truncated or corrupt"), "{error}");
        }
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM programs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kept, 4, "the previous guide stays");
    }

    #[test]
    fn inflating_past_the_cap_is_an_error_not_a_quiet_cut() {
        use std::io::Read;
        let mut capped = Capped {
            inner: std::io::Cursor::new(vec![b'x'; 100]),
            left: 50,
        };
        let mut out = Vec::new();
        assert!(capped.read_to_end(&mut out).is_err());
    }

    #[test]
    fn a_guide_is_due_when_a_feed_is_old_or_never_refreshed() {
        let feed = |at: Option<i64>| nova_storage::EpgSource {
            id: 1,
            source_id: 1,
            url: "http://h/x".into(),
            kind: "xtream".into(),
            last_refresh_utc: at,
            last_error: None,
        };
        let now = 100_000;
        assert!(!guide_due(&[], now), "no feeds, nothing to refresh");
        assert!(!guide_due(&[feed(Some(now - 3600))], now));
        assert!(guide_due(&[feed(Some(now - GUIDE_STALE_SECS))], now));
        assert!(guide_due(&[feed(Some(now)), feed(None)], now));
    }

    #[test]
    fn guide_requests_are_bounded() {
        assert!(check_guide_request(30, 0, 4 * 3600).is_ok());
        assert!(check_guide_request(MAX_GUIDE_CHANNELS + 1, 0, 3600).is_err());
        assert!(check_guide_request(1, 0, 14 * 3600).is_err());
        assert!(check_guide_request(1, 100, 100).is_err());
    }

    /// A one-shot loopback HTTP server answering with `head` then `body`.
    async fn serve(head: String, body: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}/xmltv.xml")
    }

    async fn fetch(url: &str) -> reqwest::Response {
        reqwest::get(url).await.unwrap()
    }

    #[tokio::test]
    async fn feeds_over_the_cap_are_refused_while_downloading() {
        use std::io::Read;
        // Under the cap: spooled whole.
        let body = vec![b'x'; 1_000];
        let url = serve(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ),
            body.clone(),
        )
        .await;
        let mut file = spool_capped(fetch(&url).await, 2_000).await.unwrap();
        let mut spooled = Vec::new();
        file.read_to_end(&mut spooled).unwrap();
        assert_eq!(spooled, body);

        // A declared length over the cap: refused before reading the body.
        let url = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n".into(),
            vec![b'x'; 5_000],
        )
        .await;
        assert!(spool_capped(fetch(&url).await, 2_000).await.is_err());

        // No declared length: refused once the stream passes the cap.
        let chunk = vec![b'x'; 1_500];
        let mut chunked = Vec::new();
        for _ in 0..4 {
            chunked.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            chunked.extend_from_slice(&chunk);
            chunked.extend_from_slice(b"\r\n");
        }
        chunked.extend_from_slice(b"0\r\n\r\n");
        let url = serve(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".into(),
            chunked,
        )
        .await;
        let refused = spool_capped(fetch(&url).await, 2_000).await.unwrap_err();
        assert!(refused.contains("larger than"), "{refused}");
    }
}
