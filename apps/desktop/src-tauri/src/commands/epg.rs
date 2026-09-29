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
    ChannelNowAndNext, EpgFeedStatus, EpgSourcesRepository, NowAndNext, ProgramsRepository,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};

use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

const REFRESH_BATCH: usize = 500;
/// Parsed batches in flight between the parser thread and the inserter.
const BATCHES_IN_FLIGHT: usize = 4;
/// XMLTV feeds larger than this are refused (full multi-country guides run to
/// a few hundred MB; anything bigger is a mistake or hostile).
pub(crate) const MAX_EPG_BYTES: u64 = 512 * 1024 * 1024;

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
    let pool = require_profile_pool(&profile_id, &state).await?;
    ProgramsRepository::new(&pool)
        .range(&tvg_ids, from_utc, to_utc, source_id)
        .await
        .map_err(Into::into)
}

/// Manually register an XMLTV URL for a source. Use this to wire up community
/// EPG feeds (iptv-org, epg.pw, EPGTalk, etc.) for sources that don't ship
/// their own EPG, or to supplement provider EPG with extra channel coverage.
/// The feed is refreshed straight away (a queued EPG job).
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
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .epg_refresh(source_id)
        .await
        .map_err(ApiError::Internal)
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

enum ParsedChunk {
    Programs(Vec<Program>),
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
        let mut programs = nova_epg::parse(std::io::BufReader::new(feed));
        let mut batch = Vec::with_capacity(REFRESH_BATCH);
        for p in programs.by_ref() {
            match p {
                Ok(p) => {
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
                // A broken element shouldn't discard the entire feed.
                Err(e) => tracing::warn!(error = %e, "EPG parse: skipped malformed programme"),
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
                None => return Err("EPG parser stopped unexpectedly".to_string()),
            }
        };
        // An empty body, an HTML error page or a compressed feed parses to
        // nothing: keep the guide the feed had.
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
