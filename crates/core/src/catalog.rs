//! Additive canonical-catalog contracts. Movies are served through canonical
//! entities and stream variants (`media`); other kinds still use legacy rows.
//!
//! Source evidence is never a canonical identity. IDs are allocated by a future
//! identity/persistence owner, not generated from titles or provider URLs.
//!
//! ```compile_fail
//! use nova_core::catalog::{EntityId, VariantId};
//! let entity: EntityId = VariantId::new("variant-1").unwrap();
//! ```

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

pub mod blocking;
pub mod fingerprint;
pub mod intelligence;
pub mod media;
pub mod reconcile;
pub mod resolution;
pub mod snapshot;

pub const MAX_FIELD_BYTES: usize = 16 * 1024;
pub const MAX_RECORD_BYTES: usize = 128 * 1024;
pub const MAX_ATTRIBUTES: usize = 64;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogInputError {
    #[error("invalid opaque identifier: {kind}")]
    InvalidId { kind: &'static str },
    #[error("missing required field: {field}")]
    EmptyField { field: &'static str },
    #[error("field {field} exceeds {limit} bytes")]
    FieldTooLarge { field: &'static str, limit: usize },
    #[error("source record exceeds {limit} bytes")]
    RecordTooLarge { limit: usize },
    #[error("source record exceeds {limit} attributes")]
    TooManyAttributes { limit: usize },
}

macro_rules! opaque_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CatalogInputError> {
                let value = value.into();
                if value.is_empty() || value.len() > 128 || !value.bytes().all(|b|
                    b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    return Err(CatalogInputError::InvalidId { kind: stringify!($name) });
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str { &self.0 }
        }

        impl TryFrom<String> for $name {
            type Error = CatalogInputError;
            fn try_from(value: String) -> Result<Self, Self::Error> { Self::new(value) }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self { value.0 }
        }
    )+};
}

opaque_id!(
    SourceId,
    SnapshotId,
    EndpointScope,
    SourceRecordId,
    EntityId,
    EditionId,
    VariantId,
    SourceBindingId,
    StreamReference
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    LiveChannel,
    Movie,
    Series,
    Season,
    Episode,
    Event,
    Radio,
    #[default]
    Unknown,
}

/// Provider key must be scoped by source AND namespace/kind. Never a Nova ID.
#[derive(PartialEq, Eq)]
pub struct UpstreamKey {
    pub namespace: String,
    pub value: String,
}

// Provider identifiers may contain sensitive material. Do not derive raw Debug.
impl fmt::Debug for UpstreamKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UpstreamKey([redacted])")
    }
}

/// Untrusted adapter input. No Serialize/Debug: not an IPC or diagnostics DTO.
/// Adapters must bound bytes while reading, before allocating this draft.
pub struct SourceRecordDraft {
    pub id: SourceRecordId,
    pub source_id: SourceId,
    pub snapshot_id: SnapshotId,
    pub upstream: Option<UpstreamKey>,
    pub kind_hint: ContentKind,
    pub raw_title: String,
    pub raw_group: Option<String>,
    pub raw_logo: Option<String>,
    pub raw_epg_id: Option<String>,
    /// Opaque reference to a protected locator store; never an upstream URL.
    pub stream_ref: StreamReference,
    pub attributes: BTreeMap<String, String>,
}

/// Immutable, bounded source evidence. Cleanup belongs to derived compiler facts.
pub struct SourceRecord(SourceRecordDraft);

impl TryFrom<SourceRecordDraft> for SourceRecord {
    type Error = CatalogInputError;

    fn try_from(draft: SourceRecordDraft) -> Result<Self, Self::Error> {
        if draft.attributes.len() > MAX_ATTRIBUTES {
            return Err(CatalogInputError::TooManyAttributes {
                limit: MAX_ATTRIBUTES,
            });
        }
        let mut total = 0;
        let mut field = |name: &'static str, value: &str| -> Result<(), CatalogInputError> {
            if value.len() > MAX_FIELD_BYTES {
                return Err(CatalogInputError::FieldTooLarge {
                    field: name,
                    limit: MAX_FIELD_BYTES,
                });
            }
            total += value.len();
            Ok(())
        };
        field("raw_title", &draft.raw_title)?;
        for (name, value) in [
            ("raw_group", &draft.raw_group),
            ("raw_logo", &draft.raw_logo),
            ("raw_epg_id", &draft.raw_epg_id),
        ] {
            if let Some(value) = value {
                field(name, value)?;
            }
        }
        if let Some(key) = &draft.upstream {
            for (name, value) in [
                ("upstream.namespace", &key.namespace),
                ("upstream.value", &key.value),
            ] {
                if value.trim().is_empty() {
                    return Err(CatalogInputError::EmptyField { field: name });
                }
                field(name, value)?;
            }
        }
        for (key, value) in &draft.attributes {
            if key.trim().is_empty() {
                return Err(CatalogInputError::EmptyField {
                    field: "attribute.key",
                });
            }
            field("attribute.key", key)?;
            field("attribute.value", value)?;
        }
        if total > MAX_RECORD_BYTES {
            return Err(CatalogInputError::RecordTooLarge {
                limit: MAX_RECORD_BYTES,
            });
        }
        Ok(Self(draft))
    }
}

impl SourceRecord {
    pub fn id(&self) -> &SourceRecordId {
        &self.0.id
    }
    pub fn source_id(&self) -> &SourceId {
        &self.0.source_id
    }
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.0.snapshot_id
    }
    pub fn upstream(&self) -> Option<&UpstreamKey> {
        self.0.upstream.as_ref()
    }
    pub fn kind_hint(&self) -> ContentKind {
        self.0.kind_hint
    }
    pub fn raw_title(&self) -> &str {
        &self.0.raw_title
    }
    pub fn raw_group(&self) -> Option<&str> {
        self.0.raw_group.as_deref()
    }
    pub fn raw_logo(&self) -> Option<&str> {
        self.0.raw_logo.as_deref()
    }
    pub fn raw_epg_id(&self) -> Option<&str> {
        self.0.raw_epg_id.as_deref()
    }
    pub fn stream_ref(&self) -> &StreamReference {
        &self.0.stream_ref
    }
    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.0.attributes
    }
}

impl fmt::Debug for SourceRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceRecord")
            .field("id", self.id())
            .field("source_id", self.source_id())
            .field("snapshot_id", self.snapshot_id())
            .field("kind_hint", &self.kind_hint())
            .field("attribute_count", &self.attributes().len())
            .finish_non_exhaustive()
    }
}

/// Resolved generation view; persistence must keep membership generation-scoped.
/// Technical claims/observations are added with their provenance in a later slice.
#[derive(Debug, PartialEq, Eq)]
pub struct StreamVariant {
    pub id: VariantId,
    pub entity_id: EntityId,
    pub edition_id: Option<EditionId>,
    pub binding_id: SourceBindingId,
}

/// Unknown numbering is explicit; season zero is a valid specials convention.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeNumbering {
    pub season: Option<u32>,
    pub episode: Option<u32>,
}
