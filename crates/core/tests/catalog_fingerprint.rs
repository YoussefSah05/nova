use std::collections::BTreeMap;

use nova_core::catalog::fingerprint::{VersionedFingerprint, METADATA_FINGERPRINT_VERSION};
use nova_core::catalog::*;

fn draft(id: &str, title: &str) -> SourceRecordDraft {
    SourceRecordDraft {
        id: SourceRecordId::new(id).unwrap(),
        source_id: SourceId::new("source-1").unwrap(),
        snapshot_id: SnapshotId::new("snapshot-1").unwrap(),
        upstream: Some(UpstreamKey {
            namespace: "xtream-vod".into(),
            value: "42".into(),
        }),
        kind_hint: ContentKind::Movie,
        raw_title: title.into(),
        raw_group: None,
        raw_logo: None,
        raw_epg_id: None,
        stream_ref: StreamReference::new("locator-1").unwrap(),
        attributes: BTreeMap::new(),
    }
}

#[test]
fn metadata_fingerprint_is_stable_across_recomputation() {
    let record = SourceRecord::try_from(draft("r1", "Dune Part Two")).unwrap();
    let a = VersionedFingerprint::current_metadata(&record);
    let b = VersionedFingerprint::current_metadata(&record);
    assert_eq!(a, b);
    assert_eq!(a.to_stored(), b.to_stored());
}

#[test]
fn metadata_fingerprint_changes_when_a_hashed_field_changes() {
    let a = SourceRecord::try_from(draft("r1", "Dune Part Two")).unwrap();
    let b = SourceRecord::try_from(draft("r1", "Dune Part Three")).unwrap();
    assert_ne!(
        VersionedFingerprint::current_metadata(&a),
        VersionedFingerprint::current_metadata(&b)
    );
}

#[test]
fn metadata_fingerprint_ignores_the_ephemeral_occurrence_id() {
    // Two ingest occurrences of the same upstream item must fingerprint
    // identically: `SourceRecordId` is per-occurrence and is never hashed.
    let a = SourceRecord::try_from(draft("occurrence-1", "Dune Part Two")).unwrap();
    let b = SourceRecord::try_from(draft("occurrence-2", "Dune Part Two")).unwrap();
    assert_eq!(
        VersionedFingerprint::current_metadata(&a),
        VersionedFingerprint::current_metadata(&b)
    );
}

#[test]
fn stored_encoding_round_trips_and_rejects_malformed_input() {
    let record = SourceRecord::try_from(draft("r1", "Heat")).unwrap();
    let versioned = VersionedFingerprint::current_metadata(&record);
    let stored = versioned.to_stored();
    assert_eq!(VersionedFingerprint::from_stored(&stored), Some(versioned));

    for bad in [
        "",
        "not-a-fingerprint",
        "01-tooshort",
        &format!("01-{}", "g".repeat(64)), // non-hex digest
        &"0".repeat(64),                   // no separator
    ] {
        assert!(
            VersionedFingerprint::from_stored(bad).is_none(),
            "expected rejection for {bad:?}"
        );
    }
}

#[test]
fn a_persisted_older_format_version_is_never_treated_as_current() {
    let record = SourceRecord::try_from(draft("r1", "Heat")).unwrap();
    let current = VersionedFingerprint::current_metadata(&record);
    assert!(current.is_current_metadata());

    // Simulate a fingerprint stored by a previous build: identical digest
    // bytes, an older format tag. Even a byte-for-byte digest match must not
    // be trusted once the format version has moved on.
    let stale_version = METADATA_FINGERPRINT_VERSION.wrapping_sub(1);
    let stale_stored = format!("{stale_version:04x}-{}", current.fingerprint().to_hex());
    let stale = VersionedFingerprint::from_stored(&stale_stored).unwrap();

    assert_eq!(stale.fingerprint(), current.fingerprint());
    assert_ne!(stale.version(), current.version());
    assert!(!stale.is_current_metadata());
}

#[test]
fn different_upstream_keys_never_collide() {
    let mut a = draft("r1", "Same Title");
    a.upstream = Some(UpstreamKey {
        namespace: "xtream-vod".into(),
        value: "1".into(),
    });
    let mut b = draft("r2", "Same Title");
    b.upstream = Some(UpstreamKey {
        namespace: "xtream-vod".into(),
        value: "2".into(),
    });
    let a = SourceRecord::try_from(a).unwrap();
    let b = SourceRecord::try_from(b).unwrap();
    assert_ne!(
        VersionedFingerprint::current_metadata(&a),
        VersionedFingerprint::current_metadata(&b)
    );
}
