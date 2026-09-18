//! Synthetic transport probe. Start the pinned evaluation runtime separately.
//! This does not attest the running process or publish any catalog data.
use std::{collections::BTreeSet, error::Error, time::Duration};

use nova_catalog_reasoner::{
    CatalogReasoner, DeclaredArtifacts, InputDraft, LocalReasoner, ResolutionInput,
};
use nova_core::catalog::{
    intelligence::{FieldState, Policy},
    SnapshotId, SourceRecordId,
};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../tools/catalog-eval/qwen35-4b.manifest.json"
    ))?;
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../fixtures/catalog-intelligence/v1/smoke.json"
    ))?;
    let port = std::env::args()
        .nth(1)
        .map(|s| s.parse::<u16>())
        .transpose()?
        .unwrap_or(18491);
    let artifact = |field: &str| -> Result<String, Box<dyn Error>> {
        Ok(manifest[field]["sha256"]
            .as_str()
            .ok_or("missing artifact digest")?
            .to_owned())
    };
    let reasoner = LocalReasoner::new(
        port,
        Duration::from_secs(45),
        DeclaredArtifacts::new(artifact("model")?, artifact("runtimeArchive")?)?,
    )?;
    let draft: InputDraft = serde_json::from_value(corpus["cases"][0]["input"].clone())?;
    let input = ResolutionInput::new(
        SourceRecordId::new("synthetic-probe")?,
        SnapshotId::new("synthetic-snapshot")?,
        draft,
        BTreeSet::new(),
    )?;
    let result = reasoner.resolve(&input).await?;
    let expected = corpus["cases"][0]["expected"]["candidate_id"]
        .as_str()
        .ok_or("missing expected candidate")?;
    if result.proposed_entity().map(|id| id.as_str()) != Some(expected) {
        return Err("synthetic candidate mismatch".into());
    }
    let bundle = result.provisional(
        SourceRecordId::new("synthetic-derivation-not-persisted")?,
        100,
        60,
    )?;
    let assessment = bundle.assess(
        &Policy {
            countries: BTreeSet::new(),
            languages: BTreeSet::new(),
        },
        101,
    )?;
    if assessment.fields.is_empty()
        || assessment
            .fields
            .values()
            .any(|field| field.state != FieldState::Provisional)
    {
        return Err("facts did not remain provisional".into());
    }
    println!("Synthetic Qwen probe passed: {} provisional fields; no publication or runtime attestation.", assessment.fields.len());
    Ok(())
}
