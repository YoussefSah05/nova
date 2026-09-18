//! Versioned protected record archive. Normalized fields are a replayable cache;
//! raw provider bytes remain authoritative evidence, not frontend data.
use super::*;
use nova_core::catalog::snapshot::*;
use nova_storage::snapshots::stream::{StreamWriter, StreamingSnapshotStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    num::{NonZeroU32, NonZeroU64},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;

const MAX_LINE_BYTES: usize = 2 * 1024 * 1024;
const ARCHIVE_SCOPE: &str = "record-stage-v1";
#[derive(Clone, Copy)]
pub struct StageLimits {
    pub max_records: u64,
    pub max_bytes: u64,
}
impl StageLimits {
    fn validate(self) -> Result<()> {
        if self.max_records == 0
            || self.max_records > 100_000
            || self.max_bytes == 0
            || self.max_bytes > 4 * 1024 * 1024 * 1024
        {
            return Err(IngestError::Limit);
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Header {
    format: u32,
    adapter_revision: u32,
    source: SourceId,
    raw_snapshot: SnapshotId,
    raw_scope: EndpointScope,
    raw_parser_revision: NonZeroU32,
    raw_bytes: u64,
    raw_digest: String,
    raw_complete: bool,
}
impl Header {
    fn from_input(input: &SourceSnapshot) -> Self {
        Self {
            format: 1,
            adapter_revision: ADAPTER_REVISION,
            source: input.descriptor().source_id.clone(),
            raw_snapshot: input.descriptor().id.clone(),
            raw_scope: input.descriptor().scope.clone(),
            raw_parser_revision: input.descriptor().parser_revision,
            raw_bytes: input.byte_count(),
            raw_digest: input.payload_fingerprint().to_hex(),
            raw_complete: input.completeness() == Completeness::Complete,
        }
    }
    fn owns(&self, record: &SourceRecord) -> bool {
        record.source_id() == &self.source && record.snapshot_id() == &self.raw_snapshot
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireRecord {
    id: SourceRecordId,
    source_id: SourceId,
    snapshot_id: SnapshotId,
    upstream: Option<(String, String)>,
    kind: ContentKind,
    title: String,
    group: Option<String>,
    logo: Option<String>,
    epg: Option<String>,
    stream_ref: StreamReference,
    attributes: BTreeMap<String, String>,
    raw: Vec<u8>,
}
impl WireRecord {
    fn encode(evidence: &EvidenceRecord) -> Self {
        let r = evidence.record();
        Self {
            id: r.id().clone(),
            source_id: r.source_id().clone(),
            snapshot_id: r.snapshot_id().clone(),
            upstream: r.upstream().map(|k| (k.namespace.clone(), k.value.clone())),
            kind: r.kind_hint(),
            title: r.raw_title().into(),
            group: r.raw_group().map(str::to_owned),
            logo: r.raw_logo().map(str::to_owned),
            epg: r.raw_epg_id().map(str::to_owned),
            stream_ref: r.stream_ref().clone(),
            attributes: r.attributes().clone(),
            raw: evidence.raw_object().to_vec(),
        }
    }
    fn decode(self) -> Result<EvidenceRecord> {
        object(&self.raw)?;
        let record = SourceRecord::try_from(SourceRecordDraft {
            id: self.id,
            source_id: self.source_id,
            snapshot_id: self.snapshot_id,
            upstream: self
                .upstream
                .map(|(namespace, value)| UpstreamKey { namespace, value }),
            kind_hint: self.kind,
            raw_title: self.title,
            raw_group: self.group,
            raw_logo: self.logo,
            raw_epg_id: self.epg,
            stream_ref: self.stream_ref,
            attributes: self.attributes,
        })
        .map_err(|_| IngestError::Record)?;
        Ok(EvidenceRecord {
            record,
            raw: Zeroizing::new(self.raw),
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Line {
    Header(Header),
    Record(WireRecord),
    Footer { count: u64 },
}
fn encode(line: &Line) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(serde_json::to_vec(line).map_err(|_| IngestError::Archive)?);
    if bytes.len() > MAX_LINE_BYTES {
        return Err(IngestError::Limit);
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub struct RecordStage {
    writer: Option<StreamWriter>,
    capture: SnapshotCapture,
    header: Header,
    ids: BTreeSet<SourceRecordId>,
    limits: StageLimits,
}
impl RecordStage {
    /// The input receipt must refer to separately retained raw evidence. Use a
    /// separate store namespace for simultaneous raw replay and derived writes.
    pub async fn begin(
        store: &StreamingSnapshotStore,
        artifact: SnapshotId,
        input: &SourceSnapshot,
        limits: StageLimits,
    ) -> Result<Self> {
        limits.validate()?;
        if artifact == input.descriptor().id {
            return Err(IngestError::Ownership);
        }
        let descriptor = SnapshotDescriptor {
            id: artifact,
            source_id: input.descriptor().source_id.clone(),
            scope: EndpointScope::new(ARCHIVE_SCOPE).map_err(|_| IngestError::Archive)?,
            parser_revision: NonZeroU32::new(1).ok_or(IngestError::Archive)?,
        };
        let writer = store.begin(descriptor.clone()).await?;
        let mut stage = Self {
            writer: Some(writer),
            capture: SnapshotCapture::new(
                descriptor,
                SnapshotLimits {
                    max_bytes: NonZeroU64::new(limits.max_bytes).ok_or(IngestError::Limit)?,
                    max_records: NonZeroU64::new(1).ok_or(IngestError::Limit)?,
                },
            ),
            header: Header::from_input(input),
            ids: BTreeSet::new(),
            limits,
        };
        let bytes = encode(&Line::Header(Header::from_input(input)))?;
        stage.append(&bytes).await?;
        Ok(stage)
    }
    async fn fail(&mut self) {
        if let Some(writer) = self.writer.take() {
            let _ = writer.abort().await;
        }
        let _ = self.capture.fail();
    }
    async fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if self.capture.push_bytes(bytes).is_err() {
            self.fail().await;
            return Err(IngestError::Limit);
        }
        let result = self
            .writer
            .as_mut()
            .ok_or(IngestError::Terminal)?
            .append(bytes)
            .await;
        if let Err(error) = result {
            self.fail().await;
            return Err(error.into());
        }
        Ok(())
    }
    pub async fn push(&mut self, evidence: &EvidenceRecord) -> Result<()> {
        if self.writer.is_none() {
            return Err(IngestError::Terminal);
        }
        let record = evidence.record();
        let checked = if !self.header.owns(record) {
            Err(IngestError::Ownership)
        } else if self.ids.contains(record.id()) {
            Err(IngestError::DuplicateRecord)
        } else if self.ids.len() as u64 >= self.limits.max_records {
            Err(IngestError::Limit)
        } else {
            encode(&Line::Record(WireRecord::encode(evidence)))
        };
        let bytes = match checked {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail().await;
                return Err(error);
            }
        };
        self.append(&bytes).await?;
        self.ids.insert(record.id().clone());
        Ok(())
    }
    pub async fn finish(mut self) -> Result<u64> {
        if self.writer.is_none() {
            return Err(IngestError::Terminal);
        }
        let count = self.ids.len() as u64;
        self.append(&encode(&Line::Footer { count })?).await?;
        // Transport receipt counts provider records only. This derived artifact
        // has zero such records; its authenticated footer owns the staged count.
        let receipt = self
            .capture
            .seal(Completeness::Complete)
            .map_err(|_| IngestError::Terminal)?;
        self.writer
            .take()
            .ok_or(IngestError::Terminal)?
            .seal(receipt)
            .await?;
        Ok(count)
    }
}

struct StageSink<F> {
    line: Zeroizing<Vec<u8>>,
    expected: Header,
    ids: BTreeSet<SourceRecordId>,
    seen_header: bool,
    finished: bool,
    total: u64,
    limits: StageLimits,
    callback: F,
    failure: Option<IngestError>,
}
impl<F: FnMut(EvidenceRecord) -> Result<()>> StageSink<F> {
    fn line(&mut self) -> Result<()> {
        if self.finished {
            return Err(IngestError::Archive);
        }
        let line: Line = serde_json::from_slice(&self.line).map_err(|_| IngestError::Archive)?;
        match line {
            Line::Header(header) if !self.seen_header && header == self.expected => {
                self.seen_header = true
            }
            Line::Record(wire) if self.seen_header => {
                let evidence = wire.decode()?;
                if !self.expected.owns(evidence.record()) {
                    return Err(IngestError::Ownership);
                }
                if self.ids.len() as u64 >= self.limits.max_records {
                    return Err(IngestError::Limit);
                }
                if !self.ids.insert(evidence.record().id().clone()) {
                    return Err(IngestError::DuplicateRecord);
                }
                (self.callback)(evidence)?;
            }
            Line::Footer { count } if self.seen_header && count == self.ids.len() as u64 => {
                self.finished = true
            }
            _ => return Err(IngestError::Archive),
        }
        self.line.clear();
        Ok(())
    }
    fn consume(&mut self, bytes: &[u8]) -> Result<()> {
        self.total = self
            .total
            .checked_add(bytes.len() as u64)
            .ok_or(IngestError::Limit)?;
        if self.total > self.limits.max_bytes {
            return Err(IngestError::Limit);
        }
        for &byte in bytes {
            if byte == b'\n' {
                self.line()?;
            } else {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(IngestError::Limit);
                }
                self.line.push(byte);
            }
        }
        Ok(())
    }
}
impl<F: FnMut(EvidenceRecord) -> Result<()> + Unpin> AsyncWrite for StageSink<F> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.failure.is_some() {
            return Poll::Ready(Err(std::io::Error::other("record archive rejected")));
        }
        match self.consume(bytes) {
            Ok(()) => Poll::Ready(Ok(bytes.len())),
            Err(error) => {
                self.failure = Some(error);
                Poll::Ready(Err(std::io::Error::other("record archive rejected")))
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Callbacks receive provisional evidence, not publishable catalog records. Only
/// Ok certifies both archive grammar and final encrypted transport receipt.
pub async fn replay_stage<F: FnMut(EvidenceRecord) -> Result<()> + Unpin>(
    store: &StreamingSnapshotStore,
    artifact: SnapshotId,
    input: &SourceSnapshot,
    limits: StageLimits,
    callback: F,
) -> Result<u64> {
    limits.validate()?;
    let mut sink = StageSink {
        line: Zeroizing::new(Vec::new()),
        expected: Header::from_input(input),
        ids: BTreeSet::new(),
        seen_header: false,
        finished: false,
        total: 0,
        limits,
        callback,
        failure: None,
    };
    let result = store.replay_into(artifact, &mut sink).await;
    if let Some(error) = sink.failure.take() {
        return Err(error);
    }
    let receipt = result?;
    let descriptor = receipt.descriptor();
    if !sink.finished
        || !sink.line.is_empty()
        || descriptor.source_id != input.descriptor().source_id
        || descriptor.scope.as_str() != ARCHIVE_SCOPE
        || descriptor.parser_revision.get() != 1
        || receipt.completeness() != Completeness::Complete
        || receipt.record_count() != 0
    {
        return Err(IngestError::Archive);
    }
    Ok(sink.ids.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> SourceSnapshot {
        let mut capture = SnapshotCapture::new(
            SnapshotDescriptor {
                id: SnapshotId::new("raw").unwrap(),
                source_id: SourceId::new("source").unwrap(),
                scope: EndpointScope::new("vod").unwrap(),
                parser_revision: NonZeroU32::new(1).unwrap(),
            },
            SnapshotLimits {
                max_bytes: NonZeroU64::new(1024).unwrap(),
                max_records: NonZeroU64::new(1).unwrap(),
            },
        );
        capture.seal(Completeness::Complete).unwrap()
    }
    fn sink(input: &SourceSnapshot) -> StageSink<impl FnMut(EvidenceRecord) -> Result<()> + Unpin> {
        StageSink {
            line: Zeroizing::new(Vec::new()),
            expected: Header::from_input(input),
            ids: BTreeSet::new(),
            seen_header: false,
            finished: false,
            total: 0,
            limits: StageLimits {
                max_records: 10,
                max_bytes: 10_000_000,
            },
            callback: |_| Ok(()),
            failure: None,
        }
    }
    #[test]
    fn archive_order_and_terminal_footer_are_enforced() {
        let input = input();
        let header = encode(&Line::Header(Header::from_input(&input))).unwrap();
        let footer = encode(&Line::Footer { count: 0 }).unwrap();
        assert!(sink(&input).consume(&footer).is_err());
        let mut reader = sink(&input);
        reader.consume(&header).unwrap();
        assert!(reader.consume(&header).is_err());
        let mut reader = sink(&input);
        reader.consume(&header).unwrap();
        assert!(reader
            .consume(&encode(&Line::Footer { count: 1 }).unwrap())
            .is_err());
        let mut reader = sink(&input);
        // Exercise framing independently of worker chunk boundaries.
        for byte in header.iter().chain(footer.iter()) {
            reader.consume(&[*byte]).unwrap();
        }
        assert!(reader.finished);
        assert!(reader.consume(&footer).is_err());
    }
    #[test]
    fn archive_line_limit_precedes_unbounded_accumulation() {
        let input = input();
        let mut reader = sink(&input);
        let chunk = vec![b' '; 64 * 1024];
        for _ in 0..MAX_LINE_BYTES / chunk.len() {
            reader.consume(&chunk).unwrap();
        }
        assert!(matches!(reader.consume(b" "), Err(IngestError::Limit)));
        assert_eq!(reader.line.len(), MAX_LINE_BYTES);
    }
}
