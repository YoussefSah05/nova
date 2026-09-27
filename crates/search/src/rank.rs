//! Deterministic catalog ranking.
//!
//! Lexical retrieval finds candidates; this module orders them so that
//! **exact titles are always reliable**: a query that exactly names a title
//! puts that title first, whatever BM25, popularity or semantic similarity
//! say. Tiers, strongest first:
//!
//! 1. [`MatchTier::ExactTitle`]: the query equals a title (display, original or
//!    localized), after normalization (case, accents, punctuation).
//! 2. [`MatchTier::TitlePrefix`]: a title starts with the query.
//! 3. [`MatchTier::TitleWords`]: every query word appears in a title.
//! 4. [`MatchTier::Alias`]: every query word appears in an alias (provider
//!    titles, parsed titles).
//! 5. [`MatchTier::Metadata`]: every query word appears in the title, alias or
//!    metadata (genres, keywords, people) text combined.
//!
//! Within a tier, the lexical rank (BM25 order) and then the key decide, so the
//! result is identical for identical input. Semantic retrieval is optional: its
//! hits are fused only *below* the title tiers ([`merge_with_semantic`]), so a
//! missing or unhelpful embedding model can never displace an exact title.

use std::collections::HashSet;
use std::hash::Hash;

/// How well a document matches a query. Lower is stronger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MatchTier {
    ExactTitle,
    TitlePrefix,
    TitleWords,
    Alias,
    Metadata,
}

impl MatchTier {
    /// Tiers that name the title itself; semantic hits never outrank these.
    pub fn is_title(self) -> bool {
        matches!(
            self,
            Self::ExactTitle | Self::TitlePrefix | Self::TitleWords
        )
    }
}

/// The searchable text of one catalog item.
#[derive(Debug, Clone, Default)]
pub struct SearchDoc<'a> {
    /// Display, original and localized titles.
    pub titles: Vec<&'a str>,
    /// Provider titles and parsed titles.
    pub aliases: Vec<&'a str>,
    /// Genres, keywords, people.
    pub metadata: Vec<&'a str>,
}

/// Fold common Latin accents so `Amélie` matches `amelie`.
fn fold_char(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'č' | 'ć' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ß' => 's',
        'š' | 'ś' => 's',
        'ž' | 'ź' | 'ż' => 'z',
        other => other,
    }
}

/// Normalize text for matching: lowercase, accents folded, punctuation
/// removed, words separated by single spaces. Non-Latin scripts pass through.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = true;
    for c in text.chars().flat_map(char::to_lowercase).map(fold_char) {
        if c.is_alphanumeric() {
            out.push(c);
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim_end().to_string()
}

fn words(normalized: &str) -> Vec<&str> {
    normalized.split(' ').filter(|w| !w.is_empty()).collect()
}

fn contains_all(haystack: &str, query_words: &[&str]) -> bool {
    let hay: HashSet<&str> = words(haystack).into_iter().collect();
    // The last word may be a prefix (live typeahead: "godf" → "godfather").
    query_words.iter().enumerate().all(|(i, w)| {
        hay.contains(w) || (i + 1 == query_words.len() && hay.iter().any(|h| h.starts_with(w)))
    })
}

/// The strongest tier at which `doc` matches `query`; `None` if it doesn't.
pub fn match_tier(query: &str, doc: &SearchDoc<'_>) -> Option<MatchTier> {
    let q = normalize(query);
    if q.is_empty() {
        return None;
    }
    let qw = words(&q);
    let titles: Vec<String> = doc.titles.iter().map(|t| normalize(t)).collect();
    if titles.contains(&q) {
        return Some(MatchTier::ExactTitle);
    }
    if titles.iter().any(|t| t.starts_with(&q)) {
        return Some(MatchTier::TitlePrefix);
    }
    if titles.iter().any(|t| contains_all(t, &qw)) {
        return Some(MatchTier::TitleWords);
    }
    let aliases: Vec<String> = doc.aliases.iter().map(|a| normalize(a)).collect();
    if aliases.iter().any(|a| *a == q || contains_all(a, &qw)) {
        return Some(MatchTier::Alias);
    }
    let all = titles
        .iter()
        .chain(&aliases)
        .cloned()
        .chain(doc.metadata.iter().map(|m| normalize(m)))
        .collect::<Vec<_>>()
        .join(" ");
    contains_all(&all, &qw).then_some(MatchTier::Metadata)
}

/// Order lexical candidates `(key, tier, lexical_rank)`: tier, then lexical
/// rank, then key. Deterministic for identical input.
pub fn rank_lexical<K: Ord + Clone>(mut hits: Vec<(K, MatchTier, usize)>) -> Vec<(K, MatchTier)> {
    hits.sort_by(|a, b| (a.1, a.2, &a.0).cmp(&(b.1, b.2, &b.0)));
    hits.into_iter().map(|(k, t, _)| (k, t)).collect()
}

/// Final order: title-tier lexical hits first, untouched; the remaining
/// lexical hits and the (optional) semantic hits fused below them by RRF.
/// Keys are unique in the output.
pub fn merge_with_semantic<K: Clone + Eq + Hash>(
    ranked_lexical: &[(K, MatchTier)],
    semantic: &[K],
) -> Vec<K> {
    let mut out: Vec<K> = Vec::new();
    let mut seen: HashSet<K> = HashSet::new();
    for (key, tier) in ranked_lexical {
        if tier.is_title() && seen.insert(key.clone()) {
            out.push(key.clone());
        }
    }
    let rest: Vec<K> = ranked_lexical
        .iter()
        .filter(|(k, t)| !t.is_title() && !seen.contains(k))
        .map(|(k, _)| k.clone())
        .collect();
    let semantic: Vec<K> = semantic
        .iter()
        .filter(|k| !seen.contains(*k))
        .cloned()
        .collect();
    let fused: Vec<K> = if semantic.is_empty() {
        rest
    } else {
        crate::rrf_fuse(&[&rest, &semantic], crate::RRF_K)
            .into_iter()
            .map(|(k, _)| k)
            .collect()
    };
    for key in fused {
        if seen.insert(key.clone()) {
            out.push(key);
        }
    }
    out
}

/// FTS5 expression requiring every query word, each as a prefix term, so the
/// words may appear in any field and order (`"pacino"* "godfather"*`). Words
/// are quoted, so operators and punctuation in user input stay literal.
pub fn fts_all_words(input: &str) -> String {
    input
        .split(|c: char| c.is_whitespace() || c == '"')
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc<'a>(titles: &[&'a str], aliases: &[&'a str], metadata: &[&'a str]) -> SearchDoc<'a> {
        SearchDoc {
            titles: titles.to_vec(),
            aliases: aliases.to_vec(),
            metadata: metadata.to_vec(),
        }
    }

    #[test]
    fn tiers_are_ordered_from_exact_title_to_metadata() {
        let godfather = doc(
            &["The Godfather", "Le Parrain"],
            &["FR | LE PARRAIN FHD"],
            &[
                "Crime",
                "Drama",
                "mafia",
                "Al Pacino",
                "Francis Ford Coppola",
            ],
        );
        assert_eq!(
            match_tier("the godfather", &godfather),
            Some(MatchTier::ExactTitle)
        );
        assert_eq!(
            match_tier("LE PARRAIN", &godfather),
            Some(MatchTier::ExactTitle)
        );
        assert_eq!(
            match_tier("The God", &godfather),
            Some(MatchTier::TitlePrefix)
        );
        assert_eq!(
            match_tier("godfather", &godfather),
            Some(MatchTier::TitleWords)
        );
        assert_eq!(
            match_tier("fhd parrain", &godfather),
            Some(MatchTier::Alias)
        );
        assert_eq!(match_tier("pacino", &godfather), Some(MatchTier::Metadata));
        assert_eq!(
            match_tier("coppola mafia", &godfather),
            Some(MatchTier::Metadata)
        );
        assert_eq!(match_tier("scarface", &godfather), None);
    }

    #[test]
    fn normalization_ignores_case_accents_and_punctuation() {
        let amelie = doc(&["Le Fabuleux Destin d'Amélie Poulain", "Amélie"], &[], &[]);
        assert_eq!(match_tier("amelie", &amelie), Some(MatchTier::ExactTitle));
        let mi = doc(&["Mission: Impossible"], &[], &[]);
        assert_eq!(
            match_tier("mission impossible", &mi),
            Some(MatchTier::ExactTitle)
        );
        assert_eq!(
            normalize("  Spider-Man: No Way Home! "),
            "spider man no way home"
        );
        assert_eq!(
            normalize("الحفرة"),
            "الحفرة",
            "non-Latin scripts pass through"
        );
    }

    #[test]
    fn an_exact_title_outranks_better_lexical_scores() {
        // BM25 put the longer title first; the exact title still wins.
        let ranked = rank_lexical(vec![
            ("heat-wave", MatchTier::TitleWords, 0),
            ("heat", MatchTier::ExactTitle, 5),
            ("heat-2", MatchTier::TitlePrefix, 1),
        ]);
        let keys: Vec<_> = ranked.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["heat", "heat-2", "heat-wave"]);
    }

    #[test]
    fn semantic_hits_never_displace_title_matches() {
        let lexical = rank_lexical(vec![
            ("heat", MatchTier::ExactTitle, 0),
            ("heat-wave", MatchTier::TitleWords, 1),
            ("crime-doc", MatchTier::Metadata, 2),
        ]);
        let semantic = ["thief", "collateral", "heat", "crime-doc"];
        let merged = merge_with_semantic(&lexical, &semantic);
        assert_eq!(&merged[..2], &["heat", "heat-wave"]);
        // Below the title tiers, metadata and semantic hits are fused, no dupes.
        assert_eq!(merged.len(), 5);
        assert!(merged[2..].contains(&"crime-doc") && merged[2..].contains(&"thief"));
        // Without a model, the lexical order is kept as-is.
        assert_eq!(
            merge_with_semantic(&lexical, &[]),
            vec!["heat", "heat-wave", "crime-doc"]
        );
    }

    #[test]
    fn ranking_is_deterministic_on_ties() {
        let a = rank_lexical(vec![("b", MatchTier::Alias, 3), ("a", MatchTier::Alias, 3)]);
        let b = rank_lexical(vec![("a", MatchTier::Alias, 3), ("b", MatchTier::Alias, 3)]);
        assert_eq!(a, b);
        assert_eq!(a[0].0, "a");
    }

    #[test]
    fn fts_expression_requires_all_words_and_escapes_input() {
        assert_eq!(
            fts_all_words("pacino godfather"),
            "\"pacino\"* \"godfather\"*"
        );
        assert_eq!(fts_all_words("  a \"OR\" b "), "\"a\"* \"OR\"* \"b\"*");
        assert_eq!(fts_all_words("-- !!"), "");
        assert_eq!(fts_all_words("NEAR(x"), "\"NEAR(x\"*");
    }

    #[test]
    fn the_last_word_may_be_a_prefix_for_typeahead() {
        let d = doc(&["The Godfather Part II"], &[], &[]);
        assert_eq!(match_tier("godfather par", &d), Some(MatchTier::TitleWords));
        assert_eq!(
            match_tier("godf part", &d),
            None,
            "only the last word is a prefix"
        );
    }
}
