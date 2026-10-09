//! Translate bounded committed dialogue, using the caption session's lifecycle.

use nova_core::intelligence::TranslateBatch;
use nova_core::model_manifest::{ManifestEntry, TranslationRoute};
use nova_storage::{SourceSegment, TranscriptRepository, TranslationRepository, TranslationRow};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, State};

use crate::caption_session::CaptionCue;
use crate::ipc::ApiError;
use crate::model_registry::bundled_manifest;
use crate::model_runtime::{ModelRuntimeManager, RuntimeCancellation};
use crate::resource_policy::WorkClass;
use crate::state::AppState;

const BATCH_SIZE: usize = 8;

fn language_base(tag: &str) -> Option<String> {
    let tag = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    match tag.as_str() {
        "en" | "eng" => Some("en".into()),
        "fr" | "fra" | "fre" => Some("fr".into()),
        "ar" | "ara" => Some("ar".into()),
        _ => None,
    }
}

/// The existing backend cache column is an opaque identity. A JSON value also
/// retains inspectable provenance without reinterpreting legacy cache rows.
fn cache_key(source: &str, target: &str, text: &str, route: &[&ManifestEntry]) -> String {
    serde_json::json!({
        "pipeline": "caption-translation-v1",
        "decoding": "marian-greedy-v1",
        "source": source, "target": target,
        "sourceHash": format!("{:x}", Sha256::digest(text.as_bytes())),
        "models": route.iter().map(|m| serde_json::json!({"id":m.id,"revision":m.revision})).collect::<Vec<_>>(),
    }).to_string()
}

fn validate_ids(ids: &[i64]) -> Result<(), ApiError> {
    if ids.is_empty()
        || ids.len() > BATCH_SIZE
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        return Err(ApiError::InvalidInput(
            "Provide one to eight distinct committed segments".into(),
        ));
    }
    Ok(())
}

fn validate_output(output: &[String], expected: usize) -> Result<(), ApiError> {
    if output.len() != expected
        || output
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 16_384)
    {
        return Err(ApiError::Internal(
            "Translation returned invalid caption text; source captions remain available".into(),
        ));
    }
    Ok(())
}

async fn translate_hop(
    runtime: &ModelRuntimeManager,
    root: &std::path::Path,
    entry: &ManifestEntry,
    source: &str,
    target: &str,
    texts: Vec<String>,
    cancel: &RuntimeCancellation,
) -> Result<Vec<String>, ApiError> {
    let source = source.to_string();
    let target = target.to_string();
    let count = texts.len();
    let lease = runtime.acquire_at(root, &entry.id, WorkClass::Realtime, cancel).await
        .map_err(|_| ApiError::Internal("Translation model unavailable; check Models & Storage. Source captions remain available".into()))?;
    let output = lease
        .run_blocking_at(WorkClass::Realtime, cancel, move |lease| {
            let translator = lease
                .translator()
                .ok_or(nova_core::intelligence::EngineError::LoadFailed)?;
            translator.translate(TranslateBatch {
                source: &source,
                target: &target,
                context: &[],
                segments: &texts,
            })
        })
        .await
        .map_err(|_| {
            ApiError::Internal("Translation stopped; source captions remain available".into())
        })?
        .map_err(|_| {
            ApiError::Internal("Translation failed; source captions remain available".into())
        })?;
    validate_output(&output, count)?;
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn translate_caption_segments(
    profile_id: String,
    session_id: String,
    transcript_id: i64,
    segment_ids: Vec<i64>,
    target_lang: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Vec<CaptionCue>, ApiError> {
    validate_ids(&segment_ids)?;
    let target = language_base(&target_lang)
        .ok_or_else(|| ApiError::InvalidInput("Choose English, French or Arabic".into()))?;
    let (cancel, _slot) = state
        .caption_sessions
        .translation_context(&session_id, &profile_id, transcript_id)
        .await
        .ok_or_else(|| {
            ApiError::Conflict("Caption session changed or translation is busy".into())
        })?;
    let pool = super::require_profile_pool(&profile_id, &state).await?;
    let transcripts = TranscriptRepository::new(&pool);
    let source = language_base(&transcripts.language(transcript_id).await?)
        .ok_or_else(|| ApiError::InvalidInput("Translation supports English, French and Arabic dialogue; source captions remain available".into()))?;
    let segments = transcripts
        .segments_by_ids(transcript_id, &segment_ids)
        .await?;
    if segments.len() != segment_ids.len() {
        return Err(ApiError::InvalidInput(
            "Segments do not belong to the active transcript".into(),
        ));
    }
    if cancel.is_cancelled() {
        return Err(ApiError::Conflict("Caption session stopped".into()));
    }
    if source == target {
        return Ok(segments
            .into_iter()
            .map(|s| cue(&s, s.text.clone()))
            .collect());
    }
    let manifest = bundled_manifest()
        .map_err(|_| ApiError::Internal("Translation configuration unavailable".into()))?;
    let route = manifest
        .translation_route(&source, &target)
        .ok_or_else(|| {
            ApiError::InvalidInput("No local translation route for these languages".into())
        })?;
    let entries = match route {
        TranslationRoute::Direct(entry) => vec![entry],
        TranslationRoute::Pivot(first, second) => vec![first, second],
    };
    translate_cached_batch(
        &pool,
        segments,
        &source,
        &target,
        &entries,
        &cancel,
        |texts| async {
            let root = super::transcription::app_data_dir(&app)?;
            match route {
                TranslationRoute::Direct(entry) => {
                    translate_hop(
                        &state.model_runtime,
                        &root,
                        entry,
                        &source,
                        &target,
                        texts,
                        &cancel,
                    )
                    .await
                }
                TranslationRoute::Pivot(first, second) => {
                    // Only one hop's lease/permit is held at a time.
                    let via = translate_hop(
                        &state.model_runtime,
                        &root,
                        first,
                        &source,
                        "en",
                        texts,
                        &cancel,
                    )
                    .await?;
                    translate_hop(
                        &state.model_runtime,
                        &root,
                        second,
                        "en",
                        &target,
                        via,
                        &cancel,
                    )
                    .await
                }
            }
        },
    )
    .await
}

/// Cache-first orchestration shared by the command and offline failure tests.
async fn translate_cached_batch<F, Fut>(
    pool: &sqlx::SqlitePool,
    segments: Vec<SourceSegment>,
    source: &str,
    target: &str,
    entries: &[&ManifestEntry],
    cancel: &RuntimeCancellation,
    infer: F,
) -> Result<Vec<CaptionCue>, ApiError>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, ApiError>>,
{
    let cache = TranslationRepository::new(pool);
    let mut output = Vec::with_capacity(segments.len());
    let mut missing = Vec::new();
    for segment in segments {
        if cancel.is_cancelled() {
            return Err(ApiError::Conflict("Caption session stopped".into()));
        }
        if segment.text.len() > 4096 || segment.text.trim().is_empty() {
            return Err(ApiError::InvalidInput(
                "Source segment is not valid caption text".into(),
            ));
        }
        let key = cache_key(source, target, &segment.text, entries);
        if let Some(text) = cache.cached_segment(segment.id, target, &key).await? {
            validate_output(std::slice::from_ref(&text), 1)?;
            output.push(cue(&segment, text));
        } else {
            missing.push((segment, key));
        }
    }
    if !missing.is_empty() {
        if cancel.is_cancelled() {
            return Err(ApiError::Conflict("Caption session stopped".into()));
        }
        let texts = infer(missing.iter().map(|(s, _)| s.text.clone()).collect()).await?;
        validate_output(&texts, missing.len())?;
        if cancel.is_cancelled() {
            return Err(ApiError::Conflict("Caption session stopped".into()));
        }
        let rows: Vec<_> = missing
            .iter()
            .zip(&texts)
            .map(|((s, key), text)| TranslationRow {
                segment_id: s.id,
                target_lang: target.to_string(),
                backend: key.clone(),
                text: text.clone(),
            })
            .collect();
        cache.insert_batch(&rows).await?;
        output.extend(missing.iter().zip(texts).map(|((s, _), text)| cue(s, text)));
    }
    if cancel.is_cancelled() {
        return Err(ApiError::Conflict("Caption session stopped".into()));
    }
    output.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.segment_id.cmp(&b.segment_id))
    });
    Ok(output)
}

fn cue(s: &SourceSegment, text: String) -> CaptionCue {
    CaptionCue {
        segment_id: s.id,
        start: s.start_sec,
        end: s.end_sec,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (tempfile::TempDir, sqlx::SqlitePool, Vec<SourceSegment>) {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_profile_db(dir.path().join("profile.db").to_str().unwrap())
            .await
            .unwrap();
        let transcripts = TranscriptRepository::new(&pool);
        let id = transcripts
            .get_or_create_transcript("fixture", "vod", 1, 0, "en", "m", "", "")
            .await
            .unwrap();
        let ids = transcripts
            .append_segments(
                id,
                &[nova_core::TranscriptSegment {
                    seq: 0,
                    start_sec: 4.0,
                    end_sec: 5.0,
                    text: "Hello".into(),
                    words: None,
                }],
            )
            .await
            .unwrap();
        let segments = transcripts.segments_by_ids(id, &ids).await.unwrap();
        (dir, pool, segments)
    }

    fn en_fr() -> &'static ManifestEntry {
        bundled_manifest()
            .unwrap()
            .models
            .iter()
            .find(|m| m.id == "opus-mt-en-fr")
            .unwrap()
    }

    #[tokio::test]
    async fn warm_cache_skips_inference_even_when_the_model_is_unavailable() {
        let (_dir, pool, segments) = fixture().await;
        let cancel = RuntimeCancellation::default();
        let cold = translate_cached_batch(
            &pool,
            segments.clone(),
            "en",
            "fr",
            &[en_fr()],
            &cancel,
            |texts| async move {
                assert_eq!(texts, ["Hello"]);
                Ok(vec!["Bonjour".into()])
            },
        )
        .await
        .unwrap();
        assert_eq!((cold[0].start, cold[0].end), (4.0, 5.0));
        let warm = translate_cached_batch(
            &pool,
            segments.clone(),
            "en",
            "fr",
            &[en_fr()],
            &cancel,
            |_| async { panic!("a warm cache must not acquire the unavailable model") },
        )
        .await
        .unwrap();
        assert_eq!(warm[0].text, "Bonjour");
        let mut next = en_fr().clone();
        next.revision.push_str("-next");
        let miss =
            translate_cached_batch(&pool, segments, "en", "fr", &[&next], &cancel, |_| async {
                Err(ApiError::Internal("offline model".into()))
            })
            .await;
        assert!(miss.is_err(), "new revision must not reuse previous text");
    }

    #[tokio::test]
    async fn cancelled_failed_or_malformed_inference_never_populates_the_cache() {
        let (_dir, pool, segments) = fixture().await;
        for result in [
            Ok(vec![]),
            Ok(vec![" ".into()]),
            Err(ApiError::Internal("failed".into())),
        ] {
            let cancel = RuntimeCancellation::default();
            assert!(translate_cached_batch(
                &pool,
                segments.clone(),
                "en",
                "fr",
                &[en_fr()],
                &cancel,
                |_| async { result }
            )
            .await
            .is_err());
            assert_eq!(TranslationRepository::new(&pool).count().await.unwrap(), 0);
        }
        let cancel = RuntimeCancellation::default();
        assert!(translate_cached_batch(
            &pool,
            segments,
            "en",
            "fr",
            &[en_fr()],
            &cancel,
            |_| async {
                cancel.cancel();
                Ok(vec!["Bonjour".into()])
            }
        )
        .await
        .is_err());
        assert_eq!(TranslationRepository::new(&pool).count().await.unwrap(), 0);
    }

    #[test]
    fn languages_normalize_track_tags_and_reject_unknowns() {
        assert_eq!(language_base("fr-CA"), Some("fr".into()));
        assert_eq!(language_base("ARA"), Some("ar".into()));
        assert_eq!(language_base("und"), None);
    }

    #[test]
    fn rejects_unbounded_or_duplicated_segment_requests() {
        for ids in [vec![], vec![0], vec![1, 1], (1..=9).collect()] {
            assert!(validate_ids(&ids).is_err());
        }
        assert!(validate_ids(&[1, 2]).is_ok());
    }

    #[test]
    fn cache_identity_changes_with_source_direction_revision_and_route() {
        let manifest = bundled_manifest().unwrap();
        let mut entry = manifest
            .models
            .iter()
            .find(|m| m.id == "opus-mt-en-fr")
            .unwrap()
            .clone();
        let key = cache_key("en", "fr", "Hello", &[&entry]);
        assert_ne!(key, cache_key("en", "fr", "Hello!", &[&entry]));
        assert_ne!(key, cache_key("fr", "en", "Hello", &[&entry]));
        assert_ne!(key, cache_key("en", "fr", "Hello", &[&entry, &entry]));
        entry.revision.push_str("-next");
        assert_ne!(key, cache_key("en", "fr", "Hello", &[&entry]));
        let value: serde_json::Value = serde_json::from_str(&key).unwrap();
        assert_eq!(value["pipeline"], "caption-translation-v1");
        assert!(!key.contains("Hello"));
    }

    #[test]
    fn rejects_malformed_engine_output_before_cache_write() {
        assert!(validate_output(&["Bonjour".into()], 1).is_ok());
        assert!(validate_output(&[], 1).is_err());
        assert!(validate_output(&[" ".into()], 1).is_err());
    }
}
