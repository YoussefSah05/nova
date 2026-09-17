//! In-memory capture receipts. No filesystem, catalog publication or compiler state.
//! The adapter must report completeness truthfully after validating endpoint EOF.

use super::{fingerprint::Fingerprint, EndpointScope, SnapshotId, SourceId, SourceRecord};
use sha2::{Digest, Sha256};
use std::num::{NonZeroU32, NonZeroU64};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDescriptor {
    pub id: SnapshotId,
    pub source_id: SourceId,
    /// One endpoint scope; lazy episode requests must have their own scope IDs.
    pub scope: EndpointScope,
    pub parser_revision: NonZeroU32,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotLimits {
    pub max_bytes: NonZeroU64,
    pub max_records: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureState {
    Receiving,
    Complete,
    Partial,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Partial,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("capture is terminal: {state:?}")]
    Terminal { state: CaptureState },
    #[error("snapshot byte limit exceeded")]
    ByteLimit,
    #[error("snapshot record limit exceeded")]
    RecordLimit,
    #[error("record belongs to a different source or snapshot")]
    RecordOwnership,
    #[error("snapshot source or endpoint scope differs")]
    ScopeMismatch,
    #[error("previous snapshot must be complete")]
    PreviousNotComplete,
}

/// Bounded counters and streaming digest only; raw payload storage is separate.
/// Intentionally no Clone/Debug to avoid exposing internal hash state or forking a capture.
pub struct SnapshotCapture {
    descriptor: SnapshotDescriptor,
    limits: SnapshotLimits,
    state: CaptureState,
    bytes: u64,
    records: u64,
    payload: Sha256,
}

impl SnapshotCapture {
    pub fn new(descriptor: SnapshotDescriptor, limits: SnapshotLimits) -> Self {
        Self {
            descriptor,
            limits,
            state: CaptureState::Receiving,
            bytes: 0,
            records: 0,
            payload: Sha256::new(),
        }
    }

    pub fn state(&self) -> CaptureState {
        self.state
    }

    fn receiving(&self) -> Result<(), SnapshotError> {
        if self.state != CaptureState::Receiving {
            return Err(SnapshotError::Terminal { state: self.state });
        }
        Ok(())
    }

    /// Supply exactly the bytes retained for replay, once, before parsing them.
    /// Transport decoding choice belongs to the adapter and must remain consistent.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        self.receiving()?;
        let count = self.bytes.checked_add(bytes.len() as u64);
        match count {
            Some(count) if count <= self.limits.max_bytes.get() => {
                self.payload.update(bytes);
                self.bytes = count;
                Ok(())
            }
            _ => {
                self.state = CaptureState::Failed;
                Err(SnapshotError::ByteLimit)
            }
        }
    }

    /// Count accepted records; uniqueness and rejected-record policy belong to the
    /// staging writer/compiler. No record or ID set is retained in this object.
    pub fn observe_record(&mut self, record: &SourceRecord) -> Result<(), SnapshotError> {
        self.receiving()?;
        if record.source_id() != &self.descriptor.source_id
            || record.snapshot_id() != &self.descriptor.id
        {
            self.state = CaptureState::Failed;
            return Err(SnapshotError::RecordOwnership);
        }
        match self.records.checked_add(1) {
            Some(count) if count <= self.limits.max_records.get() => {
                self.records = count;
                Ok(())
            }
            _ => {
                self.state = CaptureState::Failed;
                Err(SnapshotError::RecordLimit)
            }
        }
    }

    /// Complete means authoritative endpoint completion, not validated catalog output.
    /// A zero-record complete receipt is valid; publication still needs collapse checks.
    pub fn seal(&mut self, completeness: Completeness) -> Result<SourceSnapshot, SnapshotError> {
        self.receiving()?;
        self.state = match completeness {
            Completeness::Complete => CaptureState::Complete,
            Completeness::Partial => CaptureState::Partial,
        };
        Ok(SourceSnapshot {
            descriptor: self.descriptor.clone(),
            completeness,
            byte_count: self.bytes,
            record_count: self.records,
            payload_fingerprint: Fingerprint::from_digest(self.payload.clone().finalize().into()),
        })
    }

    pub fn cancel(&mut self) -> Result<(), SnapshotError> {
        self.receiving()?;
        self.state = CaptureState::Cancelled;
        Ok(())
    }

    pub fn fail(&mut self) -> Result<(), SnapshotError> {
        self.receiving()?;
        self.state = CaptureState::Failed;
        Ok(())
    }
}

/// Immutable evidence receipt. This does not certify durable payload persistence.
#[derive(Debug)]
pub struct SourceSnapshot {
    descriptor: SnapshotDescriptor,
    completeness: Completeness,
    byte_count: u64,
    record_count: u64,
    payload_fingerprint: Fingerprint,
}

impl SourceSnapshot {
    pub fn descriptor(&self) -> &SnapshotDescriptor {
        &self.descriptor
    }
    pub fn completeness(&self) -> Completeness {
        self.completeness
    }
    pub fn byte_count(&self) -> u64 {
        self.byte_count
    }
    pub fn record_count(&self) -> u64 {
        self.record_count
    }
    pub fn payload_fingerprint(&self) -> Fingerprint {
        self.payload_fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotFreshness {
    Current,
    StalePrevious,
    PartialFirstImport,
}

#[derive(Debug)]
pub struct SnapshotSelection<'a> {
    snapshot: &'a SourceSnapshot,
    freshness: SnapshotFreshness,
}

impl<'a> SnapshotSelection<'a> {
    pub fn snapshot(&self) -> &'a SourceSnapshot {
        self.snapshot
    }
    pub fn freshness(&self) -> SnapshotFreshness {
        self.freshness
    }
}

/// Select one endpoint input, never publish it. `previous` is the last complete
/// receipt for this profile/source/scope. Partial first import must stay visible
/// as partial to downstream validation/UI. Callers must enforce profile isolation.
pub fn select_snapshot<'a>(
    candidate: &'a SourceSnapshot,
    previous: Option<&'a SourceSnapshot>,
) -> Result<SnapshotSelection<'a>, SnapshotError> {
    if let Some(previous) = previous {
        if candidate.descriptor.source_id != previous.descriptor.source_id
            || candidate.descriptor.scope != previous.descriptor.scope
        {
            return Err(SnapshotError::ScopeMismatch);
        }
        if previous.completeness != Completeness::Complete {
            return Err(SnapshotError::PreviousNotComplete);
        }
    }
    match (candidate.completeness, previous) {
        (Completeness::Complete, _) => Ok(SnapshotSelection {
            snapshot: candidate,
            freshness: SnapshotFreshness::Current,
        }),
        (Completeness::Partial, Some(previous)) => Ok(SnapshotSelection {
            snapshot: previous,
            freshness: SnapshotFreshness::StalePrevious,
        }),
        (Completeness::Partial, None) => Ok(SnapshotSelection {
            snapshot: candidate,
            freshness: SnapshotFreshness::PartialFirstImport,
        }),
    }
}
