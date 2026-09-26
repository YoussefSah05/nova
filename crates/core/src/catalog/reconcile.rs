//! Snapshot-to-snapshot change classification and safe derived-work reuse.
//!
//! Compares two ingest generations as `identity key -> VersionedFingerprint`
//! maps — never raw [`SourceRecord`](super::SourceRecord)s, never the whole
//! catalog pairwise — and classifies each key as unchanged, changed, new, or
//! deleted. Expensive derived work (entity resolution, enrichment, AI facts)
//! may only be reused for [`ChangeKind::Unchanged`]; every other
//! classification means the evidence moved and derived work must be
//! recomputed against it.
//!
//! Both maps are already sorted (`BTreeMap`), so classification is one linear
//! merge pass: `O(n log n)` to build the maps, `O(n)` to merge them. Never a
//! per-record scan against the whole opposite generation.
//!
//! The identity key is the caller's responsibility and must be a *stable*
//! key that survives a re-import (e.g. a source-scoped binding key) — never
//! the ephemeral, per-occurrence `SourceRecordId` minted fresh by every
//! ingest, which would make every record look new on every run.

use std::collections::BTreeMap;

use super::fingerprint::VersionedFingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Same identity key, same fingerprint, and the fingerprint's format
    /// version is current in both generations.
    Unchanged,
    /// Same identity key, but a different fingerprint or a stale format
    /// version on either side.
    Changed,
    /// Present only in the current generation.
    New,
    /// Present only in the previous generation.
    Deleted,
}

/// Whether derived work computed against the previous generation's evidence
/// may be reused as-is for the current generation. Only [`ChangeKind::Unchanged`]
/// is safe: `Changed`/`New` evidence must be recomputed, and `Deleted` has no
/// current record left to attach reused work to. Centralized here so "safe
/// reuse" has exactly one definition instead of being re-derived at each
/// call site.
pub fn reuse_is_safe(kind: ChangeKind) -> bool {
    matches!(kind, ChangeKind::Unchanged)
}

/// Aggregate counts of a [`classify_generation`] result, for logging/telemetry
/// (the per-key detail is what callers act on; this is just the summary).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconciliationCounts {
    pub unchanged: usize,
    pub changed: usize,
    pub new: usize,
    pub deleted: usize,
}

/// Classify every identity key across two generations.
pub fn classify_generation<K: Ord + Clone>(
    previous: &BTreeMap<K, VersionedFingerprint>,
    current: &BTreeMap<K, VersionedFingerprint>,
) -> BTreeMap<K, ChangeKind> {
    let mut result = BTreeMap::new();
    let mut prev_iter = previous.iter().peekable();
    let mut curr_iter = current.iter().peekable();
    loop {
        let (key, kind) = match (prev_iter.peek(), curr_iter.peek()) {
            (Some((pk, _)), Some((ck, _))) if pk < ck => {
                let (k, _) = prev_iter.next().expect("peeked");
                (k.clone(), ChangeKind::Deleted)
            }
            (Some((pk, _)), Some((ck, _))) if pk > ck => {
                let (k, _) = curr_iter.next().expect("peeked");
                (k.clone(), ChangeKind::New)
            }
            (Some(_), Some(_)) => {
                let (k, prev_fp) = prev_iter.next().expect("peeked");
                let (_, curr_fp) = curr_iter.next().expect("peeked");
                let unchanged = prev_fp == curr_fp
                    && prev_fp.is_current_metadata()
                    && curr_fp.is_current_metadata();
                (
                    k.clone(),
                    if unchanged {
                        ChangeKind::Unchanged
                    } else {
                        ChangeKind::Changed
                    },
                )
            }
            (Some(_), None) => {
                let (k, _) = prev_iter.next().expect("peeked");
                (k.clone(), ChangeKind::Deleted)
            }
            (None, Some(_)) => {
                let (k, _) = curr_iter.next().expect("peeked");
                (k.clone(), ChangeKind::New)
            }
            (None, None) => break,
        };
        result.insert(key, kind);
    }
    result
}

/// Summarize a [`classify_generation`] result into aggregate counts.
pub fn summarize<K>(result: &BTreeMap<K, ChangeKind>) -> ReconciliationCounts {
    let mut counts = ReconciliationCounts::default();
    for kind in result.values() {
        match kind {
            ChangeKind::Unchanged => counts.unchanged += 1,
            ChangeKind::Changed => counts.changed += 1,
            ChangeKind::New => counts.new += 1,
            ChangeKind::Deleted => counts.deleted += 1,
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(seed: u8) -> VersionedFingerprint {
        VersionedFingerprint::from_stored(&format!("0001-{}", hex(seed))).unwrap()
    }

    fn hex(seed: u8) -> String {
        let byte = format!("{seed:02x}");
        byte.repeat(32)
    }

    #[test]
    fn classifies_all_four_kinds() {
        let mut previous = BTreeMap::new();
        previous.insert("kept", fp(1));
        previous.insert("edited", fp(2));
        previous.insert("removed", fp(3));

        let mut current = BTreeMap::new();
        current.insert("kept", fp(1));
        current.insert("edited", fp(9));
        current.insert("added", fp(4));

        let result = classify_generation(&previous, &current);
        assert_eq!(*result.get("kept").unwrap(), ChangeKind::Unchanged);
        assert_eq!(*result.get("edited").unwrap(), ChangeKind::Changed);
        assert_eq!(*result.get("removed").unwrap(), ChangeKind::Deleted);
        assert_eq!(*result.get("added").unwrap(), ChangeKind::New);
        assert_eq!(result.len(), 4);

        let counts = summarize(&result);
        assert_eq!(
            counts,
            ReconciliationCounts {
                unchanged: 1,
                changed: 1,
                new: 1,
                deleted: 1,
            }
        );
    }

    #[test]
    fn only_unchanged_permits_reuse() {
        assert!(reuse_is_safe(ChangeKind::Unchanged));
        for kind in [ChangeKind::Changed, ChangeKind::New, ChangeKind::Deleted] {
            assert!(!reuse_is_safe(kind));
        }
    }

    #[test]
    fn a_stale_format_version_forces_recompute_despite_identical_digest_bytes() {
        let current_fp = fp(7);
        let stale_stored = format!("0000-{}", hex(7));
        let stale = VersionedFingerprint::from_stored(&stale_stored).unwrap();
        assert_eq!(
            stale.fingerprint(),
            current_fp.fingerprint(),
            "digest bytes must be identical for this test to prove anything"
        );

        let mut previous = BTreeMap::new();
        previous.insert("key", stale);
        let mut current = BTreeMap::new();
        current.insert("key", current_fp);

        let result = classify_generation(&previous, &current);
        assert_eq!(*result.get("key").unwrap(), ChangeKind::Changed);
    }

    #[test]
    fn empty_generations_classify_to_nothing() {
        let empty: BTreeMap<&str, VersionedFingerprint> = BTreeMap::new();
        assert!(classify_generation(&empty, &empty).is_empty());
    }

    #[test]
    fn classification_is_linear_for_a_large_unchanged_generation() {
        const N: u32 = 20_000;
        let mut previous = BTreeMap::new();
        let mut current = BTreeMap::new();
        for i in 0..N {
            let value = fp((i % 251) as u8);
            previous.insert(i, value);
            current.insert(i, value);
        }
        let start = std::time::Instant::now();
        let result = classify_generation(&previous, &current);
        let elapsed = start.elapsed();
        assert!(result.values().all(|k| *k == ChangeKind::Unchanged));
        assert!(
            elapsed.as_secs() < 2,
            "classification took too long: {elapsed:?}"
        );
    }
}
