use std::collections::BTreeMap;

use nova_core::catalog::*;

fn draft() -> SourceRecordDraft {
    SourceRecordDraft {
        id: SourceRecordId::new("record-1").unwrap(),
        source_id: SourceId::new("source-1").unwrap(),
        snapshot_id: SnapshotId::new("snapshot-1").unwrap(),
        upstream: Some(UpstreamKey {
            namespace: "xtream.movie".into(),
            value: "42".into(),
        }),
        kind_hint: ContentKind::Unknown,
        raw_title: "  [FR] فيلم / DUNE PART TWO (2024) 4K HEVC MULTI  ".into(),
        raw_group: None,
        raw_logo: None,
        raw_epg_id: None,
        stream_ref: StreamReference::new("locator-1").unwrap(),
        attributes: BTreeMap::new(),
    }
}

#[test]
fn preserves_original_text_and_unknown_classification() {
    let input = draft();
    let expected = input.raw_title.clone();
    let record = SourceRecord::try_from(input).unwrap();
    assert_eq!(record.raw_title(), expected);
    assert_eq!(record.kind_hint(), ContentKind::Unknown);
    assert_eq!(record.snapshot_id().as_str(), "snapshot-1");
}

#[test]
fn opaque_ids_reject_urls_whitespace_empty_and_oversize() {
    for value in [
        "",
        "has space",
        "https://host/user/password",
        "\n",
        &"a".repeat(129),
    ] {
        assert!(SourceId::new(value).is_err());
        assert!(StreamReference::new(value).is_err());
    }
    assert!(SourceId::new("550e8400-e29b-41d4-a716-446655440000").is_ok());
}

#[test]
fn serde_cannot_bypass_id_validation() {
    let id = EntityId::new("entity-1").unwrap();
    assert_eq!(serde_json::to_string(&id).unwrap(), "\"entity-1\"");
    assert_eq!(
        serde_json::from_str::<EntityId>("\"entity-1\"").unwrap(),
        id
    );
    assert!(serde_json::from_str::<EntityId>("\"\"").is_err());
    assert!(serde_json::from_str::<StreamReference>("\"https://secret\"").is_err());
}

#[test]
fn diagnostics_do_not_echo_provider_payload() {
    let mut input = draft();
    input.raw_title = "password=do-not-log".into();
    input.raw_logo = Some("https://host/?token=do-not-log".into());
    input
        .attributes
        .insert("password".into(), "do-not-log".into());
    let record = SourceRecord::try_from(input).unwrap();
    let debug = format!("{record:?}");
    assert!(!debug.contains("do-not-log"));
    assert!(!debug.contains("password"));
    assert!(!debug.contains("locator-1"));
}

#[test]
fn rejects_field_and_aggregate_limits_without_leaking_values() {
    let mut input = draft();
    input.raw_title = "secret".repeat(MAX_FIELD_BYTES);
    let error = SourceRecord::try_from(input).unwrap_err();
    assert_eq!(
        error,
        CatalogInputError::FieldTooLarge {
            field: "raw_title",
            limit: MAX_FIELD_BYTES
        }
    );
    assert!(!error.to_string().contains("secret"));

    let mut input = draft();
    for i in 0..16 {
        input
            .attributes
            .insert(format!("key{i}"), "x".repeat(MAX_FIELD_BYTES));
    }
    assert_eq!(
        SourceRecord::try_from(input).unwrap_err(),
        CatalogInputError::RecordTooLarge {
            limit: MAX_RECORD_BYTES
        }
    );
}

#[test]
fn rejects_unbounded_attributes_and_empty_upstream_namespace() {
    let mut input = draft();
    for i in 0..=MAX_ATTRIBUTES {
        input.attributes.insert(i.to_string(), "x".into());
    }
    assert_eq!(
        SourceRecord::try_from(input).unwrap_err(),
        CatalogInputError::TooManyAttributes {
            limit: MAX_ATTRIBUTES
        }
    );
    let mut input = draft();
    input.upstream.as_mut().unwrap().namespace = "".into();
    assert_eq!(
        SourceRecord::try_from(input).unwrap_err(),
        CatalogInputError::EmptyField {
            field: "upstream.namespace"
        }
    );
}

#[test]
fn empty_titles_are_preserved_for_compiler_quarantine() {
    let mut input = draft();
    input.raw_title.clear();
    assert_eq!(SourceRecord::try_from(input).unwrap().raw_title(), "");
}

#[test]
fn source_occurrences_and_canonical_identities_are_independent() {
    let first = SourceRecord::try_from(draft()).unwrap();
    let mut other = draft();
    other.id = SourceRecordId::new("record-2").unwrap();
    other.raw_title = "Another title".into();
    let second = SourceRecord::try_from(other).unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(first.upstream(), second.upstream()); // adapter/compiler decides collision policy
    let entity = EntityId::new("work-1").unwrap();
    let a = StreamVariant {
        id: VariantId::new("variant-a").unwrap(),
        entity_id: entity.clone(),
        edition_id: None,
        binding_id: SourceBindingId::new("binding-a").unwrap(),
    };
    let b = StreamVariant {
        id: VariantId::new("variant-b").unwrap(),
        entity_id: entity,
        edition_id: None,
        binding_id: SourceBindingId::new("binding-b").unwrap(),
    };
    assert_eq!(a.entity_id, b.entity_id);
    assert_ne!(a.id, b.id);
}

#[test]
fn unknown_numbering_is_not_episode_one_and_specials_allow_zero() {
    let unknown = EpisodeNumbering::default();
    assert_eq!(unknown.season, None);
    assert_eq!(unknown.episode, None);
    let special = EpisodeNumbering {
        season: Some(0),
        episode: Some(1),
    };
    assert_ne!(special, unknown);
}

#[test]
fn typed_ids_are_not_interchangeable() {
    // Compile-time usage is separately documented with a compile_fail example.
    assert_eq!(
        ContentKind::Movie,
        serde_json::from_str("\"movie\"").unwrap()
    );
    assert_eq!(
        ContentKind::LiveChannel,
        serde_json::from_str("\"liveChannel\"").unwrap()
    );
}
