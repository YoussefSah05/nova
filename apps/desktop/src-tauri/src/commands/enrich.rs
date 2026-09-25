//! Phase B — catalog enrichment commands. On-device embeddings de-duplicate
//! variant entries (e.g. `HBO` / `HBO ᴴᴰ` / `HBO FHD`) and fill missing genres.
//! The model downloads once via the UI (like whisper); enrichment runs as a
//! background task emitting `catalog:enrich:progress` and is idempotent
//! (`enrich_status` gates already-processed rows; a refresh re-imports + re-runs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nova_enrich::{
    best_scored, cluster, parse_genres, parse_search, DedupItem, EnrichError, FastEmbedder,
    GenreClassifier, TmdbClient, TmdbMatch, MIN_CONFIDENCE, WEAK_CONFIDENCE,
};
use nova_storage::{
    ChannelRepository, EmbeddingRepository, EnrichUpdate, ProfileSettingsRepository,
    SeriesRepository, SeriesTmdbQuery, SeriesTmdbWrite, TmdbCacheRepository, TmdbQuery, TmdbWrite,
    UnenrichedRow, VodRepository,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::transcription::app_data_dir;
use crate::ipc::ApiError;
use crate::model_runtime::{ModelRuntimeManager, RuntimeCancellation};
use crate::state::AppState;

use super::require_profile_pool;

pub(crate) use nova_enrich::MODEL_ID;

/// TMDB results flushed to the DB this often, so a long pass persists progress
/// incrementally (and re-runs resume) instead of losing everything on a hiccup.
const TMDB_WRITE_BATCH: usize = 200;
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
struct EnrichProgress {
    source_id: i64,
    stage: String,
    processed: usize,
    total: usize,
    done: bool,
    error: Option<String>,
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
    let runtime = state.model_runtime.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_enrichment(pool, source_id, runtime, &app2).await {
            let _ = app2.emit(
                "catalog:enrich:progress",
                EnrichProgress {
                    source_id,
                    stage: "failed".into(),
                    processed: 0,
                    total: 0,
                    done: true,
                    error: Some(format!("{e:?}")),
                },
            );
        }
    });
    Ok(())
}

async fn run_enrichment(
    pool: Arc<sqlx::SqlitePool>,
    source_id: i64,
    runtime: Arc<ModelRuntimeManager>,
    app: &AppHandle,
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
        // The first model load is slow (cold ONNX init); tell the UI before we block.
        emit("loading-model", 0, false);
        let lease = runtime
            .acquire(&model_root, MODEL_ID, &RuntimeCancellation::default())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;

        let genres: Vec<String> = nova_core::classify::genres()
            .iter()
            .map(|g| g.to_string())
            .collect();
        let label_vectors = lease
            .embed_documents(&genres, &RuntimeCancellation::default(), |_| {})
            .await
            .map_err(super::models::runtime_api_error)?;
        let classifier = GenreClassifier::new(genres.into_iter().zip(label_vectors).collect());
        let texts: Vec<String> = ch_rows
            .iter()
            .chain(&vod_rows)
            .map(|row| row.text.clone())
            .collect();
        let vectors = lease
            .embed_documents(&texts, &RuntimeCancellation::default(), |processed| {
                emit("embedding", processed, false)
            })
            .await
            .map_err(super::models::runtime_api_error)?;
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
        let texts: Vec<String> = series_work
            .iter()
            .map(|(title, overview)| match overview {
                Some(o) if !o.trim().is_empty() => format!("{title}. {o}"),
                _ => title.clone(),
            })
            .collect();
        let titles: Vec<String> = series_work.iter().map(|(t, _)| t.clone()).collect();
        let lease = runtime
            .acquire(&model_root, MODEL_ID, &RuntimeCancellation::default())
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let vectors = lease
            .embed_documents(&texts, &RuntimeCancellation::default(), |_| {})
            .await
            .map_err(super::models::runtime_api_error)?;
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
            run_tmdb(&pool, source_id, key, app).await?;
        }
    }

    emit("complete", total, true);
    Ok(())
}

/// TMDB enrichment for movies + series of a source. Each search is cached by key
/// so duplicates (and re-runs) don't re-hit the API; unmatched rows are recorded
/// so they're skipped next time. Network-bound, so it runs on the async runtime
/// (not `spawn_blocking`).
async fn run_tmdb(
    pool: &Arc<sqlx::SqlitePool>,
    source_id: i64,
    api_key: String,
    app: &AppHandle,
) -> Result<(), ApiError> {
    let client = TmdbClient::new(api_key).map_err(|e| ApiError::Internal(e.to_string()))?;
    let cache = TmdbCacheRepository::new(pool);

    let movie_genres = {
        let body = fetch_cached(&cache, "genres:movie", || client.genre_list("movie")).await?;
        parse_genres(&body).map_err(|e| ApiError::Internal(e.to_string()))?
    };
    let tv_genres = {
        let body = fetch_cached(&cache, "genres:tv", || client.genre_list("tv")).await?;
        parse_genres(&body).map_err(|e| ApiError::Internal(e.to_string()))?
    };

    // ── Movies ──
    let movies = VodRepository::new(pool).list_for_tmdb(source_id).await?;
    let series = SeriesRepository::new(pool).list_for_tmdb(source_id).await?;
    let tmdb_total = movies.len() + series.len();
    let mut done = 0usize;
    let emit = |processed: usize| {
        let _ = app.emit(
            "catalog:enrich:progress",
            EnrichProgress {
                source_id,
                stage: "matching".into(),
                processed,
                total: tmdb_total,
                done: false,
                error: None,
            },
        );
    };
    emit(0);

    // Per-item errors (a transient TMDB hiccup / rate-limit on one of thousands
    // of calls) must NOT abort the whole pass — skip the row (it stays unenriched
    // and is retried next run) and keep going. Matches are flushed in batches so
    // progress persists and a re-run resumes via `tmdb_status IS NULL`.
    let mut movie_writes: Vec<TmdbWrite> = Vec::new();
    for q in &movies {
        match match_movie(&client, &cache, q, &movie_genres).await {
            Ok(best) => movie_writes.push(to_movie_write(q.id, best)),
            Err(e) => {
                tracing::warn!(id = q.id, error = ?e, "tmdb movie match failed; retry next run")
            }
        }
        done += 1;
        if done.is_multiple_of(25) {
            emit(done);
        }
        if movie_writes.len() >= TMDB_WRITE_BATCH {
            VodRepository::new(pool).apply_tmdb(&movie_writes).await?;
            movie_writes.clear();
        }
    }
    if !movie_writes.is_empty() {
        VodRepository::new(pool).apply_tmdb(&movie_writes).await?;
    }

    // ── Series (search by title only; no year column) ──
    let mut series_writes: Vec<SeriesTmdbWrite> = Vec::new();
    for q in &series {
        match match_series(&client, &cache, q, &tv_genres).await {
            Ok(best) => series_writes.push(to_series_write(q.series_title.clone(), best)),
            Err(e) => {
                tracing::warn!(series = %q.series_title, error = ?e, "tmdb series match failed; retry next run")
            }
        }
        done += 1;
        if done.is_multiple_of(25) {
            emit(done);
        }
        if series_writes.len() >= TMDB_WRITE_BATCH {
            SeriesRepository::new(pool)
                .apply_tmdb(source_id, &series_writes)
                .await?;
            series_writes.clear();
        }
    }
    if !series_writes.is_empty() {
        SeriesRepository::new(pool)
            .apply_tmdb(source_id, &series_writes)
            .await?;
    }
    Ok(())
}

/// Match one movie via the cascade: movie+year → movie no-year → `/search/multi`,
/// stopping once confident. Network errors propagate so the caller can skip just
/// this row.
async fn match_movie(
    client: &TmdbClient,
    cache: &TmdbCacheRepository<'_>,
    q: &TmdbQuery,
    genres: &HashMap<i64, String>,
) -> Result<Option<(TmdbMatch, f32)>, ApiError> {
    let query = nova_core::clean::search_title(&q.title);
    let year = q.year.map(|y| y as i32);
    let qlc = query.to_lowercase();
    let mut best: Option<(TmdbMatch, f32)> = None;

    let key = format!("movie::{qlc}::{}", year.unwrap_or(0));
    let body = fetch_cached(cache, &key, || client.search_movie(&query, year)).await?;
    best = take_better(best, score_body(&body, &query, year, genres)?);

    if year.is_some() && !is_confident(&best) {
        let key = format!("movie::{qlc}::noyear");
        let body = fetch_cached(cache, &key, || client.search_movie(&query, None)).await?;
        best = take_better(best, score_body(&body, &query, year, genres)?);
    }
    if !is_confident(&best) {
        let key = format!("multi::{qlc}");
        let body = fetch_cached(cache, &key, || client.search_multi(&query)).await?;
        best = take_better(best, score_body(&body, &query, year, genres)?);
    }
    Ok(best)
}

/// Match one series: `/search/tv` → `/search/multi` fallback.
async fn match_series(
    client: &TmdbClient,
    cache: &TmdbCacheRepository<'_>,
    q: &SeriesTmdbQuery,
    genres: &HashMap<i64, String>,
) -> Result<Option<(TmdbMatch, f32)>, ApiError> {
    let query = nova_core::clean::search_series_title(&q.series_title);
    let qlc = query.to_lowercase();
    let mut best: Option<(TmdbMatch, f32)> = None;

    let key = format!("tv::{qlc}");
    let body = fetch_cached(cache, &key, || client.search_tv(&query)).await?;
    best = take_better(best, score_body(&body, &query, None, genres)?);

    if !is_confident(&best) {
        let key = format!("multi::{qlc}");
        let body = fetch_cached(cache, &key, || client.search_multi(&query)).await?;
        best = take_better(best, score_body(&body, &query, None, genres)?);
    }
    Ok(best)
}

/// Parse a search body and return its best-scored candidate.
fn score_body(
    body: &str,
    query: &str,
    year: Option<i32>,
    genres: &std::collections::HashMap<i64, String>,
) -> Result<Option<(TmdbMatch, f32)>, ApiError> {
    let items = parse_search(body).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(best_scored(query, year, &items, genres))
}

fn is_confident(b: &Option<(TmdbMatch, f32)>) -> bool {
    b.as_ref().is_some_and(|(_, s)| *s >= MIN_CONFIDENCE)
}

/// Keep whichever candidate scored higher.
fn take_better(
    cur: Option<(TmdbMatch, f32)>,
    cand: Option<(TmdbMatch, f32)>,
) -> Option<(TmdbMatch, f32)> {
    match (cur, cand) {
        (Some(a), Some(b)) => Some(if b.1 > a.1 { b } else { a }),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
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

/// Tier the scored candidate into a movie write:
/// - confident → canonical title + canonical genres + overview + art,
/// - weak (has a poster) → poster/backdrop only, keep the original title,
/// - otherwise → unmatched.
fn to_movie_write(id: i64, scored: Option<(TmdbMatch, f32)>) -> TmdbWrite {
    match scored {
        Some((m, s)) if s >= MIN_CONFIDENCE => {
            let genres = canonicalize_genres(&m.genres);
            TmdbWrite {
                id,
                canonical_title: Some(m.canonical_title),
                poster_url: m.poster_url,
                backdrop_url: m.backdrop_url,
                overview: m.overview,
                primary_genre: genres.first().cloned(),
                genres_csv: (!genres.is_empty()).then(|| genres.join(", ")),
                matched: true,
            }
        }
        Some((m, s)) if s >= WEAK_CONFIDENCE && m.poster_url.is_some() => TmdbWrite {
            id,
            poster_url: m.poster_url,
            backdrop_url: m.backdrop_url,
            matched: true,
            ..Default::default()
        },
        _ => TmdbWrite {
            id,
            matched: false,
            ..Default::default()
        },
    }
}

fn to_series_write(series_title: String, scored: Option<(TmdbMatch, f32)>) -> SeriesTmdbWrite {
    match scored {
        Some((m, s)) if s >= MIN_CONFIDENCE => {
            let genres = canonicalize_genres(&m.genres);
            SeriesTmdbWrite {
                series_title,
                canonical_title: Some(m.canonical_title),
                poster_url: m.poster_url,
                backdrop_url: m.backdrop_url,
                overview: m.overview,
                primary_genre: genres.first().cloned(),
                genres_csv: (!genres.is_empty()).then(|| genres.join(", ")),
                matched: true,
            }
        }
        Some((m, s)) if s >= WEAK_CONFIDENCE && m.poster_url.is_some() => SeriesTmdbWrite {
            series_title,
            poster_url: m.poster_url,
            backdrop_url: m.backdrop_url,
            matched: true,
            ..Default::default()
        },
        _ => SeriesTmdbWrite {
            series_title,
            matched: false,
            ..Default::default()
        },
    }
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
            let needs_genre = r.category.as_deref().is_none_or(|c| c.trim().is_empty());
            let category = if needs_genre {
                classifier
                    .classify(v, GENRE_THRESHOLD)
                    .map(|g| g.to_string())
            } else {
                None
            };
            EnrichUpdate {
                id: r.id,
                dedup_group: Some(rep.to_string()),
                category,
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
