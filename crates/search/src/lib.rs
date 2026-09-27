//! FTS5 query building for the `transcript_fts` index (Phase 8).
//!
//! The only job here is turning raw user input into a **safe** FTS5 MATCH
//! expression. FTS5 treats `" : ( ) { } * ^ -` and bareword `AND/OR/NOT/NEAR` as
//! operators, so unescaped input is both an injection risk and a syntax-error
//! risk. We sidestep all of it by quoting the whole input as a single phrase
//! (doubling any embedded quote) and appending `*` so the last word
//! prefix-matches — i.e. live typeahead: `prof` → `"prof"*` matches "profit".
//! The storage layer binds the result to `text MATCH ?`.

pub mod rank;

/// Build a safe FTS5 phrase-prefix MATCH expression from raw user input.
/// Returns an empty string for blank input — callers must treat that as "no
/// query" and skip the search (an empty MATCH is a syntax error in FTS5).
pub fn fts_match(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let escaped = trimmed.replace('"', "\"\"");
    format!("\"{escaped}\"*")
}

// ── Semantic retrieval (hybrid search) ────────────────────────────────────────
//
// The lexical leg above (FTS5/BM25) finds exact words; the semantic leg below
// finds *meaning* by comparing on-device embedding vectors. We keep both legs as
// ranked id lists and combine them with Reciprocal Rank Fusion so exact matches
// stay sharp while paraphrases / cross-language hits still surface. All of this
// is pure math with no deps — the embedder lives in `nova-enrich`, the vectors
// are loaded by the storage layer, and this crate just ranks + fuses.

/// Cosine similarity of two equal-length vectors. Returns 0 on length mismatch
/// or a zero vector (mirrors `nova_enrich::cosine`; duplicated to keep this crate
/// dependency-free).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
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

/// Brute-force nearest-neighbour search: score `query` against every `(key,
/// vector)` in `corpus` by cosine and return the top `k` as `(key, score)`,
/// highest first. Generic over the key type so the corpus can be keyed by row id
/// (`i64`, movies/channels) or title (`String`, series). At Nova's scale (tens
/// of thousands of short vectors) this is a few ms — no ANN index / extension.
pub fn top_k<T: Clone>(query: &[f32], corpus: &[(T, Vec<f32>)], k: usize) -> Vec<(T, f32)> {
    let mut scored: Vec<(T, f32)> = corpus
        .iter()
        .map(|(key, v)| (key.clone(), cosine(query, v)))
        .collect();
    // Descending by score; NaN (shouldn't occur) sinks to the bottom.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored
}

/// Default RRF dampening constant (Cormack et al. 2009). Larger `k` flattens the
/// contribution of rank position; 60 is the well-established default.
pub const RRF_K: f64 = 60.0;

/// Reciprocal Rank Fusion of several ranked key lists into one ranking.
///
/// Each input list is an ordering (best first) from one retriever — e.g. the
/// FTS5/BM25 list and the cosine list. A key's fused score is
/// `Σ 1/(k + rank)` over the lists it appears in (rank is 0-based). Operating on
/// **ranks** (not raw scores) means we can fuse BM25 and cosine without any score
/// normalization. Generic over the key type so callers can fuse by row id
/// (`i64`, movies/dialogue) or by group key (`String`, live channels). Returns
/// `(key, fused_score)` sorted highest first.
pub fn rrf_fuse<T: Clone + Eq + std::hash::Hash>(lists: &[&[T]], k: f64) -> Vec<(T, f64)> {
    use std::collections::HashMap;
    let mut scores: HashMap<T, f64> = HashMap::new();
    let mut order: Vec<T> = Vec::new(); // first-seen order, for stable tie-breaks
    for list in lists {
        for (rank, key) in list.iter().enumerate() {
            if !scores.contains_key(key) {
                order.push(key.clone());
            }
            *scores.entry(key.clone()).or_insert(0.0) += 1.0 / (k + rank as f64);
        }
    }
    let mut fused: Vec<(T, f64)> = order
        .into_iter()
        .map(|key| {
            let s = scores[&key];
            (key, s)
        })
        .collect();
    fused.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_prefixes() {
        assert_eq!(fts_match("profit"), "\"profit\"*");
        assert_eq!(fts_match("profit margin"), "\"profit margin\"*");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(fts_match("   profit margin  "), "\"profit margin\"*");
    }

    #[test]
    fn blank_is_empty() {
        assert_eq!(fts_match(""), "");
        assert_eq!(fts_match("    "), "");
    }

    #[test]
    fn neutralizes_operators_and_quotes() {
        // Embedded quotes are doubled; operators become literal inside the phrase.
        assert_eq!(fts_match("say \"hi\""), "\"say \"\"hi\"\"\"*");
        assert_eq!(fts_match("a OR b) (c"), "\"a OR b) (c\"*");
        assert_eq!(fts_match("*"), "\"*\"*");
    }

    #[test]
    fn cosine_matches_orientation() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        // Magnitude-invariant: parallel vectors of different lengths → 1.0.
        assert!((cosine(&[2.0, 0.0], &[5.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0); // length mismatch
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0); // zero vector
    }

    #[test]
    fn top_k_returns_nearest_first_and_truncates() {
        let q = vec![1.0, 0.0];
        let corpus = vec![
            (10, vec![0.0, 1.0]),  // orthogonal → 0
            (20, vec![1.0, 0.0]),  // identical → 1
            (30, vec![1.0, 0.1]),  // close → high
            (40, vec![-1.0, 0.0]), // opposite → -1
        ];
        let hits = top_k(&q, &corpus, 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0, 20);
        assert_eq!(hits[1].0, 30);
    }

    #[test]
    fn rrf_combines_two_rankings() {
        // Lexical ranks 1 best; semantic ranks 3 best; 2 is mid in both. Because
        // 1/(k+rank) is convex, a top (rank-0) placement in one list outweighs
        // two middling placements, so the two extremes tie above the middle item.
        let lexical = [1, 2, 3];
        let semantic = [3, 2, 1];
        let fused = rrf_fuse(&[&lexical, &semantic], RRF_K);
        assert_eq!(fused.len(), 3);
        assert_eq!(fused.last().unwrap().0, 2, "the never-top item ranks last");
        // 1 and 3 are symmetric → equal scores, both above 2.
        let s1 = fused.iter().find(|(id, _)| *id == 1).unwrap().1;
        let s2 = fused.iter().find(|(id, _)| *id == 2).unwrap().1;
        let s3 = fused.iter().find(|(id, _)| *id == 3).unwrap().1;
        assert!((s1 - s3).abs() < 1e-9);
        assert!(s1 > s2);
    }

    #[test]
    fn rrf_rewards_appearing_in_both_lists() {
        // An id present in both lists beats one present in only a single list,
        // even if the single-list one is rank 0 there.
        let a = [1, 2];
        let b = [2, 3];
        let fused = rrf_fuse(&[&a, &b], RRF_K);
        assert_eq!(fused[0].0, 2, "shared id ranks first");
    }

    #[test]
    fn rrf_fuses_string_keys() {
        // Live channels fuse on group-key strings, not ids.
        let lexical = ["bein sports".to_string(), "espn".to_string()];
        let semantic = ["espn".to_string(), "sky sports".to_string()];
        let fused = rrf_fuse(&[&lexical[..], &semantic[..]], RRF_K);
        assert_eq!(fused[0].0, "espn", "key in both lists wins");
        assert_eq!(fused.len(), 3);
    }
}
