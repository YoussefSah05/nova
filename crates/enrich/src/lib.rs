//! Catalog enrichment (Phase B) — on-device embeddings for fuzzy de-duplication,
//! zero-shot genre tagging, and semantic matching. The heavy model is optional
//! and lazily loaded ([`FastEmbedder`]); all clustering/classification logic is
//! pure over vectors so it's testable without the model (CI needs no download).

mod dedup;
mod embed;
mod taxonomy;

pub use dedup::{cluster, Cluster, DedupItem};
pub use embed::{Embedder, FastEmbedder};
pub use taxonomy::GenreClassifier;

#[derive(thiserror::Error, Debug)]
pub enum EnrichError {
    #[error("embedding model error: {0}")]
    Model(String),
}

/// Cosine similarity of two equal-length vectors (0 if either is zero/length-mismatched).
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_basics() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0); // length mismatch
    }
}
