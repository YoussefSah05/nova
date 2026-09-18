#![cfg(unix)]
use nova_catalog_ingest::stage::*;
use nova_catalog_ingest::*;
use nova_core::catalog::{snapshot::*, *};
use nova_storage::snapshots::{stream::*, ProfileKey};
use std::num::{NonZeroU32, NonZeroU64};
use uuid::Uuid;

fn context(id: &str) -> RecordContext {
    RecordContext {
        id: SourceRecordId::new(id).unwrap(),
        source_id: SourceId::new("source").unwrap(),
        snapshot_id: SnapshotId::new("raw").unwrap(),
        stream_ref: StreamReference::new(format!("locator-{id}")).unwrap(),
    }
}
fn input(raw: &[u8]) -> SourceSnapshot {
    let mut capture = SnapshotCapture::new(
        SnapshotDescriptor {
            id: SnapshotId::new("raw").unwrap(),
            source_id: SourceId::new("source").unwrap(),
            scope: EndpointScope::new("vod").unwrap(),
            parser_revision: NonZeroU32::new(1).unwrap(),
        },
        SnapshotLimits {
            max_bytes: NonZeroU64::new(1_000_000).unwrap(),
            max_records: NonZeroU64::new(100).unwrap(),
        },
    );
    capture.push_bytes(raw).unwrap();
    capture.seal(Completeness::Partial).unwrap()
}
fn limits() -> StageLimits {
    StageLimits {
        max_records: 100,
        max_bytes: 1_000_000,
    }
}
async fn store(dir: &std::path::Path, profile: Uuid) -> StreamingSnapshotStore {
    StreamingSnapshotStore::open(
        dir.to_owned(),
        profile,
        ProfileKey::new([7; 32]),
        StreamLimits {
            max_payload_bytes: 1_000_000,
            max_stored_bytes: 2_000_000,
            max_files: 10,
        },
    )
    .await
    .unwrap()
}

#[test]
fn raw_object_and_unknown_fields_are_preserved_exactly() {
    let bytes = br#"{ "stream_id":201, "name":"Original", "unknown":{"token":"synthetic", "x":1.00}, "direct_source":"https://invalid.test/?password=synthetic" }"#;
    let entry = parse_xtream_object(XtreamScope::Vod, context("r1"), bytes).unwrap();
    assert_eq!(entry.raw_object(), bytes);
    assert_eq!(entry.record().raw_title(), "Original");
    assert_eq!(entry.record().upstream().unwrap().value, "201");
    assert!(!entry.record().attributes().contains_key("direct_source"));
    assert!(!entry.record().attributes().contains_key("unknown"));
    assert!(!format!("{:?}", entry.record()).contains("synthetic"));
}
#[test]
fn explicit_scope_preserves_containers_and_numbered_channels() {
    for (scope, raw, kind, namespace) in [
        (
            XtreamScope::Live,
            r#"{"stream_id":"001","name":"AR| رياضة 2 HD"}"#,
            ContentKind::LiveChannel,
            "xtream-live",
        ),
        (
            XtreamScope::Series,
            r#"{"series_id":301,"name":"FR| Exemple / مثال"}"#,
            ContentKind::Series,
            "xtream-series",
        ),
        (
            XtreamScope::Vod,
            r#"{"stream_id":201,"name":"Dune (1984)"}"#,
            ContentKind::Movie,
            "xtream-vod",
        ),
    ] {
        let entry = parse_xtream_object(scope, context("r"), raw.as_bytes()).unwrap();
        assert_eq!(entry.record().kind_hint(), kind);
        assert_eq!(entry.record().upstream().unwrap().namespace, namespace);
        assert_eq!(entry.raw_object(), raw.as_bytes());
    }
}
#[test]
fn invalid_and_ambiguous_objects_fail_without_echoing_payloads() {
    for bytes in [
        br#"{"name":"secret","name":"other"}"#.as_slice(),
        br#"{"name":123}"#,
        br#"{"stream_id":1.5}"#,
        b"[]",
        b"{} trailing-secret",
        br#"{"name": "secret""#,
    ] {
        let error = parse_xtream_object(XtreamScope::Vod, context("r"), bytes)
            .err()
            .unwrap();
        assert!(!format!("{error:?} {error}").contains("secret"));
    }
    assert!(parse_xtream_object(
        XtreamScope::Vod,
        context("r"),
        &vec![b' '; MAX_RAW_OBJECT_BYTES + 1]
    )
    .is_err());
    let absent = parse_xtream_object(XtreamScope::Vod, context("r"), b"{}").unwrap();
    assert!(absent.record().upstream().is_none());
    assert_eq!(absent.record().raw_title(), "");
}

#[tokio::test]
async fn archive_preserves_duplicate_provider_ids_and_replays_after_reopen() {
    let raw = include_bytes!("../../../fixtures/iptv/v1/vod.json");
    let objects: Vec<&serde_json::value::RawValue> = serde_json::from_slice(raw).unwrap();
    let entries: Vec<_> = objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            parse_xtream_object(
                XtreamScope::Vod,
                context(&format!("r{i}")),
                object.get().as_bytes(),
            )
            .unwrap()
        })
        .collect();
    let receipt = input(raw);
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile).await;
    let mut stage = RecordStage::begin(&db, SnapshotId::new("stage").unwrap(), &receipt, limits())
        .await
        .unwrap();
    for entry in &entries {
        stage.push(entry).await.unwrap();
    }
    assert_eq!(stage.finish().await.unwrap(), 5);
    drop(db);
    let db = store(dir.path(), profile).await;
    let mut replayed = Vec::new();
    let count = replay_stage(
        &db,
        SnapshotId::new("stage").unwrap(),
        &receipt,
        limits(),
        |entry| {
            replayed.push(entry);
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(count, 5);
    assert_eq!(
        replayed
            .iter()
            .filter(|r| r.record().upstream().unwrap().value == "201")
            .count(),
        2
    );
    for (a, b) in entries.iter().zip(&replayed) {
        assert_eq!(a.raw_object(), b.raw_object());
        assert_eq!(
            a.record().metadata_fingerprint(),
            b.record().metadata_fingerprint()
        );
        assert_eq!(a.record().id(), b.record().id());
        assert_eq!(a.record().stream_ref(), b.record().stream_ref());
    }
}

#[tokio::test]
async fn duplicate_occurrence_or_foreign_source_aborts_stage() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile).await;
    let raw = br#"{"name":"Title"}"#;
    let receipt = input(raw);
    let entry = parse_xtream_object(XtreamScope::Vod, context("r"), raw).unwrap();
    let mut stage = RecordStage::begin(
        &db,
        SnapshotId::new("duplicate").unwrap(),
        &receipt,
        limits(),
    )
    .await
    .unwrap();
    stage.push(&entry).await.unwrap();
    assert!(matches!(
        stage.push(&entry).await,
        Err(IngestError::DuplicateRecord)
    ));
    assert!(stage.finish().await.is_err());
    let mut foreign = context("other");
    foreign.source_id = SourceId::new("foreign").unwrap();
    let entry = parse_xtream_object(XtreamScope::Vod, foreign, raw).unwrap();
    let mut stage =
        RecordStage::begin(&db, SnapshotId::new("foreign").unwrap(), &receipt, limits())
            .await
            .unwrap();
    assert!(matches!(
        stage.push(&entry).await,
        Err(IngestError::Ownership)
    ));
    assert!(stage.finish().await.is_err());
    assert!(!dir
        .path()
        .join(profile.to_string())
        .join("streams-v2/duplicate.nvs2")
        .exists());
}
#[tokio::test]
async fn stage_record_limit_is_terminal_and_wrong_raw_receipt_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile).await;
    let raw = br#"{"name":"Title"}"#;
    let receipt = input(raw);
    let a = parse_xtream_object(XtreamScope::Vod, context("a"), raw).unwrap();
    let b = parse_xtream_object(XtreamScope::Vod, context("b"), raw).unwrap();
    let mut stage = RecordStage::begin(
        &db,
        SnapshotId::new("full").unwrap(),
        &receipt,
        StageLimits {
            max_records: 1,
            ..limits()
        },
    )
    .await
    .unwrap();
    stage.push(&a).await.unwrap();
    assert!(stage.push(&b).await.is_err());
    assert!(stage.finish().await.is_err());
    let mut stage = RecordStage::begin(&db, SnapshotId::new("good").unwrap(), &receipt, limits())
        .await
        .unwrap();
    stage.push(&a).await.unwrap();
    stage.finish().await.unwrap();
    let mut seen = false;
    assert!(replay_stage(
        &db,
        SnapshotId::new("good").unwrap(),
        &input(b"other payload"),
        limits(),
        |_| {
            seen = true;
            Ok(())
        }
    )
    .await
    .is_err());
    assert!(!seen);
}

#[tokio::test]
async fn empty_archive_and_tiny_budget_have_explicit_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let profile = Uuid::new_v4();
    let db = store(dir.path(), profile).await;
    let receipt = input(b"[]");
    assert!(RecordStage::begin(
        &db,
        SnapshotId::new("tiny").unwrap(),
        &receipt,
        StageLimits {
            max_bytes: 1,
            ..limits()
        }
    )
    .await
    .is_err());
    let stage = RecordStage::begin(&db, SnapshotId::new("empty").unwrap(), &receipt, limits())
        .await
        .unwrap();
    assert_eq!(stage.finish().await.unwrap(), 0);
    assert_eq!(
        replay_stage(
            &db,
            SnapshotId::new("empty").unwrap(),
            &receipt,
            limits(),
            |_| panic!("no records expected")
        )
        .await
        .unwrap(),
        0
    );
    assert!(!dir
        .path()
        .join(profile.to_string())
        .join("streams-v2/tiny.nvs2")
        .exists());
}
