//! First catalog-intelligence boundary; not connected to the production importer.
//! Models propose facts, never identities, source authority, or catalog writes.
//! Evidence registration is a trusted caller responsibility, not a model tool.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{SnapshotId, SourceRecordId};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_PROPOSAL_BYTES: usize = 32 * 1024;
pub const MAX_CANDIDATES: usize = 64;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntelligenceError {
    #[error("invalid catalog proposal schema")]
    Schema,
    #[error("catalog intelligence budget exceeded")]
    Limit,
    #[error("proposal context or protocol mismatch")]
    Context,
    #[error("invalid catalog evidence or policy")]
    Evidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    CanonicalName,
    EntityType,
    Country,
    ChannelLanguage,
    AudioLanguage,
    QualityHint,
    ObservedQuality,
    ContentType,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    LiveChannel,
    Movie,
    Series,
    Season,
    Episode,
    Event,
    Radio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Sd,
    Hd,
    Fhd,
    Uhd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    GeneralEntertainment,
    News,
    Sports,
    Movies,
    Series,
    Kids,
    Documentary,
    Music,
    Lifestyle,
    Religious,
    Shopping,
    Regional,
    International,
    Continuous,
    Event,
    Radio,
    Adult,
    Special,
    Other,
}

/// Small bounded values; no URL, external ID, or database mutation in the protocol.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(
    tag = "field",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FactValue {
    CanonicalName(String),
    EntityType(EntityType),
    Country(String),
    ChannelLanguage(Vec<String>),
    AudioLanguage(Vec<String>),
    QualityHint(Quality),
    ObservedQuality(Quality),
    ContentType(ContentType),
    Network(String),
}

impl FactValue {
    pub fn field(&self) -> Field {
        match self {
            Self::CanonicalName(_) => Field::CanonicalName,
            Self::EntityType(_) => Field::EntityType,
            Self::Country(_) => Field::Country,
            Self::ChannelLanguage(_) => Field::ChannelLanguage,
            Self::AudioLanguage(_) => Field::AudioLanguage,
            Self::QualityHint(_) => Field::QualityHint,
            Self::ObservedQuality(_) => Field::ObservedQuality,
            Self::ContentType(_) => Field::ContentType,
            Self::Network(_) => Field::Network,
        }
    }

    fn valid(&self, policy: &Policy) -> bool {
        match self {
            Self::CanonicalName(s) | Self::Network(s) => {
                !s.is_empty() && s.len() <= 256 && s.trim() == s && !s.chars().any(char::is_control)
            }
            Self::Country(s) => policy.countries.contains(s),
            Self::ChannelLanguage(values) | Self::AudioLanguage(values) => {
                !values.is_empty()
                    && values.len() <= 16
                    && values.windows(2).all(|pair| pair[0] < pair[1])
                    && values.iter().all(|s| policy.languages.contains(s))
            }
            _ => true,
        }
    }
}

/// Authority is assigned by Nova's evidence adapter, never deserialized from a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Authority {
    Inference,
    Provider,
    Metadata,
    Industry,
    Official,
    ObservedPlayback,
}

/// One assertion per evidence handle, with per-fact expiry and protected provenance.
/// The caller must verify the source, subject association, and fact extraction.
/// Neither a citation URL nor a model's assertion constitutes this verification.
pub struct Evidence {
    pub id: u16,
    pub fact: FactValue,
    pub authority: Authority,
    pub retrieved_at: u64,
    pub expires_at: u64,
    pub provenance: SourceRecordId,
}

pub struct ResolutionContext {
    pub record_id: SourceRecordId,
    pub snapshot_id: SnapshotId,
    pub evidence: Vec<Evidence>,
}

/// Registries are supplied by Nova, never by a model. Empty registries accept no codes.
pub struct Policy {
    pub countries: BTreeSet<String>,
    pub languages: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    version: u16,
    record_id: SourceRecordId,
    snapshot_id: SnapshotId,
    candidates: Vec<FactCandidate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactCandidate {
    pub fact: FactValue,
    /// Model self-score in basis points, retained for evaluation; not a probability.
    pub confidence: u16,
    pub evidence_ids: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldState {
    Supported,
    Provisional,
    Conflicted,
    Unknown,
}

pub struct FieldResolution {
    pub state: FieldState,
    /// Index into candidates, not a canonical database write or identity decision.
    pub selected: Option<usize>,
    /// Includes stale and lower-priority alternatives for explanation and replay.
    pub candidates: Vec<FactCandidate>,
    pub has_conflict: bool,
}

pub struct Assessment {
    pub fields: BTreeMap<Field, FieldResolution>,
}

/// Validate the entire response before returning any accepted fields. The caller
/// owns fallback publication, cache storage, inference execution and research.
pub fn assess(
    bytes: &[u8],
    context: &ResolutionContext,
    policy: &Policy,
    now: u64,
) -> Result<Assessment, IntelligenceError> {
    if bytes.len() > MAX_PROPOSAL_BYTES || context.evidence.len() > MAX_CANDIDATES {
        return Err(IntelligenceError::Limit);
    }
    let proposal: Proposal =
        serde_json::from_slice(bytes).map_err(|_| IntelligenceError::Schema)?;
    if proposal.version != PROTOCOL_VERSION
        || proposal.record_id != context.record_id
        || proposal.snapshot_id != context.snapshot_id
    {
        return Err(IntelligenceError::Context);
    }
    if proposal.candidates.len() > MAX_CANDIDATES {
        return Err(IntelligenceError::Limit);
    }
    let mut evidence = BTreeMap::new();
    for item in &context.evidence {
        if item.retrieved_at >= item.expires_at
            || !item.fact.valid(policy)
            || evidence.insert(item.id, item).is_some()
            || (item.authority == Authority::ObservedPlayback
                && !matches!(
                    item.fact,
                    FactValue::ObservedQuality(_) | FactValue::AudioLanguage(_)
                ))
            || (matches!(
                item.fact,
                FactValue::ObservedQuality(_) | FactValue::AudioLanguage(_)
            ) && item.authority != Authority::ObservedPlayback)
        {
            return Err(IntelligenceError::Evidence);
        }
    }
    let mut fields: BTreeMap<Field, FieldResolution> = BTreeMap::new();
    for candidate in proposal.candidates {
        if candidate.confidence > 10_000
            || !candidate.fact.valid(policy)
            || candidate.evidence_ids.is_empty()
            || candidate.evidence_ids.len() > 8
        {
            return Err(IntelligenceError::Evidence);
        }
        let mut seen = BTreeSet::new();
        for id in &candidate.evidence_ids {
            if !seen.insert(id) || evidence.get(id).is_none_or(|e| e.fact != candidate.fact) {
                return Err(IntelligenceError::Evidence);
            }
        }
        fields
            .entry(candidate.fact.field())
            .or_insert_with(|| FieldResolution {
                state: FieldState::Unknown,
                selected: None,
                candidates: Vec::new(),
                has_conflict: false,
            })
            .candidates
            .push(candidate);
    }
    for field in fields.values_mut() {
        // A model cannot suppress a conflicting assertion from the supplied context.
        let kind = field.candidates[0].fact.field();
        if context.evidence.iter().any(|e| {
            e.fact.field() == kind
                && !field
                    .candidates
                    .iter()
                    .any(|c| c.fact == e.fact && c.evidence_ids.contains(&e.id))
        }) {
            return Err(IntelligenceError::Evidence);
        }
        let rank = |candidate: &FactCandidate| {
            candidate
                .evidence_ids
                .iter()
                .filter_map(|id| evidence.get(id))
                .filter(|e| e.retrieved_at <= now && now < e.expires_at)
                .map(|e| e.authority)
                .max()
        };
        let best = field.candidates.iter().filter_map(rank).max();
        let Some(best) = best else { continue };
        let winners: Vec<usize> = field
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| rank(c) == Some(best))
            .map(|(i, _)| i)
            .collect();
        let first = winners[0];
        field.has_conflict = field
            .candidates
            .iter()
            .any(|c| c.fact != field.candidates[first].fact);
        if winners
            .iter()
            .any(|&i| field.candidates[i].fact != field.candidates[first].fact)
        {
            field.state = FieldState::Conflicted;
        } else {
            field.selected = Some(first);
            field.state = if best <= Authority::Provider {
                FieldState::Provisional
            } else {
                FieldState::Supported
            };
        }
    }
    Ok(Assessment { fields })
}
