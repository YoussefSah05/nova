use nova_core::catalog::{fingerprint::*, snapshot::*, *};
use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroU64},
};

fn record(snapshot: &str) -> SourceRecordDraft {
    SourceRecordDraft {
        id: SourceRecordId::new("r1").unwrap(),
        source_id: SourceId::new("s1").unwrap(),
        snapshot_id: SnapshotId::new(snapshot).unwrap(),
        upstream: None,
        kind_hint: ContentKind::Movie,
        raw_title: "Dune / فيلم".into(),
        raw_group: None,
        raw_logo: None,
        raw_epg_id: None,
        stream_ref: StreamReference::new("locator1").unwrap(),
        attributes: BTreeMap::new(),
    }
}
fn capture(id: &str, scope: &str, bytes: u64, records: u64) -> SnapshotCapture {
    SnapshotCapture::new(
        SnapshotDescriptor {
            id: SnapshotId::new(id).unwrap(),
            source_id: SourceId::new("s1").unwrap(),
            scope: EndpointScope::new(scope).unwrap(),
            parser_revision: NonZeroU32::new(1).unwrap(),
        },
        SnapshotLimits {
            max_bytes: NonZeroU64::new(bytes).unwrap(),
            max_records: NonZeroU64::new(records).unwrap(),
        },
    )
}

#[test]
fn payload_hash_matches_independent_sha256_vector_and_chunking() {
    let mut a = capture("a", "movies", 100, 10);
    a.push_bytes(b"abc").unwrap();
    let a = a.seal(Completeness::Complete).unwrap();
    let mut b = capture("b", "movies", 100, 10);
    b.push_bytes(b"a").unwrap();
    b.push_bytes(b"").unwrap();
    b.push_bytes(b"bc").unwrap();
    let b = b.seal(Completeness::Complete).unwrap();
    assert_eq!(a.payload_fingerprint(), b.payload_fingerprint());
    assert_eq!(
        a.payload_fingerprint().to_hex(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(a.byte_count(), 3);
}

#[test]
fn metadata_ignores_capture_ids_and_locator_but_preserves_exact_text() {
    let a = SourceRecord::try_from(record("old")).unwrap();
    let mut draft = record("new");
    draft.id = SourceRecordId::new("new-record").unwrap();
    draft.stream_ref = StreamReference::new("new-locator").unwrap();
    let b = SourceRecord::try_from(draft).unwrap();
    assert_eq!(a.metadata_fingerprint(), b.metadata_fingerprint());
    let mut draft = record("new");
    draft.raw_title.push(' ');
    assert_ne!(
        a.metadata_fingerprint(),
        SourceRecord::try_from(draft)
            .unwrap()
            .metadata_fingerprint()
    );
}

#[test]
fn framed_v1_matches_independently_computed_vectors() {
    // Generated independently with Python hashlib and struct.pack('>Q', byte_length).
    assert_eq!(FINGERPRINT_FORMAT, "sha256-framed-v1");
    assert_eq!(
        SourceRecord::try_from(record("a"))
            .unwrap()
            .metadata_fingerprint()
            .to_hex(),
        "96e537528020530541765e9f34b8e0f7e937d2890d0b9cb51adc7f3846cdc92a"
    );
    assert_eq!(
        locator_fingerprint(b"ab", b"c").to_hex(),
        "694c52289cf66a64f82d40a8a8c2ef34583245ce92f6b8c3a2c6b5882166a46c"
    );
}

#[test]
fn every_metadata_evidence_field_participates() {
    let baseline = SourceRecord::try_from(record("a"))
        .unwrap()
        .metadata_fingerprint();
    for change in 0..6 {
        let mut draft = record("a");
        match change {
            0 => draft.kind_hint = ContentKind::LiveChannel,
            1 => draft.raw_logo = Some("logo-ref".into()),
            2 => draft.raw_epg_id = Some("channel-1".into()),
            3 => {
                draft.upstream = Some(UpstreamKey {
                    namespace: "movie".into(),
                    value: "1".into(),
                })
            }
            4 => {
                draft.attributes.insert("language".into(), "ar".into());
            }
            _ => draft.raw_group = Some("group".into()),
        }
        assert_ne!(
            baseline,
            SourceRecord::try_from(draft)
                .unwrap()
                .metadata_fingerprint()
        );
    }
}

#[test]
fn field_boundaries_options_and_attribute_order_are_unambiguous() {
    let mut a = record("a");
    a.raw_title = "ab".into();
    a.raw_group = Some("c".into());
    let mut b = record("b");
    b.raw_title = "a".into();
    b.raw_group = Some("bc".into());
    assert_ne!(
        SourceRecord::try_from(a).unwrap().metadata_fingerprint(),
        SourceRecord::try_from(b).unwrap().metadata_fingerprint()
    );
    let a = SourceRecord::try_from(record("a")).unwrap();
    let mut b = record("b");
    b.raw_group = Some(String::new());
    assert_ne!(
        a.metadata_fingerprint(),
        SourceRecord::try_from(b).unwrap().metadata_fingerprint()
    );
    let mut a = record("a");
    let mut b = record("b");
    for key in ["z", "a"] {
        a.attributes.insert(key.into(), key.into());
    }
    for key in ["a", "z"] {
        b.attributes.insert(key.into(), key.into());
    }
    assert_eq!(
        SourceRecord::try_from(a).unwrap().metadata_fingerprint(),
        SourceRecord::try_from(b).unwrap().metadata_fingerprint()
    );
}

#[test]
fn locator_changes_and_credential_revision_invalidate_only_locator_digest() {
    assert_ne!(
        locator_fingerprint(b"ab", b"c"),
        locator_fingerprint(b"a", b"bc")
    );
    assert_ne!(
        locator_fingerprint(b"url", b"rev1"),
        locator_fingerprint(b"url", b"rev2")
    );
    let digest = locator_fingerprint(b"password=secret", b"1");
    assert_eq!(format!("{digest:?}"), "Fingerprint([redacted])");
}

#[test]
fn complete_empty_is_authoritative_but_partial_empty_is_not() {
    let complete = capture("a", "movies", 10, 1)
        .seal(Completeness::Complete)
        .unwrap();
    let partial = capture("b", "movies", 10, 1)
        .seal(Completeness::Partial)
        .unwrap();
    assert_eq!(complete.record_count(), 0);
    assert_eq!(
        select_snapshot(&partial, Some(&complete))
            .unwrap()
            .snapshot()
            .descriptor()
            .id
            .as_str(),
        "a"
    );
    assert_eq!(
        select_snapshot(&complete, None).unwrap().freshness(),
        SnapshotFreshness::Current
    );
    assert_eq!(
        select_snapshot(&partial, None).unwrap().freshness(),
        SnapshotFreshness::PartialFirstImport
    );
}

#[test]
fn refresh_keeps_previous_complete_scope_and_rejects_foreign_scope() {
    let old = capture("old", "movies", 10, 1)
        .seal(Completeness::Complete)
        .unwrap();
    let partial = capture("new", "movies", 10, 1)
        .seal(Completeness::Partial)
        .unwrap();
    assert_eq!(
        select_snapshot(&partial, Some(&old)).unwrap().freshness(),
        SnapshotFreshness::StalePrevious
    );
    let other = capture("other", "series", 10, 1)
        .seal(Completeness::Complete)
        .unwrap();
    assert_eq!(
        select_snapshot(&partial, Some(&other)).unwrap_err(),
        SnapshotError::ScopeMismatch
    );
    assert_eq!(
        select_snapshot(&partial, Some(&partial)).unwrap_err(),
        SnapshotError::PreviousNotComplete
    );
}

#[test]
fn sealed_cancelled_and_failed_captures_are_terminal() {
    for terminal in [
        CaptureState::Complete,
        CaptureState::Partial,
        CaptureState::Cancelled,
        CaptureState::Failed,
    ] {
        let mut capture = capture("a", "movies", 10, 1);
        match terminal {
            CaptureState::Complete => {
                capture.seal(Completeness::Complete).unwrap();
            }
            CaptureState::Partial => {
                capture.seal(Completeness::Partial).unwrap();
            }
            CaptureState::Cancelled => {
                capture.cancel().unwrap();
            }
            CaptureState::Failed => {
                capture.fail().unwrap();
            }
            CaptureState::Receiving => unreachable!(),
        }
        assert_eq!(capture.state(), terminal);
        assert_eq!(
            capture.push_bytes(b"x").unwrap_err(),
            SnapshotError::Terminal { state: terminal }
        );
        assert!(capture.seal(Completeness::Complete).is_err());
        assert!(capture.cancel().is_err());
    }
}

#[test]
fn limits_fail_closed_and_do_not_produce_truncated_complete_receipts() {
    let mut capture = capture("a", "movies", 3, 1);
    capture.push_bytes(b"abc").unwrap();
    assert_eq!(
        capture.push_bytes(b"d").unwrap_err(),
        SnapshotError::ByteLimit
    );
    assert_eq!(capture.state(), CaptureState::Failed);
    assert!(capture.seal(Completeness::Complete).is_err());
}

#[test]
fn record_quota_and_foreign_record_fail_closed() {
    let item = SourceRecord::try_from(record("a")).unwrap();
    let mut limited = capture("a", "movies", 10, 1);
    limited.observe_record(&item).unwrap();
    assert_eq!(
        limited.observe_record(&item).unwrap_err(),
        SnapshotError::RecordLimit
    );
    assert!(limited.seal(Completeness::Complete).is_err());
    let mut foreign = capture("other", "movies", 10, 1);
    assert_eq!(
        foreign.observe_record(&item).unwrap_err(),
        SnapshotError::RecordOwnership
    );
    assert_eq!(foreign.state(), CaptureState::Failed);

    let mut draft = record("a");
    draft.source_id = SourceId::new("other-source").unwrap();
    let mut wrong_source = capture("a", "movies", 10, 1);
    assert_eq!(
        wrong_source
            .observe_record(&SourceRecord::try_from(draft).unwrap())
            .unwrap_err(),
        SnapshotError::RecordOwnership
    );
}

#[test]
fn new_complete_scope_replaces_previous_and_cross_source_fallback_is_rejected() {
    let old = capture("old", "movies", 10, 1)
        .seal(Completeness::Complete)
        .unwrap();
    let new = capture("new", "movies", 10, 1)
        .seal(Completeness::Complete)
        .unwrap();
    assert_eq!(
        select_snapshot(&new, Some(&old))
            .unwrap()
            .snapshot()
            .descriptor()
            .id
            .as_str(),
        "new"
    );
    let mut other = SnapshotCapture::new(
        SnapshotDescriptor {
            id: SnapshotId::new("other").unwrap(),
            source_id: SourceId::new("s2").unwrap(),
            scope: EndpointScope::new("movies").unwrap(),
            parser_revision: NonZeroU32::new(1).unwrap(),
        },
        SnapshotLimits {
            max_bytes: NonZeroU64::new(10).unwrap(),
            max_records: NonZeroU64::new(1).unwrap(),
        },
    );
    assert_eq!(
        select_snapshot(&other.seal(Completeness::Partial).unwrap(), Some(&old)).unwrap_err(),
        SnapshotError::ScopeMismatch
    );
}

#[test]
fn receipt_records_parser_scope_and_count_without_retaining_payload() {
    let mut capture = capture("a", "movies", 100, 10);
    capture.push_bytes(b"secret provider body").unwrap();
    capture
        .observe_record(&SourceRecord::try_from(record("a")).unwrap())
        .unwrap();
    let receipt = capture.seal(Completeness::Complete).unwrap();
    assert_eq!(receipt.record_count(), 1);
    assert_eq!(receipt.descriptor().parser_revision.get(), 1);
    assert!(!format!("{receipt:?}").contains("secret"));
}
