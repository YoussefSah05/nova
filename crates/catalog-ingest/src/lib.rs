//! Unconnected catalog ingestion primitives. Raw objects are protected evidence,
//! not IPC DTOs. No network requests, canonical grouping or locator resolution.
use nova_core::catalog::*;
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};
use zeroize::Zeroizing;

#[cfg(unix)]
pub mod stage;

pub const MAX_RAW_OBJECT_BYTES: usize = 128 * 1024;
pub const ADAPTER_REVISION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid or ambiguous provider object")]
    Object,
    #[error("record evidence limit exceeded")]
    Limit,
    #[error("invalid source-record fields")]
    Record,
    #[error("record belongs to another source or snapshot")]
    Ownership,
    #[error("duplicate source-record occurrence ID")]
    DuplicateRecord,
    #[error("record stage is terminal")]
    Terminal,
    #[error("invalid protected record archive")]
    Archive,
    #[cfg(unix)]
    #[error("protected record storage failed: {0}")]
    Storage(#[from] nova_storage::snapshots::SnapshotStoreError),
}
pub type Result<T> = std::result::Result<T, IngestError>;

/// IDs and protected locator references belong to the caller's persistence layer.
/// Occurrence IDs are not upstream IDs and must not be used as canonical identities.
pub struct RecordContext {
    pub id: SourceRecordId,
    pub source_id: SourceId,
    pub snapshot_id: SnapshotId,
    pub stream_ref: StreamReference,
}
#[derive(Clone, Copy)]
pub enum XtreamScope {
    Live,
    Vod,
    Series,
}

/// No Debug/Serialize: raw bytes may contain credentials. The original whole
/// response must also be retained; these bytes cover exactly one JSON object.
pub struct EvidenceRecord {
    record: SourceRecord,
    raw: Zeroizing<Vec<u8>>,
}
impl EvidenceRecord {
    pub fn record(&self) -> &SourceRecord {
        &self.record
    }
    pub fn raw_object(&self) -> &[u8] {
        &self.raw
    }
}

struct Object(BTreeMap<String, Value>);
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("one bounded JSON object")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Object, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    if fields.len() >= 256 || fields.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("ambiguous or oversized object"));
                    }
                }
                Ok(Object(fields))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
fn object(raw: &[u8]) -> Result<BTreeMap<String, Value>> {
    if raw.len() > MAX_RAW_OBJECT_BYTES {
        return Err(IngestError::Limit);
    }
    // serde_json's default recursion limit stays enabled. Errors deliberately do
    // not carry its text, because malformed provider data can contain secrets.
    serde_json::from_slice::<Object>(raw)
        .map(|o| o.0)
        .map_err(|_| IngestError::Object)
}
fn text(fields: &BTreeMap<String, Value>, name: &str) -> Result<Option<String>> {
    match fields.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(IngestError::Object),
    }
}

/// Pure per-object adapter. Network array framing and byte-offset provenance are
/// separate gates. Unknown/nested fields stay byte-exact in raw evidence rather
/// than being promoted into metadata or transport capabilities.
pub fn parse_xtream_object(
    scope: XtreamScope,
    context: RecordContext,
    raw: &[u8],
) -> Result<EvidenceRecord> {
    let fields = object(raw)?;
    let (kind, id_field, namespace, logo) = match scope {
        XtreamScope::Live => (
            ContentKind::LiveChannel,
            "stream_id",
            "xtream-live",
            "stream_icon",
        ),
        XtreamScope::Vod => (ContentKind::Movie, "stream_id", "xtream-vod", "stream_icon"),
        XtreamScope::Series => (ContentKind::Series, "series_id", "xtream-series", "cover"),
    };
    let upstream = match fields.get(id_field) {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.as_u64().ok_or(IngestError::Object)?.to_string()),
        _ => return Err(IngestError::Object),
    }
    .map(|value| UpstreamKey {
        namespace: namespace.into(),
        value,
    });
    let mut attributes = BTreeMap::new();
    // Preserve JSON scalar types in the cache. Unrecognized keys (which may be
    // transport credentials) are available only through protected raw evidence.
    for name in ["category_id", "container_extension", "year", "rating"] {
        if let Some(value) = fields.get(name) {
            if !value.is_null() && !value.is_array() && !value.is_object() {
                attributes.insert(name.into(), value.to_string());
            }
        }
    }
    let record = SourceRecord::try_from(SourceRecordDraft {
        id: context.id,
        source_id: context.source_id,
        snapshot_id: context.snapshot_id,
        upstream,
        kind_hint: kind,
        raw_title: text(&fields, "name")?.unwrap_or_default(),
        raw_group: text(&fields, "category_name")?,
        raw_logo: text(&fields, logo)?,
        raw_epg_id: text(&fields, "epg_channel_id")?,
        stream_ref: context.stream_ref,
        attributes,
    })
    .map_err(|_| IngestError::Record)?;
    Ok(EvidenceRecord {
        record,
        raw: Zeroizing::new(raw.to_vec()),
    })
}
