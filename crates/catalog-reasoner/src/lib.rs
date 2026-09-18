//! Local inference boundary, not yet used by the desktop application.
//! No database writes, model loading, research, identity binding or external HTTP.

use std::{collections::BTreeSet, future::Future, pin::Pin, time::Duration};

use nova_core::catalog::{
    intelligence::{
        self, Assessment, Authority, EntityType, Evidence, FactValue, Policy, Quality,
        ResolutionContext,
    },
    EntityId, SnapshotId, SourceRecordId,
};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

#[cfg(unix)]
pub mod protected;

pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const PROTOCOL: &str = include_str!("../protocol.json");

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReasonerError {
    #[error("invalid reasoning input or configuration")]
    Input,
    #[error("reasoning size limit exceeded")]
    Limit,
    #[error("catalog reasoner is busy")]
    Busy,
    #[error("catalog reasoning deadline exceeded")]
    Timeout,
    #[error("local reasoning transport failed")]
    Transport,
    #[error("local reasoning HTTP status {0}")]
    Http(u16),
    #[error("invalid or incomplete reasoning response")]
    Response,
    #[error("model proposal conflicts with supplied constraints")]
    Conflict,
    #[error("provisional evidence validation failed")]
    Evidence,
}

/// Explicit allowlist. Callers must vet text before supplying it; arbitrary raw
/// SourceRecords, attributes and locator URLs are deliberately not accepted.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: EntityId,
    pub canonical_name: String,
    pub entity_type: EntityType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epg_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDraft {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epg_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_kind: Option<EntityType>,
    pub candidates: Vec<Candidate>,
}

/// Immutable after validation. IDs/constraints never enter the model's messages.
pub struct ResolutionInput {
    record_id: SourceRecordId,
    snapshot_id: SnapshotId,
    draft: InputDraft,
    cannot_link: BTreeSet<EntityId>,
    fingerprint: [u8; 32],
}

fn text_ok(text: &str, limit: usize) -> bool {
    text.len() <= limit && !text.chars().any(char::is_control)
}

fn metadata_ok(epg: &Option<String>, country: &Option<String>, year: Option<u16>) -> bool {
    epg.as_ref()
        .is_none_or(|s| !s.is_empty() && text_ok(s, 256))
        && country
            .as_ref()
            .is_none_or(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase()))
        && year.is_none_or(|y| (1..=9999).contains(&y))
}

impl ResolutionInput {
    pub fn new(
        record_id: SourceRecordId,
        snapshot_id: SnapshotId,
        draft: InputDraft,
        cannot_link: BTreeSet<EntityId>,
    ) -> Result<Self, ReasonerError> {
        if draft.candidates.len() > 16 || cannot_link.len() > 16 {
            return Err(ReasonerError::Limit);
        }
        if !text_ok(&draft.title, 1024)
            || draft.group.as_ref().is_some_and(|s| !text_ok(s, 1024))
            || !metadata_ok(&draft.epg_id, &draft.country, draft.year)
        {
            return Err(ReasonerError::Input);
        }
        let mut seen = BTreeSet::new();
        for c in &draft.candidates {
            if c.canonical_name.is_empty()
                || c.canonical_name.trim() != c.canonical_name
                || !text_ok(&c.canonical_name, 256)
                || c.aliases.len() > 8
                || c.aliases.iter().any(|s| !text_ok(s, 256))
                || c.entity_type == EntityType::Season
                || !metadata_ok(&c.epg_id, &c.country, c.year)
                || !seen.insert(&c.id)
            {
                return Err(ReasonerError::Input);
            }
        }
        if cannot_link.iter().any(|id| !seen.contains(id)) {
            return Err(ReasonerError::Input);
        }
        let bytes = serde_json::to_vec(&draft).map_err(|_| ReasonerError::Input)?;
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(ReasonerError::Limit);
        }
        // Frame all request ownership and constraint inputs, not just the title.
        let framed = serde_json::to_vec(&(&record_id, &snapshot_id, &draft, &cannot_link))
            .map_err(|_| ReasonerError::Input)?;
        let fingerprint = Sha256::digest(&framed).into();
        Ok(Self {
            record_id,
            snapshot_id,
            draft,
            cannot_link,
            fingerprint,
        })
    }
}

/// Caller-declared artifact digests, not runtime attestation. A production runtime
/// owner must verify the loaded process/model before supplying these values.
pub struct DeclaredArtifacts {
    model: String,
    runtime: String,
}

impl DeclaredArtifacts {
    pub fn new(model: String, runtime: String) -> Result<Self, ReasonerError> {
        if [&model, &runtime].iter().any(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) {
            return Err(ReasonerError::Input);
        }
        Ok(Self { model, runtime })
    }
}

pub type ReasoningFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ReasonedRecord, ReasonerError>> + Send + 'a>>;

pub trait CatalogReasoner: Send + Sync {
    fn resolve<'a>(&'a self, input: &'a ResolutionInput) -> ReasoningFuture<'a>;
}

pub struct LocalReasoner {
    client: reqwest::Client,
    endpoint: String,
    deadline: Duration,
    slot: Semaphore,
    artifacts: DeclaredArtifacts,
}

impl LocalReasoner {
    pub fn new(
        port: u16,
        deadline: Duration,
        artifacts: DeclaredArtifacts,
    ) -> Result<Self, ReasonerError> {
        if port < 1024 || deadline.is_zero() || deadline > Duration::from_secs(60) {
            return Err(ReasonerError::Input);
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(deadline)
            .build()
            .map_err(|_| ReasonerError::Transport)?;
        Ok(Self {
            client,
            endpoint: format!("http://127.0.0.1:{port}/v1/chat/completions"),
            deadline,
            slot: Semaphore::new(1),
            artifacts,
        })
    }

    async fn request(&self, input: &ResolutionInput) -> Result<ReasonedRecord, ReasonerError> {
        let protocol: Value = serde_json::from_str(PROTOCOL).map_err(|_| ReasonerError::Input)?;
        let message = serde_json::to_string(&input.draft).map_err(|_| ReasonerError::Input)?;
        let request = json!({"model":"nova-catalog-eval", "temperature":0, "seed":42,
            "max_tokens":512,"stream":false,"chat_template_kwargs":{"enable_thinking":false},
            "response_format":{"type":"json_schema","json_schema":{"name":"catalog","strict":true,"schema":protocol["schema"]}},
            "messages":[{"role":"system","content":protocol["prompt"]},{"role":"user","content":message}]});
        let bytes = serde_json::to_vec(&request).map_err(|_| ReasonerError::Input)?;
        if bytes.len() > 32 * 1024 {
            return Err(ReasonerError::Limit);
        }
        let mut response = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            return Err(ReasonerError::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ReasonerError::Limit);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ReasonerError::Limit);
            }
            bytes.extend_from_slice(&chunk);
        }
        let envelope: Envelope =
            serde_json::from_slice(&bytes).map_err(|_| ReasonerError::Response)?;
        if envelope.choices.len() != 1 {
            return Err(ReasonerError::Response);
        }
        let choice = &envelope.choices[0];
        if choice.finish_reason != "stop" {
            return Err(ReasonerError::Response);
        }
        accept_output(input, &choice.message.content, &self.artifacts)
    }
}

fn accept_output(
    input: &ResolutionInput,
    raw_output: &str,
    artifacts: &DeclaredArtifacts,
) -> Result<ReasonedRecord, ReasonerError> {
    if raw_output.len() > MAX_RESPONSE_BYTES {
        return Err(ReasonerError::Limit);
    }
    let output: Output = serde_json::from_str(raw_output).map_err(|_| ReasonerError::Response)?;
    let mut facts = Vec::new();
    match (
        &output.candidate_id,
        &output.canonical_name,
        output.entity_type,
    ) {
        (None, None, None) => {}
        (Some(id), Some(name), Some(kind)) => {
            let c = input
                .draft
                .candidates
                .iter()
                .find(|c| &c.id == id)
                .ok_or(ReasonerError::Response)?;
            if &c.canonical_name != name || c.entity_type != kind {
                return Err(ReasonerError::Response);
            }
            if input.cannot_link.contains(id)
                || contradicts(&input.draft.epg_id, &c.epg_id)
                || contradicts(&input.draft.country, &c.country)
                || contradicts(&input.draft.year, &c.year)
                || contradicts(&input.draft.season, &c.season)
                || contradicts(&input.draft.episode, &c.episode)
                || input.draft.content_kind.is_some_and(|k| k != kind)
            {
                return Err(ReasonerError::Conflict);
            }
            facts.push(FactValue::CanonicalName(name.clone()));
            facts.push(FactValue::EntityType(kind));
        }
        _ => return Err(ReasonerError::Response),
    }
    if let Some(quality) = output.quality_hint {
        facts.push(FactValue::QualityHint(quality));
    }
    Ok(ReasonedRecord {
        record_id: input.record_id.clone(),
        snapshot_id: input.snapshot_id.clone(),
        input_fingerprint: input.fingerprint,
        protocol_fingerprint: Sha256::digest(PROTOCOL.as_bytes()).into(),
        model: artifacts.model.clone(),
        runtime: artifacts.runtime.clone(),
        proposed_entity: output.candidate_id,
        facts,
        raw_output: raw_output.to_owned(),
    })
}

fn transport_error(error: reqwest::Error) -> ReasonerError {
    if error.is_timeout() {
        ReasonerError::Timeout
    } else {
        ReasonerError::Transport
    }
}

fn contradicts<T: PartialEq>(left: &Option<T>, right: &Option<T>) -> bool {
    matches!((left, right), (Some(a), Some(b)) if a != b)
}

impl CatalogReasoner for LocalReasoner {
    fn resolve<'a>(&'a self, input: &'a ResolutionInput) -> ReasoningFuture<'a> {
        Box::pin(async move {
            let _permit = self.slot.try_acquire().map_err(|_| ReasonerError::Busy)?;
            tokio::time::timeout(self.deadline, self.request(input))
                .await
                .map_err(|_| ReasonerError::Timeout)?
        })
    }
}

#[derive(Deserialize)]
struct Envelope {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    finish_reason: String,
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    content: String,
}

fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}

fn unknown<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let s = String::deserialize(d)?;
    if s == "unknown" {
        Ok(None)
    } else {
        serde_json::from_value(Value::String(s))
            .map(Some)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    #[serde(deserialize_with = "nullable")]
    canonical_name: Option<String>,
    #[serde(deserialize_with = "nullable")]
    candidate_id: Option<EntityId>,
    #[serde(deserialize_with = "unknown")]
    entity_type: Option<EntityType>,
    #[serde(deserialize_with = "unknown")]
    quality_hint: Option<Quality>,
}

/// A suggested candidate and untrusted facts, never a binding or canonical entity.
/// No Debug/Serialize implementation: callers explicitly access scoped results.
pub struct ReasonedRecord {
    record_id: SourceRecordId,
    snapshot_id: SnapshotId,
    input_fingerprint: [u8; 32],
    protocol_fingerprint: [u8; 32],
    model: String,
    runtime: String,
    proposed_entity: Option<EntityId>,
    facts: Vec<FactValue>,
    raw_output: String,
}

impl ReasonedRecord {
    pub fn proposed_entity(&self) -> Option<&EntityId> {
        self.proposed_entity.as_ref()
    }
    pub fn input_fingerprint(&self) -> &[u8; 32] {
        &self.input_fingerprint
    }
    pub fn protocol_fingerprint(&self) -> &[u8; 32] {
        &self.protocol_fingerprint
    }
    pub fn declared_model(&self) -> &str {
        &self.model
    }
    pub fn declared_runtime(&self) -> &str {
        &self.runtime
    }

    /// Caller must persist a protected derivation at `provenance`, including this
    /// record's fingerprints/artifact declarations, before publishing its facts.
    pub fn provisional(
        &self,
        provenance: SourceRecordId,
        now: u64,
        ttl_seconds: u64,
    ) -> Result<InferenceBundle, ReasonerError> {
        if ttl_seconds == 0 || ttl_seconds > 86400 {
            return Err(ReasonerError::Input);
        }
        let expiry = now.checked_add(ttl_seconds).ok_or(ReasonerError::Input)?;
        let mut context = ResolutionContext {
            record_id: self.record_id.clone(),
            snapshot_id: self.snapshot_id.clone(),
            evidence: Vec::new(),
        };
        let mut candidates = Vec::new();
        for (index, fact) in self.facts.iter().enumerate() {
            let id = index as u16;
            context.evidence.push(Evidence {
                id,
                fact: fact.clone(),
                authority: Authority::Inference,
                retrieved_at: now,
                expires_at: expiry,
                provenance: provenance.clone(),
            });
            candidates.push(json!({"fact":fact,"confidence":0,"evidence_ids":[id]}));
        }
        let proposal = serde_json::to_vec(&json!({"version":intelligence::PROTOCOL_VERSION,
            "record_id":self.record_id,"snapshot_id":self.snapshot_id,"candidates":candidates}))
        .map_err(|_| ReasonerError::Evidence)?;
        Ok(InferenceBundle { context, proposal })
    }
}

pub struct InferenceBundle {
    context: ResolutionContext,
    proposal: Vec<u8>,
}
impl InferenceBundle {
    pub fn context(&self) -> &ResolutionContext {
        &self.context
    }
    pub fn assess(&self, policy: &Policy, now: u64) -> Result<Assessment, ReasonerError> {
        intelligence::assess(&self.proposal, &self.context, policy, now)
            .map_err(|_| ReasonerError::Evidence)
    }
}
