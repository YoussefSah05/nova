//! Near-duplicate clustering. IPTV catalogs are full of variants — `HBO`,
//! `HBO ᴴᴰ`, `HBO FHD`, `HBO Backup` — that should collapse into one logical
//! entry. We bucket on a cheap key first (keeping comparisons O(n) per bucket,
//! not O(n²) over the whole catalog), then greedily cluster within a bucket by
//! cosine similarity against each cluster's representative.

use crate::cosine;

/// An item to cluster: a stable `id`, a coarse `bucket` key (e.g. first title
/// token or category) that gates which items are even compared, and its vector.
pub struct DedupItem {
    pub id: i64,
    pub bucket: String,
    pub vector: Vec<f32>,
}

/// A set of ids judged to be the same logical entry. `member_ids[0]` is the
/// representative (first seen).
pub struct Cluster {
    pub member_ids: Vec<i64>,
}

/// Greedily cluster items by cosine similarity ≥ `threshold` within each bucket.
/// Order-stable: items keep input order; each joins the first representative it
/// matches, else starts a new cluster. Returns clusters in first-seen order.
pub fn cluster(items: &[DedupItem], threshold: f32) -> Vec<Cluster> {
    // (bucket, representative-vector, cluster-index)
    let mut reps: Vec<(&str, &[f32], usize)> = Vec::new();
    let mut clusters: Vec<Cluster> = Vec::new();

    for item in items {
        let mut joined = None;
        for (bucket, rep_vec, idx) in &reps {
            if *bucket == item.bucket && cosine(rep_vec, &item.vector) >= threshold {
                joined = Some(*idx);
                break;
            }
        }
        match joined {
            Some(idx) => clusters[idx].member_ids.push(item.id),
            None => {
                let idx = clusters.len();
                clusters.push(Cluster {
                    member_ids: vec![item.id],
                });
                reps.push((&item.bucket, &item.vector, idx));
            }
        }
    }
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, bucket: &str, v: Vec<f32>) -> DedupItem {
        DedupItem {
            id,
            bucket: bucket.to_string(),
            vector: v,
        }
    }

    #[test]
    fn collapses_near_identical_within_bucket() {
        let items = vec![
            item(1, "hbo", vec![1.0, 0.0]),
            item(2, "hbo", vec![0.99, 0.01]), // ~same as 1
            item(3, "hbo", vec![0.0, 1.0]),   // different
        ];
        let clusters = cluster(&items, 0.95);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].member_ids, vec![1, 2]);
        assert_eq!(clusters[1].member_ids, vec![3]);
    }

    #[test]
    fn different_buckets_never_merge() {
        let items = vec![
            item(1, "a", vec![1.0, 0.0]),
            item(2, "b", vec![1.0, 0.0]), // identical vector, different bucket
        ];
        let clusters = cluster(&items, 0.5);
        assert_eq!(clusters.len(), 2);
    }
}
