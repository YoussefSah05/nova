//! Versioned fingerprints, not canonical IDs or credential protection.
//!
//! Framed v1 uses u64 big-endian byte lengths, option presence bytes and UTF-8
//! without normalization. Never use Rust's Hash/DefaultHasher for persisted keys.

use super::{ContentKind, SourceRecord};
use sha2::{Digest, Sha256};
use std::fmt;

pub const FINGERPRINT_FORMAT: &str = "sha256-framed-v1";

/// A digest may correlate sensitive content. Debug intentionally hides its value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub(super) fn from_digest(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Explicit export for protected persistence/checks; not ordinary diagnostics.
    pub fn to_hex(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut result = String::with_capacity(64);
        for byte in self.0 {
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 15) as usize] as char);
        }
        result
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Fingerprint([redacted])")
    }
}

fn field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn optional(hash: &mut Sha256, value: Option<&str>) {
    match value {
        Some(value) => {
            hash.update([1]);
            field(hash, value.as_bytes());
        }
        None => hash.update([0]),
    }
}

impl SourceRecord {
    /// Conservative raw-metadata change key. Scope caches by source separately.
    /// Capture IDs and protected locator references deliberately do not participate.
    /// Unknown attributes are included: adapters must separate transport secrets
    /// from metadata if token rotation is to avoid metadata recomputation.
    pub fn metadata_fingerprint(&self) -> Fingerprint {
        let mut hash = Sha256::new();
        field(&mut hash, b"nova.record-metadata.v1");
        let kind = match self.kind_hint() {
            ContentKind::LiveChannel => "liveChannel",
            ContentKind::Movie => "movie",
            ContentKind::Series => "series",
            ContentKind::Season => "season",
            ContentKind::Episode => "episode",
            ContentKind::Event => "event",
            ContentKind::Radio => "radio",
            ContentKind::Unknown => "unknown",
        };
        field(&mut hash, kind.as_bytes());
        match self.upstream() {
            Some(key) => {
                hash.update([1]);
                field(&mut hash, key.namespace.as_bytes());
                field(&mut hash, key.value.as_bytes());
            }
            None => hash.update([0]),
        }
        field(&mut hash, self.raw_title().as_bytes());
        optional(&mut hash, self.raw_group());
        optional(&mut hash, self.raw_logo());
        optional(&mut hash, self.raw_epg_id());
        hash.update((self.attributes().len() as u64).to_be_bytes());
        for (key, value) in self.attributes() {
            field(&mut hash, key.as_bytes());
            field(&mut hash, value.as_bytes());
        }
        Fingerprint::from_digest(hash.finalize().into())
    }
}

/// Hash actual canonical locator bytes and a vault-issued credential revision.
/// The caller owns canonical encoding and must not log inputs or this digest.
/// This is neither encryption nor a secret-verification/authentication protocol.
pub fn locator_fingerprint(locator: &[u8], credential_revision: &[u8]) -> Fingerprint {
    let mut hash = Sha256::new();
    field(&mut hash, b"nova.locator.v1");
    field(&mut hash, locator);
    field(&mut hash, credential_revision);
    Fingerprint::from_digest(hash.finalize().into())
}

/// Compiler binding digest for a provider-scoped key (`namespace` separates a
/// stable upstream ID from a locator). Callers scope the result by source.
/// Not a canonical ID and not credential protection.
pub fn binding_fingerprint(namespace: &str, value: &[u8]) -> Fingerprint {
    let mut hash = Sha256::new();
    field(&mut hash, b"nova.binding.v1");
    field(&mut hash, namespace.as_bytes());
    field(&mut hash, value);
    Fingerprint::from_digest(hash.finalize().into())
}
