//! Title polishing — the deterministic, offline "rules" layer of catalog
//! enrichment (Phase A). Turns a country/year/season-stripped `clean_title` into
//! a display-ready name by folding decorative Unicode (e.g. `ᴴᴰ` → `HD`,
//! fullwidth → ASCII) and dropping release/quality/codec junk tokens
//! (`HD`, `1080p`, `x265`, `WEB-DL`, `MULTI`, `VOSTFR`, …).
//!
//! Conservative by design: it only removes tokens it is confident are junk, so
//! real titles (`Spider-Man`, `S.W.A.T`, `Web of Lies`) survive. Operates per
//! whitespace token; never returns empty (falls back to the folded input).
//!
//! This is the cheap first pass; later enrichment layers (embeddings, TMDB)
//! build on the cleaner output.

/// Unambiguous release/quality/codec/source tags, normalized to uppercase
/// alphanumerics (so `H.264`, `WEB-DL`, `x265` all match after normalization).
/// Deliberately excludes ambiguous standalone English words (`WEB`, `DVD`,
/// `RAW`, `VO`, `SUB`) to avoid mangling real titles.
const JUNK_TOKENS: &[&str] = &[
    "HD", "FHD", "UHD", "SD", "HQ", "HDR", "SDR", "4K", "8K", "HEVC", "AVC", "H264", "H265",
    "X264", "X265", "AAC", "AC3", "EAC3", "DTS", "DD51", "DDP", "MULTI", "DUAL", "VOSTFR", "VOST",
    "VFF", "VFQ", "VF", "WEBDL", "WEBRIP", "HDTV", "BLURAY", "BRRIP", "BDRIP", "HDRIP", "DVDRIP",
    "HDCAM", "DUBBED", "IMAX", "REMUX", "10BIT", "8BIT",
];

/// Polish a (already country/year/season-stripped) title into a display name.
pub fn polish_title(raw: &str) -> String {
    let folded = fold(raw);
    let kept: Vec<&str> = folded
        .split_whitespace()
        .filter(|tok| !is_junk_token(tok))
        .collect();
    let cleaned = cleanup(&kept.join(" "));
    if cleaned.is_empty() {
        // Everything was stripped (e.g. a channel literally named "HD"): keep
        // the folded original rather than render an empty card.
        cleanup(&folded)
    } else {
        cleaned
    }
}

/// Fold decorative Unicode to ASCII: fullwidth forms and the modifier/superscript
/// capital letters IPTV providers use as flair (`ᴴᴰ`, `ᵁᴴᴰ`). Other characters
/// (Arabic, accented Latin, …) pass through untouched.
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            'ᴬ' => 'A',
            'ᴮ' => 'B',
            'ᴰ' => 'D',
            'ᴱ' => 'E',
            'ᴳ' => 'G',
            'ᴴ' => 'H',
            'ᴵ' => 'I',
            'ᴶ' => 'J',
            'ᴷ' => 'K',
            'ᴸ' => 'L',
            'ᴹ' => 'M',
            'ᴺ' => 'N',
            'ᴼ' => 'O',
            'ᴾ' => 'P',
            'ᴿ' => 'R',
            'ᵀ' => 'T',
            'ᵁ' => 'U',
            'ⱽ' => 'V',
            'ᵂ' => 'W',
            other => other,
        })
        .collect()
}

/// True when a whitespace token is a junk tag or pure punctuation/separator.
fn is_junk_token(tok: &str) -> bool {
    let norm: String = tok
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect();
    if norm.is_empty() {
        // e.g. "-", "|", "•" — a leftover separator.
        return true;
    }
    JUNK_TOKENS.contains(&norm.as_str()) || is_resolution(&norm)
}

/// Matches resolution tokens: `1080P`, `720P`, `480I`, `2160P`, `4K`, `8K`.
fn is_resolution(norm: &str) -> bool {
    if let Some(rest) = norm.strip_suffix(['P', 'I', 'K']) {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// Collapse whitespace and trim dangling separators/punctuation left behind by
/// token removal.
fn cleanup(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '-' | '|' | ':' | '•' | '·' | '.' | ',' | '/')
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_quality_suffixes() {
        assert_eq!(polish_title("HBO HD"), "HBO");
        assert_eq!(polish_title("Cinemax FHD"), "Cinemax");
        assert_eq!(polish_title("The Matrix 1080p"), "The Matrix");
        assert_eq!(polish_title("Dune 2160P"), "Dune");
        assert_eq!(polish_title("ESPN 4K"), "ESPN");
    }

    #[test]
    fn folds_decorative_unicode() {
        assert_eq!(polish_title("HBO ᴴᴰ"), "HBO");
        assert_eq!(polish_title("ＨＢＯ Max"), "HBO Max");
    }

    #[test]
    fn strips_release_and_codec_junk() {
        assert_eq!(polish_title("Breaking Bad [MULTI] FHD"), "Breaking Bad");
        assert_eq!(polish_title("Interstellar WEB-DL x265"), "Interstellar");
        assert_eq!(polish_title("Le Film VOSTFR HD"), "Le Film");
        assert_eq!(polish_title("Some Movie H.264 AAC"), "Some Movie");
    }

    #[test]
    fn preserves_real_titles() {
        assert_eq!(polish_title("Spider-Man"), "Spider-Man");
        assert_eq!(polish_title("S.W.A.T"), "S.W.A.T");
        assert_eq!(polish_title("Web of Lies"), "Web of Lies");
        assert_eq!(polish_title("Mad Max: Fury Road"), "Mad Max: Fury Road");
    }

    #[test]
    fn collapses_separators_and_whitespace() {
        assert_eq!(polish_title("The   Office  -  HD"), "The Office");
        assert_eq!(polish_title("Movie |"), "Movie");
    }

    #[test]
    fn never_returns_empty() {
        // A channel literally named with only a quality tag keeps the original.
        assert_eq!(polish_title("HD"), "HD");
        assert_eq!(polish_title(""), "");
    }
}
