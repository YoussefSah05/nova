//! Near-duplicate clustering. IPTV catalogs are full of *quality* variants —
//! `HBO`, `HBO ᴴᴰ`, `HBO FHD` — that should collapse into one logical entry. But
//! genuinely different entries that merely look alike — `BeIN Sports 1` vs `BeIN
//! Sports 2`, `Rocky` vs `Rocky 2` — must NOT merge (that was losing content).
//!
//! So grouping is gated on an exact rules `key` (`nova_core::clean::dedup_key`,
//! which strips quality junk but preserves distinguishing numbers); embedding
//! cosine is only a secondary guard against the rare case where two different
//! things normalize to the same key. Items with an empty key never group. Keyed
//! lookup keeps the whole pass O(n).

use std::collections::HashMap;

use crate::cosine;

/// An item to cluster: a stable `id`, a coarse `bucket` (first title token), an
/// exact grouping `key` (empty ⇒ never group), and its embedding vector.
pub struct DedupItem {
    pub id: i64,
    pub bucket: String,
    pub key: String,
    pub vector: Vec<f32>,
}

/// A set of ids judged to be the same logical entry. `member_ids[0]` is the
/// representative (first seen).
pub struct Cluster {
    pub member_ids: Vec<i64>,
}

/// Group items that share an exact `(bucket, key)` and whose vectors are at least
/// `threshold` cosine-similar. Order-stable; returns clusters in first-seen order.
pub fn cluster(items: &[DedupItem], threshold: f32) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    // Representative vector per cluster index (borrows from `items`).
    let mut rep_vec: Vec<&[f32]> = Vec::new();
    // First cluster index seen for each (bucket, key).
    let mut by_key: HashMap<(&str, &str), usize> = HashMap::new();

    for item in items {
        // An empty key (only junk in the name) never groups.
        if !item.key.is_empty() {
            let k = (item.bucket.as_str(), item.key.as_str());
            if let Some(&idx) = by_key.get(&k) {
                if cosine(rep_vec[idx], &item.vector) >= threshold {
                    clusters[idx].member_ids.push(item.id);
                    continue;
                }
                // Same key but dissimilar vectors — fall through to a new cluster
                // without remapping the key (rare; the first rep keeps the key).
            } else {
                by_key.insert(k, clusters.len());
            }
        }
        clusters.push(Cluster {
            member_ids: vec![item.id],
        });
        rep_vec.push(&item.vector);
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, bucket: &str, key: &str, v: Vec<f32>) -> DedupItem {
        DedupItem {
            id,
            bucket: bucket.to_string(),
            key: key.to_string(),
            vector: v,
        }
    }

    #[test]
    fn collapses_near_identical_within_key() {
        let items = vec![
            item(1, "hbo", "hbo", vec![1.0, 0.0]),
            item(2, "hbo", "hbo", vec![0.99, 0.01]), // same key, ~same vector
            item(3, "hbo", "hbo", vec![0.0, 1.0]),   // same key, orthogonal → guard splits
        ];
        let clusters = cluster(&items, 0.95);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].member_ids, vec![1, 2]);
        assert_eq!(clusters[1].member_ids, vec![3]);
    }

    #[test]
    fn different_keys_never_merge() {
        // Identical vectors but different keys (e.g. "Sports 1" vs "Sports 2").
        let items = vec![
            item(1, "bein", "bein sports 1", vec![1.0, 0.0]),
            item(2, "bein", "bein sports 2", vec![1.0, 0.0]),
        ];
        let clusters = cluster(&items, 0.5);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn empty_key_never_groups() {
        let items = vec![
            item(1, "hd", "", vec![1.0, 0.0]),
            item(2, "hd", "", vec![1.0, 0.0]),
        ];
        let clusters = cluster(&items, 0.5);
        assert_eq!(clusters.len(), 2);
    }
}
