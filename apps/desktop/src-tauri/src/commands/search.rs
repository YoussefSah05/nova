//! Phase 8 — transcript read + dialogue search commands.
//!
//! Both resolve the persisted transcript for the open title the same way the
//! caption/translation sessions do (stable `content_key` + audio-track slot),
//! then read its lines or run an FTS5 search scoped to it. Empty results when
//! nothing has been transcribed yet — the panel shows a friendly empty state.

use std::collections::HashMap;
use std::sync::Arc;

use nova_core::{ContentSort, VodItem};
use nova_enrich::Embedder;
use nova_storage::{
    EmbeddingRepository, PlaybackRepository, SearchHit, SearchRepository, TranscriptRepository,
    VodRepository,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};

use crate::caption_session::content_key;
use crate::commands::enrich::{embeddings_dir, marker, MODEL_ID};
use crate::ipc::ApiError;
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
async fn semantic_embedder(
    app: &AppHandle,
    state: &State<'_, AppState>,
) -> Option<Arc<dyn Embedder>> {
    let dir = embeddings_dir(app).ok()?;
    if !marker(&dir).exists() {
        return None;
    }
    state.semantic_embedder(&dir).await.ok()
}

/// Embed a query string off the async runtime (ONNX inference is blocking).
async fn embed_query_vec(embedder: Arc<dyn Embedder>, query: String) -> Result<Vec<f32>, ApiError> {
    tokio::task::spawn_blocking(move || embedder.embed_query(&query))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Dedupe ids preserving first-seen order — turns FTS hits (which may repeat a
/// segment for a source + a translation match) into one ranked id list for RRF.
fn dedup_preserve_order(ids: impl IntoIterator<Item = i64>) -> Vec<i64> {
    let mut seen = std::collections::HashSet::new();
    ids.into_iter().filter(|id| seen.insert(*id)).collect()
}

/// Lazily embed any not-yet-embedded segments of a transcript (best-effort: the
/// model is already installed when this is called). Cached thereafter.
async fn ensure_segments_embedded(
    pool: &SqlitePool,
    transcript_id: i64,
    embedder: &Arc<dyn Embedder>,
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
    let emb = embedder.clone();
    let vectors = tokio::task::spawn_blocking(move || emb.embed(&texts))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let rows: Vec<(i64, Vec<f32>)> = ids.into_iter().zip(vectors).collect();
    repo.upsert_many("segment", MODEL_ID, &rows)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(())
}

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
            nova_translate::model::MODEL_ID,
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

    // Semantic leg (optional). Lexical-only when the model isn't installed.
    let sem_ids: Vec<i64> = match semantic_embedder(&app, &state).await {
        Some(embedder) => {
            ensure_segments_embedded(&pool, transcript_id, &embedder).await?;
            let vectors = EmbeddingRepository::new(&pool)
                .load_segments(transcript_id, MODEL_ID)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if vectors.is_empty() {
                Vec::new()
            } else {
                let qvec = embed_query_vec(embedder, query.clone()).await?;
                nova_search::top_k(&qvec, &vectors, 50)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect()
            }
        }
        None => Vec::new(),
    };

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
    let sem_ids: Vec<i64> = match semantic_embedder(&app, &state).await {
        Some(embedder) => {
            let count = EmbeddingRepository::new(&pool)
                .count_catalog(source_id, "movie", MODEL_ID)
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            if count == 0 {
                Vec::new()
            } else {
                let pool2 = pool.clone();
                let vectors = state
                    .catalog_vectors(source_id, "movie", count, || async move {
                        EmbeddingRepository::new(&pool2)
                            .load_catalog(source_id, "movie", MODEL_ID)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(ApiError::Internal)?;
                let qvec = embed_query_vec(embedder, q.clone()).await?;
                nova_search::top_k(&qvec, &vectors, (limit as usize).max(60))
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect()
            }
        }
        None => Vec::new(),
    };

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
