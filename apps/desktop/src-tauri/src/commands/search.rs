//! Phase 8 — transcript read + dialogue search commands.
//!
//! Both resolve the persisted transcript for the open title the same way the
//! caption/translation sessions do (stable `content_key` + audio-track slot),
//! then read its lines or run an FTS5 search scoped to it. Empty results when
//! nothing has been transcribed yet — the panel shows a friendly empty state.

use std::collections::HashMap;

use nova_core::catalog::media::CanonicalMediaSummary;
use nova_core::catalog::EntityId;
use nova_core::{ChannelGroup, ContentSort, SeriesSummary, VodItem};
use nova_search::rank::{
    fts_all_words, match_tier, merge_with_semantic, rank_lexical, MatchTier, SearchDoc,
};
use nova_storage::{
    CanonicalRepository, CatalogSearchRepository, ChannelRepository, EmbeddingRepository,
    PlaybackRepository, SearchHit, SearchRepository, SeriesRepository, TranscriptRepository,
    VodRepository,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::caption_session::content_key;
use crate::commands::enrich::MODEL_ID;
use crate::commands::transcription::app_data_dir;
use crate::ipc::ApiError;
use crate::model_runtime::{ModelLease, RuntimeCancellation};
use crate::resource_policy::WorkClass;
use crate::state::AppState;

use super::require_profile_pool;

/// One transcript line for the companion panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptLineDto {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    pub translation: Option<String>,
}

/// One dialogue-search match.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub segment_id: i64,
    pub start_sec: f64,
    pub end_sec: f64,
    pub text: String,
    /// `source` | `translation`
    pub kind: String,
}

/// Resolve the persisted transcript id for a media item + audio track, or `None`
/// when nothing has been transcribed yet.
async fn resolve_transcript_id(
    pool: &SqlitePool,
    media_kind: &str,
    media_id: i64,
    audio_track: i64,
) -> Result<Option<i64>, ApiError> {
    let resolved = PlaybackRepository::new(pool)
        .resolve_stream_url(media_kind, media_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let key = content_key(&resolved.upstream_url);
    let stored = TranscriptRepository::new(pool)
        .get_for_media_track(&key, media_kind, media_id, audio_track)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(stored.map(|t| t.id))
}

/// The shared embedder when the enrich model is installed; `None` otherwise so
/// every search degrades gracefully to lexical-only (no download, no spin).
async fn semantic_embedder(app: &AppHandle, state: &State<'_, AppState>) -> Option<ModelLease> {
    let root = app_data_dir(app).ok()?;
    state
        .model_runtime
        .acquire_at(
            &root,
            MODEL_ID,
            WorkClass::Interactive,
            &RuntimeCancellation::default(),
        )
        .await
        .ok()
}

/// Embed a query string off the async runtime (ONNX inference is blocking).
async fn embed_query_vec(lease: ModelLease, query: String) -> Result<Vec<f32>, ApiError> {
    lease
        .run_blocking_at(
            WorkClass::Interactive,
            &RuntimeCancellation::default(),
            move |lease| {
                lease
                    .embedder()
                    .ok_or_else(|| ApiError::Internal("wrong model backend".into()))?
                    .embed_query(&query)
                    .map_err(|e| ApiError::Internal(e.to_string()))
            },
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
}

/// Run an optional semantic leg. Any failure — inference, vector load, a
/// busy or offline model — yields no semantic hits, so the caller still
/// returns its lexical results: exact and FTS search must never depend on the
/// embedding model (bootstrap §22, §45).
async fn semantic_leg<T>(
    what: &'static str,
    leg: impl std::future::Future<Output = Result<Vec<T>, ApiError>>,
) -> Vec<T> {
    match leg.await {
        Ok(hits) => hits,
        Err(error) => {
            tracing::warn!(
                leg = what,
                ?error,
                "semantic search leg failed; lexical results only"
            );
            Vec::new()
        }
    }
}

/// Dedupe keys preserving first-seen order — turns hit lists (which may repeat a
/// key, e.g. a segment matched as both source + translation, or several channels
/// in one brand group) into one ranked list for RRF.
fn dedup_preserve_order<T: Clone + Eq + std::hash::Hash>(
    keys: impl IntoIterator<Item = T>,
) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    keys.into_iter()
        .filter(|k| seen.insert(k.clone()))
        .collect()
}

/// Lazily embed any not-yet-embedded segments of a transcript (best-effort: the
/// model is already installed when this is called). Cached thereafter.
async fn ensure_segments_embedded(
    pool: &SqlitePool,
    transcript_id: i64,
    embedder: &ModelLease,
) -> Result<(), ApiError> {
    let repo = EmbeddingRepository::new(pool);
    let missing = repo
        .segments_missing_embeddings(transcript_id, MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if missing.is_empty() {
        return Ok(());
    }
    let texts: Vec<String> = missing.iter().map(|(_, t)| t.clone()).collect();
    let ids: Vec<i64> = missing.iter().map(|(id, _)| *id).collect();
    let vectors = embedder
        .embed_documents_at(
            &texts,
            WorkClass::Interactive,
            &RuntimeCancellation::default(),
            |_| {},
        )
        .await
        .map_err(super::models::runtime_api_error)?;
    let rows: Vec<(i64, Vec<f32>)> = ids.into_iter().zip(vectors).collect();
    repo.upsert_many("segment", MODEL_ID, &rows)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

/// Translations cached by the removed NLLB engine stay readable and searchable;
/// nothing produces new ones until translation returns (roadmap Phase 7/10).
const LEGACY_TRANSLATION_MODEL: &str = "nllb-200-distilled-600M";

#[tauri::command]
pub async fn get_transcript(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    audio_track_index: Option<u32>,
    target_lang: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<TranscriptLineDto>, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let track = audio_track_index.unwrap_or(0) as i64;
    let Some(transcript_id) = resolve_transcript_id(&pool, &media_kind, media_id, track).await?
    else {
        return Ok(Vec::new());
    };
    let lines = TranscriptRepository::new(&pool)
        .transcript_with_translations(
            transcript_id,
            target_lang.as_deref().unwrap_or(""),
            LEGACY_TRANSLATION_MODEL,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(lines
        .into_iter()
        .map(|l| TranscriptLineDto {
            segment_id: l.segment_id,
            start_sec: l.start_sec,
            end_sec: l.end_sec,
            text: l.text,
            translation: l.translation,
        })
        .collect())
}

fn hit_to_dto(h: SearchHit) -> SearchHitDto {
    SearchHitDto {
        segment_id: h.segment_id,
        start_sec: h.start_sec,
        end_sec: h.end_sec,
        text: h.text,
        kind: h.kind,
    }
}

/// Hybrid dialogue search: the FTS5/BM25 lexical leg (exact words, source +
/// translation) fused with a semantic leg (cosine over per-segment embeddings)
/// so a paraphrase — even across languages — finds the spoken line. Falls back
/// to pure FTS when the embedding model isn't installed.
#[tauri::command]
pub async fn search_transcript(
    profile_id: String,
    media_kind: String,
    media_id: i64,
    audio_track_index: Option<u32>,
    query: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<SearchHitDto>, ApiError> {
    let match_expr = nova_search::fts_match(&query);
    if match_expr.is_empty() {
        return Ok(Vec::new());
    }
    let pool = require_profile_pool(&profile_id, &state).await?;
    let track = audio_track_index.unwrap_or(0) as i64;
    let Some(transcript_id) = resolve_transcript_id(&pool, &media_kind, media_id, track).await?
    else {
        return Ok(Vec::new());
    };

    let fts_hits = SearchRepository::new(&pool)
        .search_in_transcript(transcript_id, &match_expr, 200)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Semantic leg (optional). Lexical-only when the model isn't installed
    // or fails.
    let sem_ids = semantic_segment_ids(&app, &state, &pool, transcript_id, &query).await;

    if sem_ids.is_empty() {
        return Ok(fts_hits.into_iter().map(hit_to_dto).collect());
    }

    // Fuse at segment-id granularity. Keep the first (best-bm25) FTS row per
    // segment so a matched line shows its matched text/kind; semantic-only
    // segments are hydrated from their source text.
    let lexical_ids = dedup_preserve_order(fts_hits.iter().map(|h| h.segment_id));
    let mut fts_by_seg: HashMap<i64, SearchHit> = HashMap::new();
    for h in fts_hits {
        fts_by_seg.entry(h.segment_id).or_insert(h);
    }
    let seg_meta: HashMap<i64, (f64, f64, String)> = TranscriptRepository::new(&pool)
        .segments_with_ids(transcript_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .map(|s| (s.id, (s.start_sec, s.end_sec, s.text)))
        .collect();

    let fused = nova_search::rrf_fuse(&[&lexical_ids, &sem_ids], nova_search::RRF_K);
    let out: Vec<SearchHitDto> = fused
        .into_iter()
        .take(200)
        .filter_map(|(id, _)| {
            if let Some(h) = fts_by_seg.remove(&id) {
                Some(hit_to_dto(h))
            } else {
                seg_meta.get(&id).map(|(start, end, text)| SearchHitDto {
                    segment_id: id,
                    start_sec: *start,
                    end_sec: *end,
                    text: text.clone(),
                    kind: "source".to_string(),
                })
            }
        })
        .collect();
    Ok(out)
}

/// Transcript segments nearest to `query`, best first; embeds any segments
/// not yet embedded. Empty when the model isn't installed.
async fn semantic_segment_ids(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &SqlitePool,
    transcript_id: i64,
    query: &str,
) -> Vec<i64> {
    semantic_leg(
        "transcript",
        try_semantic_segment_ids(app, state, pool, transcript_id, query),
    )
    .await
}

async fn try_semantic_segment_ids(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &SqlitePool,
    transcript_id: i64,
    query: &str,
) -> Result<Vec<i64>, ApiError> {
    let Some(embedder) = semantic_embedder(app, state).await else {
        return Ok(Vec::new());
    };
    ensure_segments_embedded(pool, transcript_id, &embedder).await?;
    let vectors = EmbeddingRepository::new(pool)
        .load_segments(transcript_id, MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if vectors.is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embed_query_vec(embedder, query.to_string()).await?;
    Ok(nova_search::top_k(&qvec, &vectors, 50)
        .into_iter()
        .map(|(id, _)| id)
        .collect())
}

/// Hybrid catalog (movies) search: the LIKE lexical leg fused with a semantic
/// leg (cosine over the catalog embeddings enrichment persisted) so a
/// concept query — "space adventure" — surfaces thematically-matching titles,
/// not just literal substring matches. Lexical-only when no model/vectors.
/// Returns `VodItem`s (same shape as `list_vod`) so the browse grid renders them.
#[tauri::command]
pub async fn semantic_search_catalog(
    profile_id: String,
    source_id: i64,
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<VodItem>, ApiError> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(60).clamp(1, 200);
    let pool = require_profile_pool(&profile_id, &state).await?;
    let vod = VodRepository::new(&pool);

    // Lexical leg — reuse the existing LIKE path; fetch a few pages' worth so
    // fusion has candidates to work with.
    let lexical = vod
        .list(
            source_id,
            (limit * 3).max(90),
            0,
            None,
            Some(&q),
            false,
            false,
            ContentSort::Default,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lexical_ids = dedup_preserve_order(lexical.iter().map(|v| v.id));

    // Semantic leg (optional).
    let sem_ids =
        semantic_movie_rows(&app, &state, &pool, source_id, &q, (limit as usize).max(60)).await;

    let ordered: Vec<i64> = if sem_ids.is_empty() {
        lexical_ids
    } else {
        nova_search::rrf_fuse(&[&lexical_ids, &sem_ids], nova_search::RRF_K)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    let ordered: Vec<i64> = ordered.into_iter().take(limit as usize).collect();

    // Hydrate: reuse rows we already have from the lexical page; fetch the rest.
    let mut by_id: HashMap<i64, VodItem> = lexical.into_iter().map(|v| (v.id, v)).collect();
    let missing: Vec<i64> = ordered
        .iter()
        .copied()
        .filter(|id| !by_id.contains_key(id))
        .collect();
    for v in vod
        .list_by_ids(source_id, &missing)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        by_id.insert(v.id, v);
    }
    Ok(ordered
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect())
}

/// Provider movie rows nearest to `query` by the on-device embedding model,
/// best first. Empty when the model or the source's vectors are missing:
/// semantic search is an optional leg, never required.
async fn semantic_movie_rows(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Vec<i64> {
    semantic_leg(
        "catalog",
        try_semantic_movie_rows(app, state, pool, source_id, query, k),
    )
    .await
}

async fn try_semantic_movie_rows(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Result<Vec<i64>, ApiError> {
    let Some(embedder) = semantic_embedder(app, state).await else {
        return Ok(Vec::new());
    };
    let count = EmbeddingRepository::new(pool)
        .count_catalog(source_id, "movie", MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let pool2 = pool.clone();
    let vectors = state
        .catalog_vectors(|| async move {
            EmbeddingRepository::new(&pool2)
                .load_catalog(source_id, "movie", MODEL_ID)
                .await
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(ApiError::Internal)?;
    let qvec = embed_query_vec(embedder, query.to_string()).await?;
    Ok(nova_search::top_k(&qvec, &vectors, k)
        .into_iter()
        .map(|(id, _)| id)
        .collect())
}

/// Candidates fetched from the lexical index before ranking.
const MOVIE_SEARCH_CANDIDATES: i64 = 300;

/// Rank a movie search over canonical entities. Lexical candidates are
/// ordered by `nova_search::rank` (exact title first); `semantic` (entity IDs,
/// best first, possibly empty) only fills in below the title tiers. Pure given
/// its inputs, so ranking regressions are testable without a model.
pub(crate) async fn rank_movie_search(
    pool: &SqlitePool,
    source_id: i64,
    query: &str,
    limit: usize,
    semantic: &[EntityId],
) -> Result<Vec<CanonicalMediaSummary>, ApiError> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let candidates = CatalogSearchRepository::new(pool)
        .movie_candidates(source_id, &fts_all_words(q), MOVIE_SEARCH_CANDIDATES)
        .await?;
    let hits = candidates
        .iter()
        .enumerate()
        .map(|(rank, c)| {
            let doc = SearchDoc {
                titles: c.titles.lines().collect(),
                aliases: c.aliases.lines().collect(),
                metadata: c.metadata.lines().collect(),
            };
            // FTS matched every word as a prefix somewhere; keep it even when
            // the stricter tiering doesn't place it.
            let tier = match_tier(q, &doc).unwrap_or(MatchTier::Metadata);
            (c.entity_id.clone(), tier, rank)
        })
        .collect();
    let ordered: Vec<EntityId> = merge_with_semantic(&rank_lexical(hits), semantic)
        .into_iter()
        .take(limit)
        .collect();
    Ok(CanonicalRepository::new(pool)
        .movies_by_ids(source_id, &ordered)
        .await?)
}

/// Canonical movie search: one result per canonical entity (never per
/// provider row), exact titles first, then title words, aliases (provider and
/// parsed titles), and metadata (genres, keywords, people). The semantic leg
/// is used when the embedding model is installed and never outranks a title
/// match; without it, results are purely lexical.
#[tauri::command]
pub async fn search_movies(
    profile_id: String,
    source_id: i64,
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<CanonicalMediaSummary>, ApiError> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(60).clamp(1, 200) as usize;
    let pool = require_profile_pool(&profile_id, &state).await?;
    let semantic = semantic_movie_entities(&app, &state, &pool, source_id, &q, limit.max(60)).await;
    rank_movie_search(&pool, source_id, &q, limit, &semantic).await
}

/// Canonical entities of the provider rows nearest to `query`, best first.
async fn semantic_movie_entities(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Vec<EntityId> {
    semantic_leg(
        "movies",
        try_semantic_movie_entities(app, state, pool, source_id, query, k),
    )
    .await
}

async fn try_semantic_movie_entities(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Result<Vec<EntityId>, ApiError> {
    let rows = try_semantic_movie_rows(app, state, pool, source_id, query, k).await?;
    let canonical = CanonicalRepository::new(pool);
    let mut semantic: Vec<EntityId> = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(entity) = canonical.entity_for_vod(row).await? {
            if !semantic.contains(&entity) {
                semantic.push(entity);
            }
        }
    }
    Ok(semantic)
}

/// Hybrid live-channel search. Channel vectors are keyed by raw channel id, but
/// the live wall shows brand-family **groups**, so cosine hits are mapped to
/// their group key and fused (on the group key) with the lexical grouped search.
/// Returns `ChannelGroup`s (same shape as `list_channel_groups`) so the grid
/// renders unchanged. Lexical-only when no model/vectors.
#[tauri::command]
pub async fn semantic_search_channels(
    profile_id: String,
    source_id: i64,
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<ChannelGroup>, ApiError> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(60).clamp(1, 200);
    let pool = require_profile_pool(&profile_id, &state).await?;
    let ch = ChannelRepository::new(&pool);

    // Lexical leg — existing grouped LIKE search.
    let lexical = ch
        .list_groups(
            source_id,
            None,
            None,
            Some(&q),
            false,
            false,
            (limit * 3).max(90),
            0,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lexical_keys = dedup_preserve_order(lexical.iter().map(|g| g.group.clone()));

    // Semantic leg (optional) → map channel-id hits to their group keys.
    let sem_keys =
        semantic_channel_keys(&app, &state, &pool, source_id, &q, (limit as usize).max(60)).await;

    let ordered: Vec<String> = if sem_keys.is_empty() {
        lexical_keys
    } else {
        nova_search::rrf_fuse(&[&lexical_keys[..], &sem_keys[..]], nova_search::RRF_K)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    let ordered: Vec<String> = ordered.into_iter().take(limit as usize).collect();

    let mut by_key: HashMap<String, ChannelGroup> =
        lexical.into_iter().map(|g| (g.group.clone(), g)).collect();
    let missing: Vec<String> = ordered
        .iter()
        .filter(|k| !by_key.contains_key(*k))
        .cloned()
        .collect();
    for g in ch
        .groups_by_keys(source_id, &missing)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        by_key.insert(g.group.clone(), g);
    }
    Ok(ordered
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect())
}

/// Brand-group keys of the live channels nearest to `query`, best first.
async fn semantic_channel_keys(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Vec<String> {
    semantic_leg(
        "channels",
        try_semantic_channel_keys(app, state, pool, source_id, query, k),
    )
    .await
}

async fn try_semantic_channel_keys(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &std::sync::Arc<SqlitePool>,
    source_id: i64,
    query: &str,
    k: usize,
) -> Result<Vec<String>, ApiError> {
    let Some(embedder) = semantic_embedder(app, state).await else {
        return Ok(Vec::new());
    };
    let count = EmbeddingRepository::new(pool)
        .count_catalog(source_id, "live", MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let pool2 = pool.clone();
    let vectors = state
        .catalog_vectors(|| async move {
            EmbeddingRepository::new(&pool2)
                .load_catalog(source_id, "live", MODEL_ID)
                .await
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(ApiError::Internal)?;
    let qvec = embed_query_vec(embedder, query.to_string()).await?;
    let ids: Vec<i64> = nova_search::top_k(&qvec, &vectors, k)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let key_of: HashMap<i64, String> = ChannelRepository::new(pool)
        .group_keys_for_ids(source_id, &ids)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .into_iter()
        .collect();
    Ok(dedup_preserve_order(
        ids.into_iter().filter_map(|id| key_of.get(&id).cloned()),
    ))
}

/// Hybrid series search. Series are title-keyed, so vectors live in
/// `series_embeddings` and fusion is on the `series_title` string. Returns
/// `SeriesSummary`s (same shape as `list_series`). The series corpus is small,
/// so its vectors load per search (no AppState cache). Lexical-only when no
/// model/vectors.
#[tauri::command]
pub async fn semantic_search_series(
    profile_id: String,
    source_id: i64,
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<SeriesSummary>, ApiError> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.unwrap_or(60).clamp(1, 200);
    let pool = require_profile_pool(&profile_id, &state).await?;
    let series = SeriesRepository::new(&pool);

    // Lexical leg — existing LIKE series search.
    let lexical = series
        .list_series(
            source_id,
            None,
            Some(&q),
            false,
            false,
            ContentSort::Default,
            (limit * 3).max(90),
            0,
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let lexical_keys = dedup_preserve_order(lexical.iter().map(|s| s.series_title.clone()));

    // Semantic leg (optional).
    let sem_keys =
        semantic_series_titles(&app, &state, &pool, source_id, &q, (limit as usize).max(60)).await;

    let ordered: Vec<String> = if sem_keys.is_empty() {
        lexical_keys
    } else {
        nova_search::rrf_fuse(&[&lexical_keys[..], &sem_keys[..]], nova_search::RRF_K)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    let ordered: Vec<String> = ordered.into_iter().take(limit as usize).collect();

    let mut by_title: HashMap<String, SeriesSummary> = lexical
        .into_iter()
        .map(|s| (s.series_title.clone(), s))
        .collect();
    let missing: Vec<String> = ordered
        .iter()
        .filter(|t| !by_title.contains_key(*t))
        .cloned()
        .collect();
    for s in series
        .list_by_titles(source_id, &missing)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        by_title.insert(s.series_title.clone(), s);
    }
    let fused: Vec<SeriesSummary> = ordered
        .into_iter()
        .filter_map(|t| by_title.remove(&t))
        .collect();
    Ok(exact_titles_first(&q, fused))
}

/// Series titles nearest to `query`, best first.
async fn semantic_series_titles(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &SqlitePool,
    source_id: i64,
    query: &str,
    k: usize,
) -> Vec<String> {
    semantic_leg(
        "series",
        try_semantic_series_titles(app, state, pool, source_id, query, k),
    )
    .await
}

async fn try_semantic_series_titles(
    app: &AppHandle,
    state: &State<'_, AppState>,
    pool: &SqlitePool,
    source_id: i64,
    query: &str,
    k: usize,
) -> Result<Vec<String>, ApiError> {
    let Some(embedder) = semantic_embedder(app, state).await else {
        return Ok(Vec::new());
    };
    let vectors = SeriesRepository::new(pool)
        .load_embeddings(source_id, MODEL_ID)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if vectors.is_empty() {
        return Ok(Vec::new());
    }
    let qvec = embed_query_vec(embedder, query.to_string()).await?;
    Ok(nova_search::top_k(&qvec, &vectors, k)
        .into_iter()
        .map(|(title, _)| title)
        .collect())
}

/// Series have no canonical search index yet: after fusion, move title-tier
/// matches (exact, prefix, all title words) to the front in tier order, so an
/// exact title is never outranked by a semantic neighbour.
fn exact_titles_first(query: &str, results: Vec<SeriesSummary>) -> Vec<SeriesSummary> {
    let mut scored: Vec<(usize, Option<MatchTier>, SeriesSummary)> = results
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let mut titles = vec![s.display_title.as_str(), s.series_title.as_str()];
            titles.extend(s.canonical_title.as_deref());
            let doc = SearchDoc {
                titles,
                ..SearchDoc::default()
            };
            let tier = match_tier(query, &doc).filter(|t| t.is_title());
            (i, tier, s)
        })
        .collect();
    // Title tiers first (by tier), everything else after, each keeping order.
    scored.sort_by_key(|(i, tier, _)| (tier.map_or(1, |_| 0), *tier, *i));
    scored.into_iter().map(|(_, _, s)| s).collect()
}

#[cfg(test)]
mod canonical_search_tests {
    use super::*;
    use nova_core::SourceEvidence;
    use nova_storage::{
        init_profile_db, ExternalMatchRepository, ExternalMatchWrite, MatchMethod, MatchedMetadata,
        SourceRepository,
    };

    struct Db {
        _dir: tempfile::TempDir,
        pool: SqlitePool,
        a: i64,
        b: i64,
    }

    async fn db() -> Db {
        let dir = tempfile::tempdir().unwrap();
        let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        let sources = SourceRepository::new(&pool);
        let a = sources
            .create("xtream", "A", "http://a/get.php")
            .await
            .unwrap()
            .id;
        let b = sources
            .create("xtream", "B", "http://b/get.php")
            .await
            .unwrap()
            .id;
        Db {
            _dir: dir,
            pool,
            a,
            b,
        }
    }

    /// A provider movie row as the importer builds it (parsed title + year).
    fn movie(source: i64, raw: &str, n: u32) -> VodItem {
        let p = nova_core::clean::parse_source_title(raw, nova_core::clean::TitleKind::Movie);
        VodItem {
            id: 0,
            source_id: source,
            name: raw.into(),
            clean_title: Some(p.title),
            year: p.year.map(i64::from),
            logo_url: None,
            stream_url: format!("http://{source}/movie/u/p/{n}.mkv"),
            category: None,
            is_favorite: false,
            is_in_watchlist: false,
            added_at: None,
            dedup_group: None,
            canonical_title: None,
            poster_url: None,
            backdrop_url: None,
            overview: None,
            upstream_id: Some(format!("xtream:vod:{n}")),
            provider_tmdb_id: None,
            evidence: SourceEvidence::default(),
        }
    }

    /// The import pipeline's catalog steps: insert, compile, resolve, index.
    async fn import(db: &Db, source: i64, rows: &[VodItem]) {
        let mut tx = db.pool.begin().await.unwrap();
        VodRepository::insert_batch_tx(&mut tx, rows).await.unwrap();
        CanonicalRepository::compile_movies_tx(&mut tx, source)
            .await
            .unwrap();
        CanonicalRepository::resolve_movies_tx(&mut tx)
            .await
            .unwrap();
        CatalogSearchRepository::rebuild_movies_tx(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn search(db: &Db, source: i64, q: &str) -> Vec<String> {
        search_with(db, source, q, &[]).await
    }

    async fn search_with(db: &Db, source: i64, q: &str, semantic: &[EntityId]) -> Vec<String> {
        rank_movie_search(&db.pool, source, q, 50, semantic)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.title)
            .collect()
    }

    async fn entity(db: &Db, source: i64, title: &str) -> EntityId {
        rank_movie_search(&db.pool, source, title, 50, &[])
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.title == title)
            .unwrap()
            .entity_id
    }

    #[tokio::test]
    async fn exact_titles_rank_first_regardless_of_lexical_noise() {
        let db = db().await;
        import(
            &db,
            db.a,
            &[
                movie(db.a, "Heat Wave (2020)", 1),
                movie(db.a, "Dead Heat (1988)", 2),
                movie(db.a, "The Heat (2013)", 3),
                movie(db.a, "Heat (1995)", 4),
                movie(db.a, "Heat and Dust (1983)", 5),
            ],
        )
        .await;
        let results = search(&db, db.a, "heat").await;
        assert_eq!(results[0], "Heat", "exact title first: {results:?}");
        assert_eq!(results.len(), 5);
        // Title prefix beats a word in the middle of a title.
        let heat_pos = |t: &str| results.iter().position(|r| r == t).unwrap();
        assert!(heat_pos("Heat Wave") < heat_pos("Dead Heat"));
        assert_eq!(search(&db, db.a, "the heat").await[0], "The Heat");
        assert_eq!(
            search(&db, db.a, "HEAT").await[0],
            "Heat",
            "case-insensitive"
        );
    }

    #[tokio::test]
    async fn semantic_hits_fill_in_but_never_outrank_an_exact_title() {
        let db = db().await;
        import(
            &db,
            db.a,
            &[
                movie(db.a, "Heat (1995)", 1),
                movie(db.a, "Collateral (2004)", 2),
                movie(db.a, "Heat Wave (2020)", 3),
            ],
        )
        .await;
        let collateral = entity(&db, db.a, "Collateral").await;
        let wave = entity(&db, db.a, "Heat Wave").await;
        // The model ranks Collateral and Heat Wave above Heat.
        let results = search_with(&db, db.a, "heat", &[collateral, wave]).await;
        assert_eq!(&results[..2], &["Heat", "Heat Wave"]);
        assert_eq!(results[2], "Collateral", "semantic-only hit appended");
        // Without a model the same query is purely lexical.
        assert_eq!(search(&db, db.a, "heat").await, vec!["Heat", "Heat Wave"]);
    }

    #[tokio::test]
    async fn duplicate_provider_rows_return_one_canonical_entity() {
        let db = db().await;
        // Two copies in one source and one in another: one film.
        import(
            &db,
            db.a,
            &[
                movie(db.a, "FR | OPPENHEIMER [4K] MULTI (2023)", 1),
                movie(db.a, "Oppenheimer (2023) FHD", 2),
            ],
        )
        .await;
        import(&db, db.b, &[movie(db.b, "EN - Oppenheimer (2023)", 9)]).await;
        let a = rank_movie_search(&db.pool, db.a, "oppenheimer", 50, &[])
            .await
            .unwrap();
        let b = rank_movie_search(&db.pool, db.b, "oppenheimer", 50, &[])
            .await
            .unwrap();
        assert_eq!(a.len(), 1, "one card per entity, not per provider row");
        assert_eq!(b.len(), 1);
        assert_eq!(
            a[0].entity_id, b[0].entity_id,
            "same canonical entity in both sources"
        );
        // The provider's raw spelling still finds it (alias tier).
        assert_eq!(search(&db, db.a, "oppenheimer multi").await.len(), 1);
    }

    #[tokio::test]
    async fn titles_original_titles_people_keywords_and_genres_are_searchable() {
        let db = db().await;
        import(&db, db.a, &[movie(db.a, "FR | Le Parrain (1972)", 1)]).await;
        let parrain = entity(&db, db.a, "Le Parrain").await;
        ExternalMatchRepository::new(&db.pool)
            .apply_movie_matches(&[ExternalMatchWrite {
                entity_id: parrain.clone(),
                matched: Some(MatchedMetadata {
                    external_id: "238".into(),
                    canonical_title: "Le Parrain".into(),
                    original_title: Some("The Godfather".into()),
                    title_language: "fr-FR".into(),
                    poster_url: None,
                    backdrop_url: None,
                    overview: None,
                    genres: vec!["Crime".into(), "Drama".into()],
                    people: vec!["Francis Ford Coppola".into(), "Al Pacino".into()],
                    keywords: vec!["mafia".into()],
                }),
                method: MatchMethod::TrustedId,
                confidence_bp: Some(10_000),
                matcher_version: 3,
                evidence_json: "{}".into(),
            }])
            .await
            .unwrap();
        for query in [
            "le parrain",
            "the godfather",
            "godfather",
            "pacino",
            "coppola mafia",
            "crime",
        ] {
            assert_eq!(
                search(&db, db.a, query).await,
                vec!["Le Parrain"],
                "{query}"
            );
        }
        assert!(search(&db, db.a, "scarface").await.is_empty());
    }

    #[tokio::test]
    async fn merged_entities_are_found_once_under_their_survivor() {
        let db = db().await;
        import(&db, db.a, &[movie(db.a, "Heat (1995)", 1)]).await;
        import(&db, db.b, &[movie(db.b, "Heat (1995) 4K", 2)]).await;
        let active: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM canonical_entities WHERE state = 'active'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(active, 1, "resolution merged the copies");
        let indexed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM movie_search")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(indexed, 1, "no document for the merged-away entity");
        assert_eq!(search(&db, db.b, "heat").await, vec!["Heat"]);
    }

    #[tokio::test]
    async fn semantic_leg_passes_hits_through_and_swallows_failures() {
        let ok = semantic_leg("test", async { Ok::<_, ApiError>(vec![3, 1, 2]) }).await;
        assert_eq!(ok, vec![3, 1, 2]);
        for error in [
            ApiError::Internal("inference failed".into()),
            ApiError::Conflict("model busy".into()),
            ApiError::NotFound("vectors".into()),
        ] {
            let failed: Vec<i64> = semantic_leg("test", async { Err(error) }).await;
            assert!(failed.is_empty());
        }
    }

    #[tokio::test]
    async fn user_input_cannot_break_the_index_query() {
        let db = db().await;
        import(&db, db.a, &[movie(db.a, "Heat (1995)", 1)]).await;
        for hostile in [
            "\"",
            "heat\" OR \"x",
            "NEAR(heat",
            "*",
            "-heat",
            "heat)",
            ":",
        ] {
            let _ = rank_movie_search(&db.pool, db.a, hostile, 10, &[])
                .await
                .unwrap();
        }
        assert!(search(&db, db.a, "   ").await.is_empty());
    }
}
