use std::collections::BTreeSet;

use nova_core::catalog::{intelligence::*, SnapshotId, SourceRecordId};
use serde_json::{json, Value};

fn context() -> ResolutionContext {
    ResolutionContext {
        record_id: SourceRecordId::new("record-1").unwrap(),
        snapshot_id: SnapshotId::new("snapshot-1").unwrap(),
        evidence: vec![evidence(1, "TF1", Authority::Official)],
    }
}

fn evidence(id: u16, name: &str, authority: Authority) -> Evidence {
    Evidence {
        id,
        fact: FactValue::CanonicalName(name.into()),
        authority,
        retrieved_at: 10,
        expires_at: 100,
        provenance: SourceRecordId::new(format!("protected-fact-{id}")).unwrap(),
    }
}

fn policy() -> Policy {
    Policy {
        countries: BTreeSet::from(["FR".into()]),
        languages: BTreeSet::from(["ar".into(), "fr".into()]),
    }
}

fn candidate(name: &str, id: u16, confidence: u16) -> Value {
    json!({"fact":{"field":"canonical_name","value":name}, "confidence":confidence,"evidence_ids":[id]})
}

fn proposal(candidates: Vec<Value>) -> Value {
    json!({"version":1,"record_id":"record-1","snapshot_id":"snapshot-1","candidates":candidates})
}

fn evaluate(
    value: Value,
    ctx: &ResolutionContext,
    now: u64,
) -> Result<Assessment, IntelligenceError> {
    assess(&serde_json::to_vec(&value).unwrap(), ctx, &policy(), now)
}

#[test]
fn authority_wins_over_self_score_and_preserves_conflicts() {
    let mut ctx = context();
    ctx.evidence.push(evidence(2, "Wrong", Authority::Provider));
    let result = evaluate(
        proposal(vec![candidate("Wrong", 2, 10000), candidate("TF1", 1, 1)]),
        &ctx,
        20,
    )
    .unwrap();
    let field = &result.fields[&Field::CanonicalName];
    assert_eq!(field.state, FieldState::Supported);
    assert_eq!(field.selected, Some(1));
    assert!(field.has_conflict);
    assert_eq!(field.candidates.len(), 2);
}

#[test]
fn equal_authority_conflict_abstains_independent_of_order() {
    let mut ctx = context();
    ctx.evidence
        .push(evidence(2, "Other TF1", Authority::Official));
    for candidates in [
        vec![candidate("TF1", 1, 9999), candidate("Other TF1", 2, 1)],
        vec![candidate("Other TF1", 2, 1), candidate("TF1", 1, 9999)],
    ] {
        let result = evaluate(proposal(candidates), &ctx, 20).unwrap();
        let field = &result.fields[&Field::CanonicalName];
        assert_eq!(field.state, FieldState::Conflicted);
        assert_eq!(field.selected, None);
    }
}

#[test]
fn inference_stays_provisional_even_at_maximum_confidence() {
    let mut ctx = context();
    ctx.evidence[0].authority = Authority::Inference;
    let result = evaluate(proposal(vec![candidate("TF1", 1, 10000)]), &ctx, 20).unwrap();
    assert_eq!(
        result.fields[&Field::CanonicalName].state,
        FieldState::Provisional
    );
}

#[test]
fn expiry_and_future_retrieval_never_accept_facts() {
    for now in [0, 100, u64::MAX] {
        let result = evaluate(proposal(vec![candidate("TF1", 1, 10000)]), &context(), now).unwrap();
        assert_eq!(
            result.fields[&Field::CanonicalName].state,
            FieldState::Unknown
        );
    }
}

#[test]
fn forged_or_suppressed_evidence_is_rejected() {
    for c in [
        candidate("TF1", 99, 10000),
        candidate("Fabrication", 1, 10000),
        candidate("TF1", 1, 10001),
    ] {
        assert!(matches!(
            evaluate(proposal(vec![c]), &context(), 20),
            Err(IntelligenceError::Evidence)
        ));
    }
    let mut ctx = context();
    ctx.evidence
        .push(evidence(2, "Conflict", Authority::Official));
    assert!(matches!(
        evaluate(proposal(vec![candidate("TF1", 1, 1)]), &ctx, 20),
        Err(IntelligenceError::Evidence)
    ));
}

#[test]
fn strict_schema_rejects_unknown_fields_taxonomy_and_duplicate_keys() {
    let mut value = proposal(vec![candidate("TF1", 1, 1)]);
    value["write_database"] = json!(true);
    assert!(matches!(
        evaluate(value, &context(), 20),
        Err(IntelligenceError::Schema)
    ));
    let value = proposal(vec![
        json!({"fact":{"field":"content_type","value":"invented_genre"},"confidence":1,"evidence_ids":[1]}),
    ]);
    assert!(matches!(
        evaluate(value, &context(), 20),
        Err(IntelligenceError::Schema)
    ));
    let value = br#"{"version":1,"version":1,"record_id":"record-1","snapshot_id":"snapshot-1","candidates":[]}"#;
    assert!(matches!(
        assess(value, &context(), &policy(), 20),
        Err(IntelligenceError::Schema)
    ));
}

#[test]
fn context_mismatch_and_oversize_are_rejected_before_acceptance() {
    for key in ["record_id", "snapshot_id"] {
        let mut value = proposal(vec![candidate("TF1", 1, 1)]);
        value[key] = json!("other");
        assert!(matches!(
            evaluate(value, &context(), 20),
            Err(IntelligenceError::Context)
        ));
    }
    assert!(matches!(
        assess(
            &vec![b' '; MAX_PROPOSAL_BYTES + 1],
            &context(),
            &policy(),
            20
        ),
        Err(IntelligenceError::Limit)
    ));
}

#[test]
fn playback_observations_are_separate_from_provider_hints() {
    let mut ctx = context();
    ctx.evidence[0].fact = FactValue::QualityHint(Quality::Fhd);
    ctx.evidence[0].authority = Authority::Provider;
    let mut observed = evidence(2, "unused", Authority::ObservedPlayback);
    observed.fact = FactValue::ObservedQuality(Quality::Uhd);
    ctx.evidence.push(observed);
    let value = proposal(vec![
        json!({"fact":{"field":"quality_hint","value":"fhd"},"confidence":9000,"evidence_ids":[1]}),
        json!({"fact":{"field":"observed_quality","value":"uhd"},"confidence":1,"evidence_ids":[2]}),
    ]);
    let result = evaluate(value.clone(), &ctx, 20).unwrap();
    assert_eq!(
        result.fields[&Field::QualityHint].state,
        FieldState::Provisional
    );
    assert_eq!(
        result.fields[&Field::ObservedQuality].state,
        FieldState::Supported
    );
    ctx.evidence[1].authority = Authority::Official;
    assert!(matches!(
        evaluate(value, &ctx, 20),
        Err(IntelligenceError::Evidence)
    ));
}

#[test]
fn multilingual_names_preserved_and_language_codes_controlled() {
    let mut ctx = context();
    ctx.evidence[0].fact = FactValue::CanonicalName("قناة العربية".into());
    assert!(evaluate(proposal(vec![candidate("قناة العربية", 1, 9000)]), &ctx, 20).is_ok());
    ctx.evidence[0].fact = FactValue::ChannelLanguage(vec!["made-up".into()]);
    assert!(matches!(
        evaluate(proposal(vec![]), &ctx, 20),
        Err(IntelligenceError::Evidence)
    ));
    ctx.evidence[0].fact = FactValue::ChannelLanguage(vec!["ar".into(), "fr".into()]);
    let value = proposal(vec![
        json!({"fact":{"field":"channel_language","value":["ar","fr"]},"confidence":9000,"evidence_ids":[1]}),
    ]);
    let result = evaluate(value, &ctx, 20).unwrap();
    assert!(result.fields.contains_key(&Field::ChannelLanguage));
    assert!(!result.fields.contains_key(&Field::AudioLanguage));
}
