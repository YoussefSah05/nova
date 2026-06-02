//! Phase B — catalog enrichment commands. On-device embeddings de-duplicate
//! variant entries (e.g. `HBO` / `HBO ᴴᴰ` / `HBO FHD`) and fill missing genres.
//! The model downloads once via the UI (like whisper); enrichment runs as a
//! background task emitting `catalog:enrich:progress` and is idempotent
//! (`enrich_status` gates already-processed rows; a refresh re-imports + re-runs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nova_enrich::{
    best_scored, cluster, parse_genres, parse_search, DedupItem, Embedder, EnrichError,
    FastEmbedder, GenreClassifier, TmdbClient, TmdbMatch, MIN_CONFIDENCE, WEAK_CONFIDENCE,
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
/// Rows embedded per batch; small enough that progress advances visibly.
const EMBED_CHUNK: usize = 256;
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
    // The first model load is slow (cold ONNX init); tell the UI before we block.
    emit("loading-model", 0, false);

    // Embedding + clustering is CPU-heavy and synchronous — do it off the async
    // runtime. Embed in chunks and emit progress per chunk so the bar actually
    // moves (the whole-catalog single call is what made it look stuck at 0).
    let app_bg = app.clone();
    let (ch_updates, vod_updates) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(Vec<EnrichUpdate>, Vec<EnrichUpdate>), String> {
            let embedder = FastEmbedder::load(&cache_dir, false).map_err(|e| e.to_string())?;
            let genres = nova_core::classify::genres();
            let genre_strings: Vec<String> = genres.iter().map(|g| g.to_string()).collect();
            let label_vecs = embedder.embed(&genre_strings).map_err(|e| e.to_string())?;
            let labels: Vec<(String, Vec<f32>)> =
                genre_strings.into_iter().zip(label_vecs).collect();
            let classifier = GenreClassifier::new(labels);

            // Embed channels + movies in one 0..total sweep, then split the
            // vectors back out for per-type clustering.
            let ch_n = ch_rows.len();
            let mut texts: Vec<String> = Vec::with_capacity(total);
            texts.extend(ch_rows.iter().map(|r| r.text.clone()));
            texts.extend(vod_rows.iter().map(|r| r.text.clone()));

            let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(total);
            for batch in texts.chunks(EMBED_CHUNK) {
                vectors.extend(embedder.embed(batch).map_err(|e| e.to_string())?);
                let _ = app_bg.emit(
                    "catalog:enrich:progress",
                    EnrichProgress {
                        source_id,
                        stage: "embedding".into(),
                        processed: vectors.len(),
                        total,
                        done: false,
                        error: None,
                    },
                );
            }
            let (ch_vecs, vod_vecs) = vectors.split_at(ch_n);
            let ch = cluster_and_genre(&ch_rows, ch_vecs, &classifier);
            let vod = cluster_and_genre(&vod_rows, vod_vecs, &classifier);
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
        // Use a search-optimized query (strip prefixes/year/junk); year separate.
        let query = nova_core::clean::search_title(&q.title);
        let year = q.year.map(|y| y as i32);
        let qlc = query.to_lowercase();
        let mut best: Option<(TmdbMatch, f32)> = None;

        // 1) movie + year → 2) movie no-year → 3) multi. Stop once confident.
        let key = format!("movie::{qlc}::{}", year.unwrap_or(0));
        let body = fetch_cached(&cache, &key, || client.search_movie(&query, year)).await?;
        best = take_better(best, score_body(&body, &query, year, &movie_genres)?);

        if year.is_some() && !is_confident(&best) {
            let key = format!("movie::{qlc}::noyear");
            let body = fetch_cached(&cache, &key, || client.search_movie(&query, None)).await?;
            best = take_better(best, score_body(&body, &query, year, &movie_genres)?);
        }
        if !is_confident(&best) {
            let key = format!("multi::{qlc}");
            let body = fetch_cached(&cache, &key, || client.search_multi(&query)).await?;
            best = take_better(best, score_body(&body, &query, year, &movie_genres)?);
        }

        movie_writes.push(to_movie_write(q.id, best));
        done += 1;
        if done.is_multiple_of(25) {
            emit(done);
        }
    }
    VodRepository::new(pool).apply_tmdb(&movie_writes).await?;

    // ── Series (search by title only; no year column) ──
    let mut series_writes = Vec::with_capacity(series.len());
    for q in &series {
        let query = nova_core::clean::search_title(&q.series_title);
        let qlc = query.to_lowercase();
        let mut best: Option<(TmdbMatch, f32)> = None;

        let key = format!("tv::{qlc}");
        let body = fetch_cached(&cache, &key, || client.search_tv(&query)).await?;
        best = take_better(best, score_body(&body, &query, None, &tv_genres)?);

        if !is_confident(&best) {
            let key = format!("multi::{qlc}");
            let body = fetch_cached(&cache, &key, || client.search_multi(&query)).await?;
            best = take_better(best, score_body(&body, &query, None, &tv_genres)?);
        }

        series_writes.push(to_series_write(q.series_title.clone(), best));
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
