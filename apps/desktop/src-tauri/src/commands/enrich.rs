//! Phase B — catalog enrichment commands. On-device embeddings de-duplicate
//! variant entries (e.g. `HBO` / `HBO ᴴᴰ` / `HBO FHD`) and fill missing genres.
//! The model downloads once via the UI (like whisper); enrichment runs as a
//! background task emitting `catalog:enrich:progress` and is idempotent
//! (`enrich_status` gates already-processed rows; a refresh re-imports + re-runs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nova_enrich::{
    cluster, decide, parse_details, parse_genres, parse_search, rank, retain_kind, verify_trusted,
    CandidateEvidence, Decision, DedupItem, EnrichError, FastEmbedder, GenreClassifier,
    MatchEvidence, TmdbClient, TmdbMatch, TmdbMediaKind, TrustedCheck, TrustedIdEvidence,
    UnresolvedReason, MATCHER_VERSION,
};
use nova_storage::{
    ChannelRepository, EmbeddingRepository, EnrichUpdate, ExternalMatchRepository,
    ExternalMatchWrite, MatchMethod, MatchedMetadata, MovieMatchWork, ProfileSettingsRepository,
    SeriesRepository, SeriesTmdbQuery, SeriesTmdbWrite, TmdbCacheRepository, UnenrichedRow,
    VodRepository,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::transcription::app_data_dir;
use crate::ipc::ApiError;
use crate::model_runtime::{ModelLease, ModelRuntimeManager, RuntimeCancellation};
use crate::resource_policy::{JobSlot, WorkClass};
use crate::state::AppState;

use super::require_profile_pool;

pub(crate) use nova_enrich::MODEL_ID;

/// Variant entries embed nearly identically; cluster above this cosine. Used only
/// as a *secondary* guard now — primary grouping is exact `dedup_key` equality.
const DEDUP_THRESHOLD: f32 = 0.92;
/// Only fill a genre when the nearest label clears this — avoids weak guesses.
const GENRE_THRESHOLD: f32 = 0.78;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichModelStatus {
    pub installed: bool,
    pub model_id: String,
    pub availability: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrichProgress {
    pub(crate) source_id: i64,
    pub(crate) stage: String,
    pub(crate) processed: usize,
    pub(crate) total: usize,
    pub(crate) done: bool,
    pub(crate) error: Option<String>,
}

pub(crate) fn embeddings_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("models").join("embeddings"))
}

/// Sentinel written after a successful download (fastembed's own cache layout is
/// opaque, so we track installation with a marker like the whisper flow).
pub(crate) fn marker(dir: &Path) -> PathBuf {
    dir.join(".installed")
}

#[tauri::command]
pub async fn get_enrich_model_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EnrichModelStatus, ApiError> {
    let observed = super::models::observe_internal_model(&app, &state.pool, MODEL_ID).await?;
    let available = state
        .model_runtime
        .is_available(&app_data_dir(&app)?, MODEL_ID)
        .await
        .map_err(super::models::runtime_api_error)?;
    Ok(EnrichModelStatus {
        installed: available,
        model_id: observed.descriptor.id.as_str().to_string(),
        availability: if available {
            "available"
        } else {
            observed.availability.as_str()
        }
        .to_string(),
    })
}

#[tauri::command]
pub async fn download_enrich_models(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), ApiError> {
    let dir = embeddings_dir(&app)?;
    state
        .model_runtime
        .mutate(MODEL_ID, async move {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            let load_dir = dir.clone();
            tauri::async_runtime::spawn_blocking(move || {
                FastEmbedder::load(&load_dir, true).map(|_| ())
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            tokio::fs::write(marker(&dir), b"1")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(super::models::runtime_api_error)?
}

#[tauri::command]
pub async fn start_enrichment(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let root = app_data_dir(&app)?;
    if !state
        .model_runtime
        .is_available(&root, MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        // No model installed. Auto-trigger after import is harmless, but emit a
        // terminal event so a *manual* run from Settings doesn't spin forever —
        // the panel turns this into a "download the model first" prompt.
        let _ = app.emit(
            "catalog:enrich:progress",
            EnrichProgress {
                source_id,
                stage: "no-model".into(),
                processed: 0,
                total: 0,
                done: true,
                error: None,
            },
        );
        return Ok(());
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    state
        .jobs_for_profile(&profile_id, pool, app)
        .await
        .enrich(source_id)
        .await
        .map_err(ApiError::Internal)
}

/// Cooperative resource checkpoint for the enrichment job. Releases the job
/// slot while playback suspends metadata work or higher-priority jobs wait,
/// and fails once the job is cancelled (for example, its source was deleted).
/// Never called while a database transaction is open.
async fn yield_point(
    slot: &mut JobSlot,
    cancellation: &RuntimeCancellation,
    on_wait: impl FnOnce(),
) -> Result<(), ApiError> {
    if slot.must_yield() && !cancellation.is_cancelled() {
        on_wait();
    }
    slot.checkpoint(cancellation)
        .await
        .map_err(|_| ApiError::Internal("enrichment cancelled".into()))
}

async fn acquire_embedder(
    runtime: &ModelRuntimeManager,
    model_root: &Path,
    cancellation: &RuntimeCancellation,
) -> Result<ModelLease, ApiError> {
    runtime
        .acquire_at(model_root, MODEL_ID, WorkClass::Background, cancellation)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Embed in bounded 256-document units with a job checkpoint between them. A
/// paused job drops its lease so the idle model stays evictable, then
/// reacquires it (cheaply, if still resident) when resumed.
#[allow(clippy::too_many_arguments)]
async fn embed_checkpointed(
    runtime: &ModelRuntimeManager,
    model_root: &Path,
    lease: &mut Option<ModelLease>,
    texts: &[String],
    slot: &mut JobSlot,
    cancellation: &RuntimeCancellation,
    on_wait: impl Fn(usize),
    on_progress: impl Fn(usize),
) -> Result<Vec<Vec<f32>>, ApiError> {
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(256) {
        if slot.must_yield() {
            *lease = None;
            let processed = vectors.len();
            yield_point(slot, cancellation, || on_wait(processed)).await?;
        }
        let current = match lease.take() {
            Some(lease) => lease,
            None => acquire_embedder(runtime, model_root, cancellation).await?,
        };
        let offset = vectors.len();
        let chunk_vectors = current
            .embed_documents_at(chunk, WorkClass::Background, cancellation, |processed| {
                on_progress(offset + processed)
            })
            .await
            .map_err(super::models::runtime_api_error)?;
        vectors.extend(chunk_vectors);
        *lease = Some(current);
    }
    Ok(vectors)
}

pub(crate) async fn run_enrichment(
    pool: Arc<sqlx::SqlitePool>,
    source_id: i64,
    runtime: Arc<ModelRuntimeManager>,
    app: &AppHandle,
    slot: &mut JobSlot,
    cancellation: &RuntimeCancellation,
) -> Result<(), ApiError> {
    let ch_rows = ChannelRepository::new(&pool)
        .list_unenriched(source_id)
        .await?;
    let vod_rows = VodRepository::new(&pool).list_unenriched(source_id).await?;
    let total = ch_rows.len() + vod_rows.len();
    let model_root = app_data_dir(app)?;

    let emit = |stage: &str, processed: usize, done: bool| {
        let _ = app.emit(
            "catalog:enrich:progress",
            EnrichProgress {
                source_id,
                stage: stage.to_string(),
                processed,
                total,
                done,
                error: None,
            },
        );
    };

    // Embedding/clustering only runs when there are *unenriched* rows. The TMDB
    // stage below is independent — it must still run on a re-trigger even when
    // embeddings are already done (`total == 0`), otherwise an opt-in TMDB pass
    // could never catch up on rows it hasn't matched yet.
    if total > 0 {
        yield_point(slot, cancellation, || emit("queued", 0, false)).await?;
        // The first model load is slow (cold ONNX init); tell the UI before we block.
        emit("loading-model", 0, false);
        let mut lease = Some(acquire_embedder(&runtime, &model_root, cancellation).await?);

        // Generic buckets ("Movies", "Series") describe nothing; never infer them.
        let genres: Vec<String> = nova_core::classify::specific_genres()
            .iter()
            .map(|g| g.to_string())
            .collect();
        let label_vectors = lease
            .as_ref()
            .ok_or(ApiError::Internal("model lease missing".into()))?
            .embed_documents_at(&genres, WorkClass::Background, cancellation, |_| {})
            .await
            .map_err(super::models::runtime_api_error)?;
        let classifier = GenreClassifier::new(genres.into_iter().zip(label_vectors).collect());
        let texts: Vec<String> = ch_rows
            .iter()
            .chain(&vod_rows)
            .map(|row| row.text.clone())
            .collect();
        let vectors = embed_checkpointed(
            &runtime,
            &model_root,
            &mut lease,
            &texts,
            slot,
            cancellation,
            |processed| emit("queued", processed, false),
            |processed| emit("embedding", processed, false),
        )
        .await?;
        drop(lease);

        // Clustering is CPU work, but does not own the model admission permit.
        type EmbedRows = Vec<(i64, Vec<f32>)>;
        let (ch_updates, vod_updates, ch_embed, vod_embed) =
            tauri::async_runtime::spawn_blocking(move || {
                let (ch_vecs, vod_vecs) = vectors.split_at(ch_rows.len());
                let ch = cluster_and_genre(&ch_rows, ch_vecs, &classifier);
                let vod = cluster_and_genre(&vod_rows, vod_vecs, &classifier);
                let ch_embed: EmbedRows = ch_rows
                    .iter()
                    .zip(ch_vecs)
                    .map(|(r, v)| (r.id, v.clone()))
                    .collect();
                let vod_embed: EmbedRows = vod_rows
                    .iter()
                    .zip(vod_vecs)
                    .map(|(r, v)| (r.id, v.clone()))
                    .collect();
                (ch, vod, ch_embed, vod_embed)
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        ChannelRepository::new(&pool)
            .apply_enrichment(&ch_updates)
            .await?;
        VodRepository::new(&pool)
            .apply_enrichment(&vod_updates)
            .await?;

        // Persist the embeddings (same vectors we just clustered on) so semantic
        // search is a free byproduct. `live`/`movie` match the catalog_embeddings
        // content_kind convention.
        let embed_repo = EmbeddingRepository::new(&pool);
        embed_repo.upsert_many("live", MODEL_ID, &ch_embed).await?;
        embed_repo
            .upsert_many("movie", MODEL_ID, &vod_embed)
            .await?;
    }

    // Series embedding stage — series are title-keyed (not in the catalog sweep),
    // so they embed separately into series_embeddings. Idempotent: only titles
    // not yet embedded for this model are processed. Title + TMDB overview (when
    // present) gives the vector richer meaning than the bare title.
    let series_repo = SeriesRepository::new(&pool);
    let series_work = series_repo.list_for_embedding(source_id, MODEL_ID).await?;
    if !series_work.is_empty() {
        yield_point(slot, cancellation, || emit("queued", total, false)).await?;
        let texts: Vec<String> = series_work
            .iter()
            .map(|(title, overview)| match overview {
                Some(o) if !o.trim().is_empty() => format!("{title}. {o}"),
                _ => title.clone(),
            })
            .collect();
        let titles: Vec<String> = series_work.iter().map(|(t, _)| t.clone()).collect();
        let mut lease = None;
        let vectors = embed_checkpointed(
            &runtime,
            &model_root,
            &mut lease,
            &texts,
            slot,
            cancellation,
            |_| emit("queued", total, false),
            |_| {},
        )
        .await?;
        drop(lease);
        let rows: Vec<(String, Vec<f32>)> = titles.into_iter().zip(vectors).collect();
        series_repo
            .apply_embeddings(source_id, MODEL_ID, &rows)
            .await?;
    }

    // Phase C — opt-in TMDB pass (canonical titles, posters, backdrops, genres).
    // Offline-first: only runs when enabled *and* a key is set. Independent of the
    // embedding stage so a re-run can keep matching rows even after embeddings done.
    let settings = ProfileSettingsRepository::new(&pool).get().await?;
    if settings.tmdb_enabled {
        if let Some(key) = settings.tmdb_api_key.filter(|k| !k.trim().is_empty()) {
            yield_point(slot, cancellation, || emit("queued", total, false)).await?;
            run_tmdb(&pool, source_id, key, app, slot, cancellation).await?;
        }
    }

    emit("complete", total, true);
    Ok(())
}

/// Titles matched per committed batch: small, so results appear progressively
/// and an interrupted pass loses at most one batch.
const TMDB_BATCH: usize = 25;
/// Unresolved titles are retried after this long (TMDB's catalog grows, and a
/// provider may fix a title). A matcher version bump retries everything.
const RETRY_UNRESOLVED_SECS: i64 = 7 * 24 * 3600;

/// One TMDB request, identified by its response-cache key.
enum TmdbRequest {
    MovieDetails(i64),
    SearchMovie { query: String, year: Option<i32> },
    SearchTv(String),
    SearchMulti(String),
}

impl TmdbRequest {
    /// Stable cache key (unchanged from the Phase C keys for searches, so
    /// existing cached bodies are reused).
    fn cache_key(&self) -> String {
        match self {
            Self::MovieDetails(id) => format!("movie-details::{id}"),
            Self::SearchMovie { query, year } => match year {
                Some(y) => format!("movie::{}::{y}", query.to_lowercase()),
                None => format!("movie::{}::noyear", query.to_lowercase()),
            },
            Self::SearchTv(q) => format!("tv::{}", q.to_lowercase()),
            Self::SearchMulti(q) => format!("multi::{}", q.to_lowercase()),
        }
    }
}

/// Where TMDB bodies come from: the cached network client in the app, a fixture
/// map in tests. `Ok(None)` means TMDB answered 404.
trait TmdbFetch {
    async fn fetch(&self, request: &TmdbRequest) -> Result<Option<String>, ApiError>;
}

struct CachedTmdb<'a> {
    client: &'a TmdbClient,
    cache: &'a TmdbCacheRepository<'a>,
}

impl TmdbFetch for CachedTmdb<'_> {
    async fn fetch(&self, request: &TmdbRequest) -> Result<Option<String>, ApiError> {
        let key = request.cache_key();
        if let Some(body) = self.cache.get(&key).await? {
            // An empty cached body records a 404.
            return Ok((!body.is_empty()).then_some(body));
        }
        let fetched = match request {
            TmdbRequest::MovieDetails(id) => self.client.movie_details(*id).await,
            TmdbRequest::SearchMovie { query, year } => {
                self.client.search_movie(query, *year).await.map(Some)
            }
            TmdbRequest::SearchTv(q) => self.client.search_tv(q).await.map(Some),
            TmdbRequest::SearchMulti(q) => self.client.search_multi(q).await.map(Some),
        }
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        self.cache
            .put(&key, fetched.as_deref().unwrap_or(""))
            .await?;
        Ok(fetched)
    }
}

/// TMDB metadata for the movies (canonical entities) and series of a source.
/// Offline-first: only called when the user enabled TMDB and set a credential.
///
/// Progressive: work is ordered by importance (favourites/watchlist, then
/// in-progress, then recently added) and committed every [`TMDB_BATCH`] titles,
/// so the most relevant posters appear first and a restart resumes. A transient
/// error on one title skips it (it is retried on the next run) without failing
/// the pass.
async fn run_tmdb(
    pool: &Arc<sqlx::SqlitePool>,
    source_id: i64,
    credential: String,
    app: &AppHandle,
    slot: &mut JobSlot,
    cancellation: &RuntimeCancellation,
) -> Result<(), ApiError> {
    let client = TmdbClient::new(credential).map_err(|e| ApiError::Internal(e.to_string()))?;
    let cache = TmdbCacheRepository::new(pool);
    let tmdb = CachedTmdb {
        client: &client,
        cache: &cache,
    };

    let movie_genres = {
        let body = fetch_cached(&cache, "genres:movie", || client.genre_list("movie")).await?;
        parse_genres(&body).map_err(|e| ApiError::Internal(e.to_string()))?
    };
    let tv_genres = {
        let body = fetch_cached(&cache, "genres:tv", || client.genre_list("tv")).await?;
        parse_genres(&body).map_err(|e| ApiError::Internal(e.to_string()))?
    };

    let retry_before = nova_core::now_ts() - RETRY_UNRESOLVED_SECS;
    let matches = ExternalMatchRepository::new(pool);
    let movies = matches
        .movie_work(source_id, MATCHER_VERSION, retry_before)
        .await?;
    let series = SeriesRepository::new(pool)
        .list_for_tmdb(source_id, MATCHER_VERSION, retry_before)
        .await?;
    let tmdb_total = movies.len() + series.len();
    let mut done = 0usize;
    let emit_stage = |stage: &str, processed: usize| {
        let _ = app.emit(
            "catalog:enrich:progress",
            EnrichProgress {
                source_id,
                stage: stage.into(),
                processed,
                total: tmdb_total,
                done: false,
                error: None,
            },
        );
    };
    emit_stage("matching", 0);

    for batch in movies.chunks(TMDB_BATCH) {
        let mut writes = Vec::with_capacity(batch.len());
        for work in batch {
            yield_point(slot, cancellation, || emit_stage("queued", done)).await?;
            match match_movie(&tmdb, work, &movie_genres).await {
                Ok(write) => writes.push(write),
                Err(e) => tracing::warn!(
                    entity = work.entity_id.as_str(),
                    error = ?e,
                    "tmdb movie match failed; retried next run"
                ),
            }
            done += 1;
        }
        matches.apply_movie_matches(&writes).await?;
        // Emitted after the commit: the UI may refresh and show these titles.
        emit_stage("matching", done);
    }

    for batch in series.chunks(TMDB_BATCH) {
        let mut writes = Vec::with_capacity(batch.len());
        for q in batch {
            yield_point(slot, cancellation, || emit_stage("queued", done)).await?;
            match match_series(&tmdb, q, &tv_genres).await {
                Ok(write) => writes.push(write),
                Err(e) => tracing::warn!(error = ?e, "tmdb series match failed; retried next run"),
            }
            done += 1;
        }
        SeriesRepository::new(pool)
            .apply_tmdb(source_id, &writes)
            .await?;
        emit_stage("matching", done);
    }
    Ok(())
}

fn parse_err(e: EnrichError) -> ApiError {
    ApiError::Internal(e.to_string())
}

fn evidence_json(evidence: &MatchEvidence) -> String {
    serde_json::to_string(evidence).unwrap_or_else(|_| "{}".into())
}

fn outcome_name(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::NoCandidates => "no_candidates",
        UnresolvedReason::LowConfidence => "low_confidence",
        UnresolvedReason::Ambiguous => "ambiguous",
    }
}

fn matched_metadata(m: TmdbMatch) -> MatchedMetadata {
    MatchedMetadata {
        external_id: m.tmdb_id.to_string(),
        canonical_title: m.canonical_title,
        poster_url: m.poster_url,
        backdrop_url: m.backdrop_url,
        overview: m.overview,
        genres: canonicalize_genres(&m.genres),
    }
}

/// Search, accumulating candidates across the cascade until one decides.
async fn search_cascade(
    tmdb: &impl TmdbFetch,
    steps: Vec<TmdbRequest>,
    kind: TmdbMediaKind,
    query: &str,
    year: Option<i32>,
    evidence: &mut MatchEvidence,
) -> Result<Vec<nova_enrich::Scored>, ApiError> {
    let mut items = Vec::new();
    let mut ranked = Vec::new();
    for step in steps {
        evidence.requests.push(step.cache_key());
        if let Some(body) = tmdb.fetch(&step).await? {
            items.extend(retain_kind(parse_search(&body).map_err(parse_err)?, kind));
        }
        ranked = rank(query, year, items.clone());
        if matches!(decide(&ranked), Decision::Matched(_)) {
            break;
        }
    }
    evidence.candidates = ranked
        .iter()
        .take(nova_enrich::tmdb::MAX_EVIDENCE_CANDIDATES)
        .map(CandidateEvidence::of)
        .collect();
    Ok(ranked)
}

/// Decide one movie entity: a corroborated provider ID first, otherwise a
/// search whose top candidate must be confident and unambiguous. Anything else
/// is recorded as unresolved, with the evidence that led there.
async fn match_movie(
    tmdb: &impl TmdbFetch,
    work: &MovieMatchWork,
    genres: &HashMap<i64, String>,
) -> Result<ExternalMatchWrite, ApiError> {
    let raw = work.source_title.as_deref().unwrap_or(&work.title);
    let query = nova_core::clean::search_title(raw);
    let titles = vec![work.title.clone(), raw.to_string()];
    let mut evidence = MatchEvidence::new(query.clone(), work.year);
    let write = |matched: Option<TmdbMatch>,
                 method: MatchMethod,
                 confidence_bp: Option<u16>,
                 evidence: &MatchEvidence| ExternalMatchWrite {
        entity_id: work.entity_id.clone(),
        matched: matched.map(matched_metadata),
        method,
        confidence_bp,
        matcher_version: MATCHER_VERSION,
        evidence_json: evidence_json(evidence),
    };

    // 1. Trusted ID. Several distinct provider IDs are contradictory evidence:
    //    rely on search instead.
    if let [tmdb_id] = work.trusted_tmdb_ids[..] {
        let request = TmdbRequest::MovieDetails(tmdb_id);
        evidence.requests.push(request.cache_key());
        let check = match tmdb.fetch(&request).await? {
            None => None,
            Some(body) => {
                let details = parse_details(&body).map_err(parse_err)?;
                let check = verify_trusted(&details, &titles, work.year);
                if check == TrustedCheck::Consistent {
                    evidence.trusted_id = Some(TrustedIdEvidence {
                        tmdb_id,
                        check: Some(check),
                    });
                    evidence.outcome = "matched".into();
                    let m = TmdbMatch::from_item(&details, &work.title, genres);
                    return Ok(write(
                        Some(m),
                        MatchMethod::TrustedId,
                        Some(10_000),
                        &evidence,
                    ));
                }
                Some(check)
            }
        };
        evidence.trusted_id = Some(TrustedIdEvidence { tmdb_id, check });
    }

    // 2. Search: movie+year → movie → multi (movies only), until decided.
    let mut steps = Vec::new();
    if work.year.is_some() {
        steps.push(TmdbRequest::SearchMovie {
            query: query.clone(),
            year: work.year,
        });
    }
    steps.push(TmdbRequest::SearchMovie {
        query: query.clone(),
        year: None,
    });
    steps.push(TmdbRequest::SearchMulti(query.clone()));
    let ranked = search_cascade(
        tmdb,
        steps,
        TmdbMediaKind::Movie,
        &query,
        work.year,
        &mut evidence,
    )
    .await?;
    Ok(match decide(&ranked) {
        Decision::Matched(i) => {
            evidence.outcome = "matched".into();
            let m = TmdbMatch::from_item(&ranked[i].item, &work.title, genres);
            let bp = nova_enrich::tmdb::score_bp(ranked[i].score);
            write(Some(m), MatchMethod::Search, Some(bp), &evidence)
        }
        Decision::Unresolved(reason) => {
            evidence.outcome = outcome_name(reason).into();
            let bp = ranked.first().map(|s| nova_enrich::tmdb::score_bp(s.score));
            write(None, MatchMethod::Search, bp, &evidence)
        }
    })
}

/// Decide one series (no trusted IDs yet): `/search/tv` → `/search/multi`.
async fn match_series(
    tmdb: &impl TmdbFetch,
    q: &SeriesTmdbQuery,
    genres: &HashMap<i64, String>,
) -> Result<SeriesTmdbWrite, ApiError> {
    let query = nova_core::clean::search_series_title(&q.series_title);
    let mut evidence = MatchEvidence::new(query.clone(), None);
    let steps = vec![
        TmdbRequest::SearchTv(query.clone()),
        TmdbRequest::SearchMulti(query.clone()),
    ];
    let ranked =
        search_cascade(tmdb, steps, TmdbMediaKind::Tv, &query, None, &mut evidence).await?;
    Ok(match decide(&ranked) {
        Decision::Matched(i) => {
            evidence.outcome = "matched".into();
            let m = TmdbMatch::from_item(&ranked[i].item, &q.series_title, genres);
            let genres = canonicalize_genres(&m.genres);
            SeriesTmdbWrite {
                series_title: q.series_title.clone(),
                canonical_title: Some(m.canonical_title),
                poster_url: m.poster_url,
                backdrop_url: m.backdrop_url,
                overview: m.overview,
                primary_genre: genres.first().cloned(),
                genres_csv: (!genres.is_empty()).then(|| genres.join(", ")),
                matched: true,
                method: "search",
                confidence_bp: Some(nova_enrich::tmdb::score_bp(ranked[i].score)),
                matcher_version: MATCHER_VERSION,
                evidence_json: evidence_json(&evidence),
            }
        }
        Decision::Unresolved(reason) => {
            evidence.outcome = outcome_name(reason).into();
            SeriesTmdbWrite {
                series_title: q.series_title.clone(),
                canonical_title: None,
                poster_url: None,
                backdrop_url: None,
                overview: None,
                primary_genre: None,
                genres_csv: None,
                matched: false,
                method: "search",
                confidence_bp: ranked.first().map(|s| nova_enrich::tmdb::score_bp(s.score)),
                matcher_version: MATCHER_VERSION,
                evidence_json: evidence_json(&evidence),
            }
        }
    })
}

/// Map TMDB genre names onto Nova's canonical taxonomy so chips/rails stay
/// consistent; unknown names pass through. Order-preserving, de-duplicated.
fn canonicalize_genres(genres: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in genres {
        let c = nova_core::classify::canonical_genre(g)
            .map(|s| s.to_string())
            .unwrap_or_else(|| g.clone());
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Read a cached TMDB body or fetch + store it.
async fn fetch_cached<F, Fut>(
    cache: &TmdbCacheRepository<'_>,
    key: &str,
    fetch: F,
) -> Result<String, ApiError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, EnrichError>>,
{
    if let Some(body) = cache.get(key).await? {
        return Ok(body);
    }
    let body = fetch()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    cache.put(key, &body).await?;
    Ok(body)
}

/// Cluster near-duplicates over already-computed vectors (embedding happens in
/// the chunked sweep in `run_enrichment`) and zero-shot a genre for rows lacking
/// one.
fn cluster_and_genre(
    rows: &[UnenrichedRow],
    vectors: &[Vec<f32>],
    classifier: &GenreClassifier,
) -> Vec<EnrichUpdate> {
    if rows.is_empty() {
        return Vec::new();
    }
    let items: Vec<DedupItem> = rows
        .iter()
        .zip(vectors)
        .map(|(r, v)| DedupItem {
            id: r.id,
            bucket: bucket_key(&r.text),
            key: nova_core::clean::dedup_key(&r.text),
            vector: v.clone(),
        })
        .collect();
    let clusters = cluster(&items, DEDUP_THRESHOLD);
    let mut group_of: HashMap<i64, i64> = HashMap::new();
    for c in &clusters {
        let rep = c.member_ids[0];
        for &id in &c.member_ids {
            group_of.insert(id, rep);
        }
    }

    rows.iter()
        .zip(vectors)
        .map(|(r, v)| {
            let rep = group_of.get(&r.id).copied().unwrap_or(r.id);
            // Inferred genres are the weakest evidence: only for rows with no
            // genre yet (TMDB and provider genres always win).
            let needs_genre = r.genre.as_deref().is_none_or(|g| g.trim().is_empty());
            let genre = if needs_genre {
                classifier
                    .classify(v, GENRE_THRESHOLD)
                    .map(|g| g.to_string())
            } else {
                None
            };
            EnrichUpdate {
                id: r.id,
                dedup_group: Some(rep.to_string()),
                genre,
            }
        })
        .collect()
}

/// Coarse bucket key (first alphanumeric token, lowercased) so only plausibly
/// related entries are compared — keeps clustering O(n) per bucket.
fn bucket_key(text: &str) -> String {
    text.split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tmdb_matching_tests {
    use super::*;
    use nova_core::catalog::EntityId;
    use nova_storage::MatchPriority;
    use std::sync::Mutex;

    /// Offline TMDB: bodies by cache key; a missing key is a 404. Records the
    /// keys requested, in order.
    #[derive(Default)]
    struct Fixture {
        bodies: HashMap<String, String>,
        requested: Mutex<Vec<String>>,
    }

    impl Fixture {
        fn with(mut self, key: &str, body: &str) -> Self {
            self.bodies.insert(key.into(), body.into());
            self
        }
        fn requested(&self) -> Vec<String> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl TmdbFetch for Fixture {
        async fn fetch(&self, request: &TmdbRequest) -> Result<Option<String>, ApiError> {
            let key = request.cache_key();
            self.requested.lock().unwrap().push(key.clone());
            Ok(self.bodies.get(&key).cloned())
        }
    }

    const GODFATHER: &str = r#"{"id": 238, "title": "The Godfather", "release_date": "1972-03-14",
        "poster_path": "/g.jpg", "overview": "Spanning the years...",
        "genres": [{"id": 18, "name": "Drama"}, {"id": 80, "name": "Crime"}]}"#;
    const EMPTY: &str = r#"{"results": []}"#;

    fn work(title: &str, year: Option<i32>, trusted: &[i64]) -> MovieMatchWork {
        MovieMatchWork {
            entity_id: EntityId::new("mov-1").unwrap(),
            title: title.into(),
            source_title: Some(format!("FR| {title} FHD")),
            year,
            trusted_tmdb_ids: trusted.to_vec(),
            priority: MatchPriority::Catalog,
        }
    }

    fn evidence(write: &ExternalMatchWrite) -> serde_json::Value {
        serde_json::from_str(&write.evidence_json).unwrap()
    }

    #[tokio::test]
    async fn a_corroborated_trusted_id_matches_without_searching() {
        let tmdb = Fixture::default().with("movie-details::238", GODFATHER);
        let w = match_movie(
            &tmdb,
            &work("Le Parrain", Some(1972), &[238]),
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(tmdb.requested(), vec!["movie-details::238"]);
        assert_eq!(w.method, MatchMethod::TrustedId);
        assert_eq!(w.confidence_bp, Some(10_000));
        let m = w.matched.as_ref().unwrap();
        assert_eq!(m.external_id, "238");
        assert_eq!(m.canonical_title, "The Godfather");
        assert_eq!(m.genres, vec!["Drama", "Crime"]);
        let e = evidence(&w);
        assert_eq!(e["trustedId"]["check"], "consistent");
        assert_eq!(e["outcome"], "matched");
        assert_eq!(w.entity_id.as_str(), "mov-1", "the Nova ID is untouched");
    }

    #[tokio::test]
    async fn a_contradicted_or_unknown_trusted_id_falls_back_to_search() {
        let search =
            r#"{"results": [{"id": 238, "title": "The Godfather", "release_date": "1972-03-14"}]}"#;
        // The provider ID points at a 1972 film, but the catalog says 1990.
        let tmdb = Fixture::default()
            .with("movie-details::238", GODFATHER)
            .with("movie::le parrain::1990", EMPTY)
            .with("movie::le parrain::noyear", EMPTY)
            .with("multi::le parrain", EMPTY);
        let w = match_movie(
            &tmdb,
            &work("Le Parrain", Some(1990), &[238]),
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(w.matched.is_none(), "a contradicted ID is never trusted");
        assert_eq!(evidence(&w)["trustedId"]["check"], "contradicted");
        assert_eq!(evidence(&w)["outcome"], "no_candidates");

        // Unknown to TMDB (404): search decides.
        let tmdb = Fixture::default().with("movie::the godfather::1972", search);
        let w = match_movie(
            &tmdb,
            &work("The Godfather", Some(1972), &[999]),
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert_eq!(w.method, MatchMethod::Search);
        assert_eq!(w.matched.as_ref().unwrap().external_id, "238");
        let e = evidence(&w);
        assert!(e["trustedId"]["check"].is_null());
        assert_eq!(
            tmdb.requested(),
            vec!["movie-details::999", "movie::the godfather::1972"],
            "the cascade stops at the first decisive search"
        );
    }

    #[tokio::test]
    async fn conflicting_trusted_ids_are_not_used() {
        let tmdb = Fixture::default();
        let w = match_movie(
            &tmdb,
            &work("Heat", Some(1995), &[949, 1234]),
            &HashMap::new(),
        )
        .await
        .unwrap();
        assert!(tmdb
            .requested()
            .iter()
            .all(|k| !k.starts_with("movie-details")));
        assert!(w.matched.is_none());
    }

    #[tokio::test]
    async fn low_confidence_and_ambiguous_results_stay_unresolved_with_evidence() {
        let weak = r#"{"results": [{"id": 5, "title": "Heat Wave Rising", "release_date": "2012-01-01", "poster_path": "/p.jpg"}]}"#;
        let tmdb = Fixture::default()
            .with("movie::heat::1995", weak)
            .with("movie::heat::noyear", weak)
            .with("multi::heat", weak);
        let w = match_movie(&tmdb, &work("Heat", Some(1995), &[]), &HashMap::new())
            .await
            .unwrap();
        assert!(w.matched.is_none(), "no poster-only guesses");
        let e = evidence(&w);
        assert_eq!(e["outcome"], "low_confidence");
        assert_eq!(e["candidates"][0]["tmdbId"], 5);
        assert_eq!(e["requests"].as_array().unwrap().len(), 3);

        let remakes = r#"{"results": [
            {"id": 111, "title": "Scarface", "release_date": "1983-12-09"},
            {"id": 112, "title": "Scarface", "release_date": "1932-04-09"}]}"#;
        let tmdb = Fixture::default()
            .with("movie::scarface::noyear", remakes)
            .with("multi::scarface", remakes);
        let w = match_movie(&tmdb, &work("Scarface", None, &[]), &HashMap::new())
            .await
            .unwrap();
        assert!(w.matched.is_none());
        assert_eq!(evidence(&w)["outcome"], "ambiguous");
    }

    #[tokio::test]
    async fn a_tv_result_is_never_recorded_as_a_movie_match() {
        let tv_only = r#"{"results": [{"id": 60622, "name": "Fargo", "media_type": "tv", "first_air_date": "2014-04-15"}]}"#;
        let tmdb = Fixture::default()
            .with("movie::fargo::noyear", EMPTY)
            .with("multi::fargo", tv_only);
        let w = match_movie(&tmdb, &work("Fargo", None, &[]), &HashMap::new())
            .await
            .unwrap();
        assert!(w.matched.is_none());
        assert_eq!(evidence(&w)["outcome"], "no_candidates");
    }

    #[tokio::test]
    async fn series_follow_the_same_rules() {
        let tv = r#"{"results": [{"id": 1396, "name": "Breaking Bad", "first_air_date": "2008-01-20", "poster_path": "/b.jpg"}]}"#;
        let tmdb = Fixture::default().with("tv::breaking bad", tv);
        let q = SeriesTmdbQuery {
            series_title: "Breaking Bad".into(),
        };
        let w = match_series(&tmdb, &q, &HashMap::new()).await.unwrap();
        assert!(w.matched);
        assert_eq!((w.method, w.matcher_version), ("search", MATCHER_VERSION));
        assert!(w.evidence_json.contains("\"outcome\":\"matched\""));

        let tmdb = Fixture::default();
        let q = SeriesTmdbQuery {
            series_title: "Unknown Show".into(),
        };
        let w = match_series(&tmdb, &q, &HashMap::new()).await.unwrap();
        assert!(!w.matched && w.poster_url.is_none() && w.canonical_title.is_none());
    }
}
