//! Variant selection (bootstrap §24): which of a canonical title's playable
//! provider sources to play. Deterministic and explainable — no model in the
//! playback path. Criteria, in order:
//!
//! 1. the variant the user explicitly asked for;
//! 2. no recent playback failure (health recorded by the playback
//!    coordinator);
//! 3. the profile's preferred audio language: a source announcing it, then a
//!    multi-language source, then one with no language evidence, then others;
//! 4. picture quality: UHD, then full HD, then HD, then unknown, then SD;
//! 5. the lowest provider row (the previous rule), so ties stay stable.

use crate::clean::{parse_source_title, TitleKind};

/// What is known about one playable source of a title.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantCandidate {
    pub variant_id: String,
    /// Current provider row (the transient player locator).
    pub row_id: i64,
    /// Raw provider title: its prefix (`FR |`, `EN -`) names the language.
    pub raw_title: String,
    /// Normalized picture/codec tags (`4K`, `1080P`, `HEVC`).
    pub quality_hints: Vec<String>,
    /// Normalized audio-version tags (`MULTI`, `VF`, `VOSTFR`).
    pub language_hints: Vec<String>,
    /// Failed more recently than it last played.
    pub recently_failed: bool,
}

/// How a source's audio language matches the preference (lower is better).
fn language_rank(c: &VariantCandidate, preferred: Option<&str>) -> u8 {
    let Some(preferred) = preferred.map(|p| p.trim().to_ascii_lowercase()) else {
        return 0;
    };
    let preferred = preferred.split(['-', '_']).next().unwrap_or("").to_string();
    if preferred.is_empty() {
        return 0;
    }
    let tags: Vec<&str> = c.language_hints.iter().map(String::as_str).collect();
    // French audio versions (version française / québécoise).
    let dubbed_fr = tags.iter().any(|t| matches!(*t, "VF" | "VFF" | "VFQ"));
    let multi = tags.iter().any(|t| matches!(*t, "MULTI" | "DUAL"));
    let prefix = parse_source_title(&c.raw_title, TitleKind::Movie)
        .country_prefix
        .map(|p| p.to_ascii_lowercase());
    let announced: Vec<String> = prefix
        .into_iter()
        .chain(dubbed_fr.then(|| "fr".to_string()))
        .collect();
    if announced.contains(&preferred) {
        0
    } else if multi {
        1
    } else if announced.is_empty() {
        2
    } else {
        3
    }
}

/// Picture quality from the source's tags (lower is better).
fn quality_rank(c: &VariantCandidate) -> u8 {
    let has = |tags: &[&str]| c.quality_hints.iter().any(|q| tags.contains(&q.as_str()));
    if has(&["4K", "UHD", "2160P", "8K"]) {
        0
    } else if has(&["FHD", "1080P", "1080I"]) {
        1
    } else if has(&["HD", "720P"]) {
        2
    } else if has(&["SD", "480P", "576P", "360P"]) {
        4
    } else {
        3
    }
}

/// The candidates in play order (best first). `requested` is honoured first
/// when it is one of them.
pub fn rank_variants<'a>(
    candidates: &'a [VariantCandidate],
    requested: Option<&str>,
    preferred_lang: Option<&str>,
) -> Vec<&'a VariantCandidate> {
    let mut ranked: Vec<&VariantCandidate> = candidates.iter().collect();
    ranked.sort_by_key(|c| {
        (
            requested != Some(c.variant_id.as_str()),
            c.recently_failed,
            language_rank(c, preferred_lang),
            quality_rank(c),
            c.row_id,
        )
    });
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variant(id: &str, row: i64, raw: &str, quality: &[&str], lang: &[&str]) -> VariantCandidate {
        VariantCandidate {
            variant_id: id.into(),
            row_id: row,
            raw_title: raw.into(),
            quality_hints: quality.iter().map(|s| s.to_string()).collect(),
            language_hints: lang.iter().map(|s| s.to_string()).collect(),
            recently_failed: false,
        }
    }

    fn order(ranked: Vec<&VariantCandidate>) -> Vec<&str> {
        ranked.into_iter().map(|c| c.variant_id.as_str()).collect()
    }

    #[test]
    fn preferred_language_then_quality_then_row() {
        let cands = [
            variant("en-fhd", 1, "EN - Heat (1995)", &["FHD"], &[]),
            variant("fr-sd", 2, "FR | Heat (1995)", &["SD"], &[]),
            variant("multi-4k", 3, "Heat (1995)", &["4K"], &["MULTI"]),
            variant("vf-hd", 4, "Heat (1995)", &["HD"], &["VF"]),
            variant("plain", 5, "Heat (1995)", &[], &[]),
        ];
        assert_eq!(
            order(rank_variants(&cands, None, Some("fr"))),
            ["vf-hd", "fr-sd", "multi-4k", "plain", "en-fhd"],
            "announced French first (by quality), then multi, then unknown, then other"
        );
        assert_eq!(
            order(rank_variants(&cands, None, Some("en-US"))),
            ["en-fhd", "multi-4k", "plain", "vf-hd", "fr-sd"]
        );
        assert_eq!(
            order(rank_variants(&cands, None, None)),
            ["multi-4k", "en-fhd", "vf-hd", "plain", "fr-sd"],
            "no preference: quality, then row"
        );
    }

    #[test]
    fn an_explicit_choice_wins_and_recent_failures_sink() {
        let mut cands = vec![
            variant("a", 1, "Heat", &["4K"], &[]),
            variant("b", 2, "Heat", &["SD"], &[]),
            variant("c", 3, "Heat", &["FHD"], &[]),
        ];
        cands[0].recently_failed = true;
        assert_eq!(order(rank_variants(&cands, None, None)), ["c", "b", "a"]);
        assert_eq!(
            order(rank_variants(&cands, Some("a"), None)),
            ["a", "c", "b"]
        );
        assert_eq!(
            order(rank_variants(&cands, Some("unknown"), None)),
            ["c", "b", "a"],
            "a request for another title's variant is ignored"
        );
    }

    #[test]
    fn identical_evidence_keeps_the_previous_lowest_row_rule() {
        let cands = [
            variant("x", 9, "Heat", &[], &[]),
            variant("y", 4, "Heat", &[], &[]),
        ];
        assert_eq!(order(rank_variants(&cands, None, Some("fr"))), ["y", "x"]);
    }
}
