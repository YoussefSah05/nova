//! Phase B — catalog enrichment commands. On-device embeddings de-duplicate
//! variant entries (e.g. `HBO` / `HBO ᴴᴰ` / `HBO FHD`) and fill missing genres.
//! The model downloads once via the UI (like whisper); enrichment runs as a
//! background task emitting `catalog:enrich:progress` and is idempotent
//! (`enrich_status` gates already-processed rows; a refresh re-imports + re-runs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nova_enrich::{
    best_match, cluster, parse_genres, parse_search, DedupItem, Embedder, EnrichError,
    FastEmbedder, GenreClassifier, TmdbClient, MIN_CONFIDENCE,
};
use nova_storage::{
    ChannelRepository, EnrichUpdate, ProfileSettingsRepository, SeriesRepository, SeriesTmdbWrite,
    TmdbCacheRepository, TmdbWrite, UnenrichedRow, VodRepository,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::transcription::app_data_dir;
use crate::ipc::ApiError;
use crate::state::AppState;

use super::require_profile_pool;

const MODEL_ID: &str = "multilingual-e5-small";
/// Variant entries embed nearly identically; cluster above this cosine.
const DEDUP_THRESHOLD: f32 = 0.92;
/// Only fill a genre when the nearest label clears this — avoids weak guesses.
const GENRE_THRESHOLD: f32 = 0.78;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichModelStatus {
    pub installed: bool,
    pub model_id: String,
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

fn embeddings_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    Ok(app_data_dir(app)?.join("models").join("embeddings"))
}

/// Sentinel written after a successful download (fastembed's own cache layout is
/// opaque, so we track installation with a marker like the whisper flow).
fn marker(dir: &Path) -> PathBuf {
    dir.join(".installed")
}

#[tauri::command]
pub async fn get_enrich_model_status(app: AppHandle) -> Result<EnrichModelStatus, ApiError> {
    let dir = embeddings_dir(&app)?;
    Ok(EnrichModelStatus {
        installed: marker(&dir).exists(),
        model_id: MODEL_ID.to_string(),
    })
}

#[tauri::command]
pub async fn download_enrich_models(app: AppHandle) -> Result<(), ApiError> {
    let dir = embeddings_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::Internal(e.to_string()))?;
    let load_dir = dir.clone();
    tauri::async_runtime::spawn_blocking(move || FastEmbedder::load(&load_dir, true).map(|_| ()))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    std::fs::write(marker(&dir), b"1").map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn start_enrichment(
    profile_id: String,
    source_id: i64,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), ApiError> {
    let dir = embeddings_dir(&app)?;
    if !marker(&dir).exists() {
        // No model → silently skip, so auto-triggering after import is harmless.
        // The Settings panel surfaces install state explicitly.
        return Ok(());
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_enrichment(pool, source_id, dir, &app2).await {
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
    cache_dir: PathBuf,
    app: &AppHandle,
) -> Result<(), ApiError> {
    let ch_rows = ChannelRepository::new(&pool)
        .list_unenriched(source_id)
        .await?;
    let vod_rows = VodRepository::new(&pool).list_unenriched(source_id).await?;
    let total = ch_rows.len() + vod_rows.len();

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

    if total == 0 {
        emit("complete", 0, true);
        return Ok(());
    }
    emit("embedding", 0, false);

    // Embedding + clustering is CPU-heavy and synchronous — do it off the async
    // runtime, then write results back.
    let (ch_updates, vod_updates) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Vec<EnrichUpdate>, Vec<EnrichUpdate>), String> {
            let embedder = FastEmbedder::load(&cache_dir, false).map_err(|e| e.to_string())?;
            let genres = nova_core::classify::genres();
            let genre_strings: Vec<String> = genres.iter().map(|g| g.to_string()).collect();
            let label_vecs = embedder.embed(&genre_strings).map_err(|e| e.to_string())?;
            let labels: Vec<(String, Vec<f32>)> =
                genre_strings.into_iter().zip(label_vecs).collect();
            let classifier = GenreClassifier::new(labels);
            let ch = enrich_rows(&embedder, &classifier, &ch_rows).map_err(|e| e.to_string())?;
            let vod = enrich_rows(&embedder, &classifier, &vod_rows).map_err(|e| e.to_string())?;
            Ok((ch, vod))
        },
    )
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(ApiError::Internal)?;

    ChannelRepository::new(&pool)
        .apply_enrichment(&ch_updates)
        .await?;
    VodRepository::new(&pool)
        .apply_enrichment(&vod_updates)
        .await?;

    // Phase C — opt-in TMDB pass (canonical titles, posters, backdrops, genres).
    // Offline-first: only runs when enabled *and* a key is set.
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

    let mut movie_writes = Vec::with_capacity(movies.len());
    for q in &movies {
        let key = format!("movie::{}::{}", q.title.to_lowercase(), q.year.unwrap_or(0));
        let body = fetch_cached(&cache, &key, || {
            client.search_movie(&q.title, q.year.map(|y| y as i32))
        })
        .await?;
        let items = parse_search(&body).map_err(|e| ApiError::Internal(e.to_string()))?;
        let m = best_match(
            &q.title,
            q.year.map(|y| y as i32),
            &items,
            &movie_genres,
            MIN_CONFIDENCE,
        );
        movie_writes.push(to_movie_write(q.id, m));
        done += 1;
        if done.is_multiple_of(25) {
            emit(done);
        }
    }
    VodRepository::new(pool).apply_tmdb(&movie_writes).await?;

    // ── Series (search by title only; no year column) ──
    let mut series_writes = Vec::with_capacity(series.len());
    for q in &series {
        let key = format!("tv::{}", q.series_title.to_lowercase());
        let body = fetch_cached(&cache, &key, || client.search_tv(&q.series_title)).await?;
        let items = parse_search(&body).map_err(|e| ApiError::Internal(e.to_string()))?;
        let m = best_match(&q.series_title, None, &items, &tv_genres, MIN_CONFIDENCE);
        series_writes.push(to_series_write(q.series_title.clone(), m));
        done += 1;
        if done.is_multiple_of(25) {
            emit(done);
        }
    }
    SeriesRepository::new(pool)
        .apply_tmdb(source_id, &series_writes)
        .await?;
    Ok(())
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

fn to_movie_write(id: i64, m: Option<nova_enrich::TmdbMatch>) -> TmdbWrite {
    match m {
        Some(m) => TmdbWrite {
            id,
            canonical_title: Some(m.canonical_title),
            poster_url: m.poster_url,
            backdrop_url: m.backdrop_url,
            overview: m.overview,
            primary_genre: m.genres.first().cloned(),
            genres_csv: (!m.genres.is_empty()).then(|| m.genres.join(", ")),
            matched: true,
        },
        None => TmdbWrite {
            id,
            matched: false,
            ..Default::default()
        },
    }
}

fn to_series_write(series_title: String, m: Option<nova_enrich::TmdbMatch>) -> SeriesTmdbWrite {
    match m {
        Some(m) => SeriesTmdbWrite {
            series_title,
            canonical_title: Some(m.canonical_title),
            poster_url: m.poster_url,
            backdrop_url: m.backdrop_url,
            overview: m.overview,
            genres_csv: (!m.genres.is_empty()).then(|| m.genres.join(", ")),
            matched: true,
        },
        None => SeriesTmdbWrite {
            series_title,
            matched: false,
            ..Default::default()
        },
    }
}

/// Embed a batch, cluster near-duplicates (within a cheap first-token bucket),
/// and zero-shot a genre for rows that lack one.
fn enrich_rows(
    embedder: &FastEmbedder,
    classifier: &GenreClassifier,
    rows: &[UnenrichedRow],
) -> Result<Vec<EnrichUpdate>, EnrichError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<String> = rows.iter().map(|r| r.text.clone()).collect();
    let vectors = embedder.embed(&texts)?;

    let items: Vec<DedupItem> = rows
        .iter()
        .zip(&vectors)
        .map(|(r, v)| DedupItem {
            id: r.id,
            bucket: bucket_key(&r.text),
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

    Ok(rows
        .iter()
        .zip(&vectors)
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
        .collect())
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
