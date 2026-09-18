//! Exact-request derivation cache. The caller owns the artifact index and key lifecycle.
//! Use a dedicated quota-limited store namespace, separate from raw source archives.
use super::*;
use nova_core::catalog::{snapshot::*, EndpointScope, SourceId};
use nova_storage::snapshots::{stream::StreamingSnapshotStore, SnapshotStoreError};
use std::{
    num::{NonZeroU32, NonZeroU64},
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;
use zeroize::Zeroizing;

const SCOPE: &str = "catalog-derivation-v1";
const MAX_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("invalid protected derivation")]
    Invalid,
    #[error("derivation scope does not match")]
    Scope,
    #[error("protected derivation storage failed: {0}")]
    Storage(#[from] SnapshotStoreError),
    #[error("derivation reasoning validation failed: {0}")]
    Reasoning(#[from] ReasonerError),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    // Exact framed input includes scope, candidate metadata and cannot-link constraints.
    context: String,
    input_digest: [u8; 32],
    protocol_digest: [u8; 32],
    protocol: String,
    model: String,
    runtime: String,
    created: u64,
    expires: u64,
    output: String,
}

fn context(input: &ResolutionInput) -> Result<Zeroizing<String>, CacheError> {
    serde_json::to_string(&(
        &input.record_id,
        &input.snapshot_id,
        &input.draft,
        &input.cannot_link,
    ))
    .map(Zeroizing::new)
    .map_err(|_| CacheError::Invalid)
}

/// Only constructed after a successful encrypted commit or fully authenticated replay.
/// No raw result getter: cache reuse cannot reset the original evidence timestamps.
pub struct CachedDerivation {
    record: ReasonedRecord,
    provenance: SourceRecordId,
    created: u64,
    expires: u64,
}
impl CachedDerivation {
    pub fn proposed_entity(&self) -> Option<&EntityId> {
        self.record.proposed_entity()
    }
    pub fn provenance(&self) -> &SourceRecordId {
        &self.provenance
    }
    pub fn expires_at(&self) -> u64 {
        self.expires
    }
    pub fn assess(&self, policy: &Policy, now: u64) -> Result<Assessment, CacheError> {
        Ok(self
            .record
            .provisional(
                self.provenance.clone(),
                self.created,
                self.expires - self.created,
            )?
            .assess(policy, now)?)
    }
}

pub struct ProtectedDerivations {
    store: StreamingSnapshotStore,
}

pub struct DerivationWrite {
    pub artifact: SnapshotId,
    pub source: SourceId,
    pub created_at: u64,
    pub ttl_seconds: u64,
}
impl ProtectedDerivations {
    /// Store profile/key/quotas are configured by its trusted owner, never by model output.
    pub fn new(store: StreamingSnapshotStore) -> Self {
        Self { store }
    }

    /// Cache-first resolution using the caller's artifact index. `created_at` is
    /// the request-start timestamp: inference latency consumes, never extends, TTL.
    /// Cache hits need no runtime connection. Cache corruption stops this operation.
    /// A miss runs inference and returns only after protected persistence succeeds.
    pub async fn resolve(
        &self,
        reasoner: &LocalReasoner,
        input: &ResolutionInput,
        previous: Option<SnapshotId>,
        write: DerivationWrite,
    ) -> Result<CachedDerivation, CacheError> {
        if let Some(previous) = previous {
            if let Some(hit) = self
                .load(
                    previous,
                    &write.source,
                    input,
                    &reasoner.artifacts,
                    write.created_at,
                )
                .await?
            {
                return Ok(hit);
            }
        }
        let result = reasoner.resolve(input).await?;
        self.save(input, &result, write).await
    }

    /// Immutable no-overwrite commit. Caller allocates a fresh artifact ID, then indexes
    /// it only after success. An interrupted index update may leave a quota-counted orphan.
    pub async fn save(
        &self,
        input: &ResolutionInput,
        result: &ReasonedRecord,
        write: DerivationWrite,
    ) -> Result<CachedDerivation, CacheError> {
        let DerivationWrite {
            artifact,
            source,
            created_at: created,
            ttl_seconds: ttl,
        } = write;
        if artifact == input.snapshot_id || ttl == 0 || ttl > 86400 {
            return Err(CacheError::Invalid);
        }
        let expires = created.checked_add(ttl).ok_or(CacheError::Invalid)?;
        let protocol_digest: [u8; 32] = Sha256::digest(PROTOCOL.as_bytes()).into();
        if result.input_fingerprint != input.fingerprint
            || result.protocol_fingerprint != protocol_digest
        {
            return Err(CacheError::Scope);
        }
        let artifacts = DeclaredArtifacts::new(result.model.clone(), result.runtime.clone())?;
        let record = accept_output(input, &result.raw_output, &artifacts)?;
        let wire = Wire {
            version: 1,
            context: context(input)?.to_string(),
            input_digest: input.fingerprint,
            protocol_digest,
            protocol: PROTOCOL.to_owned(),
            model: result.model.clone(),
            runtime: result.runtime.clone(),
            created,
            expires,
            output: result.raw_output.clone(),
        };
        let bytes = Zeroizing::new(serde_json::to_vec(&wire).map_err(|_| CacheError::Invalid)?);
        if bytes.len() > MAX_BYTES {
            return Err(CacheError::Invalid);
        }
        let descriptor = SnapshotDescriptor {
            id: artifact.clone(),
            source_id: source,
            scope: EndpointScope::new(SCOPE).map_err(|_| CacheError::Invalid)?,
            parser_revision: NonZeroU32::new(1).ok_or(CacheError::Invalid)?,
        };
        let mut capture = SnapshotCapture::new(
            descriptor.clone(),
            SnapshotLimits {
                max_bytes: NonZeroU64::new(MAX_BYTES as u64).ok_or(CacheError::Invalid)?,
                max_records: NonZeroU64::new(1).ok_or(CacheError::Invalid)?,
            },
        );
        capture
            .push_bytes(&bytes)
            .map_err(|_| CacheError::Invalid)?;
        let receipt = capture
            .seal(Completeness::Complete)
            .map_err(|_| CacheError::Invalid)?;
        let mut writer = self.store.begin(descriptor).await?;
        writer.append(&bytes).await?;
        writer.seal(receipt).await?;
        Ok(CachedDerivation {
            record,
            provenance: SourceRecordId::new(artifact.as_str()).map_err(|_| CacheError::Invalid)?,
            created,
            expires,
        })
    }

    /// Missing, expired or changed requests/artifacts are misses. Authentication,
    /// malformed archive and source-ownership failures remain errors, never silent misses.
    pub async fn load(
        &self,
        artifact: SnapshotId,
        source: &SourceId,
        input: &ResolutionInput,
        artifacts: &DeclaredArtifacts,
        now: u64,
    ) -> Result<Option<CachedDerivation>, CacheError> {
        let mut sink = BoundedSink(Zeroizing::new(Vec::new()));
        let receipt = match self.store.replay_into(artifact.clone(), &mut sink).await {
            Err(SnapshotStoreError::NotFound) => return Ok(None),
            other => other?,
        };
        let descriptor = receipt.descriptor();
        if &descriptor.source_id != source
            || descriptor.scope.as_str() != SCOPE
            || descriptor.parser_revision.get() != 1
            || receipt.completeness() != Completeness::Complete
            || receipt.record_count() != 0
        {
            return Err(CacheError::Scope);
        }
        let wire: Wire = serde_json::from_slice(&sink.0).map_err(|_| CacheError::Invalid)?;
        if wire.version != 1
            || wire.expires <= wire.created
            || wire.expires - wire.created > 86400
            || wire.input_digest != <[u8; 32]>::from(Sha256::digest(wire.context.as_bytes()))
            || wire.protocol_digest != <[u8; 32]>::from(Sha256::digest(wire.protocol.as_bytes()))
        {
            return Err(CacheError::Invalid);
        }
        let protocol_digest: [u8; 32] = Sha256::digest(PROTOCOL.as_bytes()).into();
        if wire.created > now
            || wire.expires <= now
            || wire.input_digest != input.fingerprint
            || wire.context != *context(input)?
            || wire.protocol_digest != protocol_digest
            || wire.model != artifacts.model
            || wire.runtime != artifacts.runtime
        {
            return Ok(None);
        }
        let record = accept_output(input, &wire.output, artifacts)?;
        Ok(Some(CachedDerivation {
            record,
            provenance: SourceRecordId::new(artifact.as_str()).map_err(|_| CacheError::Invalid)?,
            created: wire.created,
            expires: wire.expires,
        }))
    }
}

struct BoundedSink(Zeroizing<Vec<u8>>);
impl AsyncWrite for BoundedSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if bytes.len() > MAX_BYTES.saturating_sub(self.0.len()) {
            return Poll::Ready(Err(std::io::Error::other("derivation size limit exceeded")));
        }
        self.0.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::catalog::intelligence::FieldState;
    use nova_storage::snapshots::{stream::StreamLimits, ProfileKey};
    use tempfile::TempDir;
    use uuid::Uuid;

    fn input(title: &str, denied: bool) -> ResolutionInput {
        let draft = serde_json::from_value(json!({"title":title,"candidates":[{"id":"channel","canonical_name":"Synthetic Channel","entity_type":"live_channel"}]})).unwrap();
        ResolutionInput::new(
            SourceRecordId::new("record").unwrap(),
            SnapshotId::new("raw").unwrap(),
            draft,
            if denied {
                BTreeSet::from([EntityId::new("channel").unwrap()])
            } else {
                BTreeSet::new()
            },
        )
        .unwrap()
    }
    fn artifacts() -> DeclaredArtifacts {
        DeclaredArtifacts::new("a".repeat(64), "b".repeat(64)).unwrap()
    }
    fn result(input: &ResolutionInput) -> ReasonedRecord {
        accept_output(input, r#"{"canonical_name":"Synthetic Channel","candidate_id":"channel","entity_type":"live_channel","quality_hint":"hd"}"#, &artifacts()).unwrap()
    }
    fn source() -> SourceId {
        SourceId::new("source").unwrap()
    }
    fn id() -> SnapshotId {
        SnapshotId::new("derivation").unwrap()
    }
    fn write() -> DerivationWrite {
        DerivationWrite {
            artifact: id(),
            source: source(),
            created_at: 100,
            ttl_seconds: 60,
        }
    }
    async fn store(dir: &TempDir, profile: Uuid, key: u8, max_files: u32) -> ProtectedDerivations {
        ProtectedDerivations::new(
            StreamingSnapshotStore::open(
                dir.path().to_owned(),
                profile,
                ProfileKey::new([key; 32]),
                StreamLimits {
                    max_payload_bytes: MAX_BYTES as u64,
                    max_stored_bytes: 1024 * 1024,
                    max_files,
                },
            )
            .await
            .unwrap(),
        )
    }
    fn policy() -> Policy {
        Policy {
            countries: BTreeSet::new(),
            languages: BTreeSet::new(),
        }
    }

    #[tokio::test]
    async fn encrypted_reopen_preserves_provenance_and_original_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let profile = Uuid::new_v4();
        let cache = store(&dir, profile, 7, 8).await;
        let input = input("PRIVATE synthetic title", false);
        let saved = cache.save(&input, &result(&input), write()).await.unwrap();
        assert_eq!(saved.provenance().as_str(), "derivation");
        drop(cache);
        let bytes = tokio::fs::read(
            dir.path()
                .join(profile.to_string())
                .join("streams-v2/derivation.nvs2"),
        )
        .await
        .unwrap();
        assert!(!bytes.windows(7).any(|w| w == b"PRIVATE"));
        assert!(!bytes.windows(17).any(|w| w == b"Synthetic Channel"));
        let cache = store(&dir, profile, 7, 8).await;
        let hit = cache
            .load(id(), &source(), &input, &artifacts(), 159)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hit.expires_at(), 160);
        assert_eq!(hit.proposed_entity().unwrap().as_str(), "channel");
        assert!(hit
            .assess(&policy(), 159)
            .unwrap()
            .fields
            .values()
            .all(|f| f.state == FieldState::Provisional));
        assert!(hit
            .assess(&policy(), 160)
            .unwrap()
            .fields
            .values()
            .all(|f| f.state == FieldState::Unknown));
        assert!(cache
            .load(id(), &source(), &input, &artifacts(), 160)
            .await
            .unwrap()
            .is_none());
        assert!(cache
            .load(id(), &source(), &input, &artifacts(), 99)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn changed_context_constraints_artifacts_and_absence_are_misses() {
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 8).await;
        let original = input("title", false);
        assert!(cache
            .load(id(), &source(), &original, &artifacts(), 101)
            .await
            .unwrap()
            .is_none());
        cache
            .save(&original, &result(&original), write())
            .await
            .unwrap();
        for changed in [input("other", false), input("title", true)] {
            assert!(cache
                .load(id(), &source(), &changed, &artifacts(), 101)
                .await
                .unwrap()
                .is_none());
        }
        let mut changed = input("title", false);
        changed.snapshot_id = SnapshotId::new("other-snapshot").unwrap();
        // Scope comparison checks the exact framed context even if a caller-internal
        // bug leaves an old fingerprint on an input (public inputs are immutable).
        assert!(cache
            .load(id(), &source(), &changed, &artifacts(), 101)
            .await
            .unwrap()
            .is_none());
        for declared in [
            DeclaredArtifacts::new("c".repeat(64), "b".repeat(64)).unwrap(),
            DeclaredArtifacts::new("a".repeat(64), "c".repeat(64)).unwrap(),
        ] {
            assert!(cache
                .load(id(), &source(), &original, &declared, 101)
                .await
                .unwrap()
                .is_none());
        }
        assert!(matches!(
            cache
                .load(
                    id(),
                    &SourceId::new("other").unwrap(),
                    &original,
                    &artifacts(),
                    101
                )
                .await,
            Err(CacheError::Scope)
        ));
    }

    #[tokio::test]
    async fn corruption_and_wrong_key_never_return_a_hit() {
        let dir = tempfile::tempdir().unwrap();
        let profile = Uuid::new_v4();
        let cache = store(&dir, profile, 7, 8).await;
        let input = input("title", false);
        cache.save(&input, &result(&input), write()).await.unwrap();
        let wrong = store(&dir, profile, 8, 8).await;
        assert!(matches!(
            wrong.load(id(), &source(), &input, &artifacts(), 101).await,
            Err(CacheError::Storage(SnapshotStoreError::Integrity))
        ));
        let path = dir
            .path()
            .join(profile.to_string())
            .join("streams-v2/derivation.nvs2");
        let mut bytes = tokio::fs::read(&path).await.unwrap();
        bytes.pop();
        tokio::fs::write(path, bytes).await.unwrap();
        assert!(cache
            .load(id(), &source(), &input, &artifacts(), 101)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn scope_overwrite_ttl_and_quota_fail_without_usable_new_result() {
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 1).await;
        let original = input("title", false);
        let result = result(&original);
        assert!(matches!(
            cache.save(&input("other", false), &result, write()).await,
            Err(CacheError::Scope)
        ));
        for ttl in [0, 86401] {
            let mut w = write();
            w.ttl_seconds = ttl;
            assert!(cache.save(&original, &result, w).await.is_err());
        }
        let mut w = write();
        w.created_at = u64::MAX;
        assert!(cache.save(&original, &result, w).await.is_err());
        cache.save(&original, &result, write()).await.unwrap();
        assert!(cache.save(&original, &result, write()).await.is_err());
        let mut w = write();
        w.artifact = SnapshotId::new("another").unwrap();
        assert!(matches!(
            cache.save(&original, &result, w).await,
            Err(CacheError::Storage(SnapshotStoreError::Quota))
        ));
        assert!(cache
            .load(id(), &source(), &original, &artifacts(), 101)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn abstention_is_reusable_without_inventing_an_identity() {
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 8).await;
        let input = input("title", false);
        let result = accept_output(&input, r#"{"canonical_name":null,"candidate_id":null,"entity_type":"unknown","quality_hint":"unknown"}"#, &artifacts()).unwrap();
        cache.save(&input, &result, write()).await.unwrap();
        let hit = cache
            .load(id(), &source(), &input, &artifacts(), 101)
            .await
            .unwrap()
            .unwrap();
        assert!(hit.proposed_entity().is_none());
        assert!(hit.assess(&policy(), 101).unwrap().fields.is_empty());
    }

    #[tokio::test]
    async fn replay_sink_stops_before_unbounded_allocation() {
        use tokio::io::AsyncWriteExt;
        let mut sink = BoundedSink(Zeroizing::new(Vec::new()));
        sink.write_all(&vec![0; MAX_BYTES]).await.unwrap();
        assert!(sink.write_all(b"x").await.is_err());
        assert_eq!(sink.0.len(), MAX_BYTES);
    }

    #[tokio::test]
    async fn cache_first_hit_needs_no_runtime_and_does_not_extend_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 8).await;
        let input = input("title", false);
        cache.save(&input, &result(&input), write()).await.unwrap();
        // No HTTP handler: any unintended inference request times out.
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let reasoner = LocalReasoner::new(
            socket.local_addr().unwrap().port(),
            Duration::from_millis(100),
            artifacts(),
        )
        .unwrap();
        let mut w = write();
        w.created_at = 159;
        w.ttl_seconds = 86400;
        w.artifact = SnapshotId::new("unused").unwrap();
        let hit = cache
            .resolve(&reasoner, &input, Some(id()), w)
            .await
            .unwrap();
        assert_eq!(hit.expires_at(), 160);
        assert_eq!(hit.provenance().as_str(), "derivation");
        assert!(cache
            .load(
                SnapshotId::new("unused").unwrap(),
                &source(),
                &input,
                &artifacts(),
                159
            )
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn cache_miss_commits_before_return_and_quota_failure_returns_no_derivation() {
        use axum::{routing::post, Json, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let reasoner = LocalReasoner::new(
            listener.local_addr().unwrap().port(),
            Duration::from_secs(1),
            artifacts(),
        )
        .unwrap();
        let app = Router::new().route("/v1/chat/completions", post(|| async {
            Json(json!({"choices":[{"finish_reason":"stop","message":{"content":
                r#"{"canonical_name":"Synthetic Channel","candidate_id":"channel","entity_type":"live_channel","quality_hint":"hd"}"#}}]}))
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 1).await;
        let input = input("title", false);
        let saved = cache
            .resolve(&reasoner, &input, None, write())
            .await
            .unwrap();
        assert_eq!(saved.provenance().as_str(), "derivation");
        assert!(cache
            .load(id(), &source(), &input, &artifacts(), 101)
            .await
            .unwrap()
            .is_some());
        let mut w = write();
        w.artifact = SnapshotId::new("quota-rejected").unwrap();
        assert!(matches!(
            cache.resolve(&reasoner, &input, None, w).await,
            Err(CacheError::Storage(SnapshotStoreError::Quota))
        ));
        server.abort();
    }

    #[tokio::test]
    async fn authenticated_payload_still_requires_semantic_validation() {
        let dir = tempfile::tempdir().unwrap();
        let cache = store(&dir, Uuid::new_v4(), 7, 8).await;
        let input = input("title", false);
        cache.save(&input, &result(&input), write()).await.unwrap();
        let mut sink = BoundedSink(Zeroizing::new(Vec::new()));
        cache.store.replay_into(id(), &mut sink).await.unwrap();
        let original: Value = serde_json::from_slice(&sink.0).unwrap();
        for case in 0..4 {
            let mut wire = original.clone();
            match case {
                0 => {
                    wire["protocol"] = json!("older protocol");
                    wire["protocol_digest"] =
                        json!(<[u8; 32]>::from(Sha256::digest(b"older protocol")));
                }
                1 => wire["output"] = json!(r#"{"authority":"official"}"#),
                2 => wire["context"] = json!("different context"),
                _ => wire["version"] = json!(999),
            }
            let descriptor = SnapshotDescriptor {
                id: SnapshotId::new(format!("modified-{case}")).unwrap(),
                source_id: source(),
                scope: EndpointScope::new(SCOPE).unwrap(),
                parser_revision: NonZeroU32::new(1).unwrap(),
            };
            let bytes = serde_json::to_vec(&wire).unwrap();
            let mut capture = SnapshotCapture::new(
                descriptor.clone(),
                SnapshotLimits {
                    max_bytes: NonZeroU64::new(MAX_BYTES as u64).unwrap(),
                    max_records: NonZeroU64::new(1).unwrap(),
                },
            );
            capture.push_bytes(&bytes).unwrap();
            let mut writer = cache.store.begin(descriptor.clone()).await.unwrap();
            writer.append(&bytes).await.unwrap();
            writer
                .seal(capture.seal(Completeness::Complete).unwrap())
                .await
                .unwrap();
            let replay = cache
                .load(descriptor.id, &source(), &input, &artifacts(), 101)
                .await;
            if case == 0 {
                assert!(replay.unwrap().is_none());
            } else {
                assert!(replay.is_err());
            }
        }
    }
}
