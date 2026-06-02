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

/// A grouping key for near-duplicate detection. Strips the same quality/codec
/// junk as [`polish_title`] and normalizes to lowercase alphanumerics, but
/// **preserves distinguishing numbers** so genuinely different entries never
/// merge: `BeIN Sports 1` ≠ `BeIN Sports 2`, `Rocky` ≠ `Rocky 2`, while
/// `HBO`, `HBO HD`, `HBO ᴴᴰ`, `HBO FHD` all collapse to `hbo`.
///
/// Returns an empty string when nothing distinctive remains (e.g. a channel
/// literally named "HD"); callers treat an empty key as "do not group".
pub fn dedup_key(raw: &str) -> String {
    fold(raw)
        .split_whitespace()
        .filter(|tok| !is_junk_token(tok))
        .map(|tok| {
            tok.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A **brand-family** key for grouping live channels: the inverse of
/// [`dedup_key`] — it *folds* a trailing channel number so a whole family
/// gathers (`beIN Sports 1`, `beIN Sports 2`, `beIN Sports HD` → `bein sports`).
/// To avoid clobbering single-word numbered brands (`France 24`, `RT`), the
/// trailing number is only dropped when at least two alphabetic words remain
/// (`Sky Sport 1` → `sky sport`, but `France 24` stays `france 24`).
pub fn channel_brand(raw: &str) -> String {
    let mut tokens: Vec<String> = fold(raw)
        .split_whitespace()
        .filter(|t| !is_junk_token(t))
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.is_empty() {
        // Only junk (e.g. "HD") — fall back to a normalized form of the whole name.
        return fold(raw)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
    }

    let alpha_words = tokens
        .iter()
        .filter(|t| t.chars().any(|c| c.is_alphabetic()))
        .count();
    if alpha_words >= 2
        && tokens
            .last()
            .is_some_and(|t| t.chars().all(|c| c.is_ascii_digit()))
    {
        tokens.pop();
    }
    tokens.join(" ")
}

/// Build the cleanest possible **TMDB search query** from a raw catalog name.
/// More aggressive than [`polish_title`]: drops a leading provider/country
/// prefix (everything before the last `|`), bracketed groups (`[MULTI]`,
/// `(2019)`), quality/codec junk, a 4-digit year (passed to the API separately),
/// and `SxxEyy` episode markers. Keeps the real words (incl. hyphenated names).
/// Falls back to the folded input if stripping leaves nothing.
pub fn search_title(raw: &str) -> String {
    let folded = fold(raw);
    // Provider/country prefixes precede pipes: "FR| US| The Matrix" → "The Matrix".
    let after_pipe = folded.rsplit('|').next().unwrap_or(&folded);

    // Drop bracketed/parenthesized groups.
    let mut unbracketed = String::with_capacity(after_pipe.len());
    let mut depth = 0i32;
    for c in after_pipe.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => unbracketed.push(c),
            _ => {}
        }
    }

    let kept: Vec<&str> = unbracketed
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | '_' | '|'))
        .filter(|t| !t.is_empty())
        .filter(|t| !is_junk_token(t))
        .filter(|t| !is_year_token(t))
        .filter(|t| !is_season_episode(t))
        .collect();

    let joined = cleanup(&kept.join(" "));
    if joined.is_empty() {
        cleanup(&folded)
    } else {
        joined
    }
}

/// Series/season indicator words (normalized, multilingual) dropped from a TV
/// search query — providers append these but TMDB wants just the show name.
const SERIES_WORDS: &[&str] = &[
    "season", "saison", "temporada", "staffel", "stagione", "series", "serie", "show", "مسلسل",
];

/// Like [`search_title`] but for **TV series**: also strips season/series
/// indicator words and the season number that usually follows them
/// (`Game of Thrones Season 8` → `Game of Thrones`, `مسلسل Breaking Bad` →
/// `Breaking Bad`).
pub fn search_series_title(raw: &str) -> String {
    let folded = fold(raw);
    let mut had_season = false;
    let kept: Vec<&str> = folded
        .split_whitespace()
        .filter(|t| {
            let norm: String = t
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect();
            if SERIES_WORDS.contains(&norm.as_str()) {
                had_season = true;
                return false;
            }
            true
        })
        .collect();
    // Once a season word appeared, a bare number is its index, not part of the title.
    let cleaned: Vec<&str> = if had_season {
        kept.into_iter()
            .filter(|t| {
                let alnum: String = t.chars().filter(|c| c.is_alphanumeric()).collect();
                alnum.is_empty() || !alnum.bytes().all(|b| b.is_ascii_digit())
            })
            .collect()
    } else {
        kept
    };
    search_title(&cleaned.join(" "))
}

/// A standalone 4-digit release year (1900–2099) — removed from search queries
/// since the year is sent to TMDB as a separate parameter.
fn is_year_token(tok: &str) -> bool {
    tok.len() == 4
        && tok.bytes().all(|c| c.is_ascii_digit())
        && matches!(tok.parse::<u32>(), Ok(y) if (1900..=2099).contains(&y))
}

/// An `SxxEyy` / `Sx` / `NxNN` episode marker (lowercased), e.g. `S01E02`, `1x05`.
fn is_season_episode(tok: &str) -> bool {
    let l = tok.to_ascii_lowercase();
    let b = l.as_bytes();
    // s<digits>e<digits> or s<digits>
    if b.first() == Some(&b's') && b.len() > 1 {
        let rest = &l[1..];
        if let Some(epos) = rest.find('e') {
            let (sn, ep) = (&rest[..epos], &rest[epos + 1..]);
            return !sn.is_empty()
                && sn.bytes().all(|c| c.is_ascii_digit())
                && !ep.is_empty()
                && ep.bytes().all(|c| c.is_ascii_digit());
        }
        if rest.bytes().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
            return true;
        }
    }
    // <digits>x<digits>
    if let Some(xpos) = l.find('x') {
        let (a, c) = (&l[..xpos], &l[xpos + 1..]);
        return !a.is_empty()
            && a.bytes().all(|d| d.is_ascii_digit())
            && !c.is_empty()
            && c.bytes().all(|d| d.is_ascii_digit());
    }
    false
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

    #[test]
    fn dedup_key_merges_quality_variants() {
        // Quality variants share a key (true duplicates).
        assert_eq!(dedup_key("HBO"), "hbo");
        assert_eq!(dedup_key("HBO HD"), "hbo");
        assert_eq!(dedup_key("HBO ᴴᴰ"), "hbo");
        assert_eq!(dedup_key("HBO FHD"), "hbo");
    }

    #[test]
    fn dedup_key_keeps_numbered_variants_distinct() {
        // Different numbers ⇒ different keys ⇒ never merged (the content-loss bug).
        assert_ne!(dedup_key("BeIN Sports 1 HD"), dedup_key("BeIN Sports 2 HD"));
        assert_eq!(dedup_key("BeIN Sports 1 HD"), "bein sports 1");
        assert_ne!(dedup_key("Rocky"), dedup_key("Rocky 2"));
        // A "backup"/alternate variant stays separate so it isn't hidden.
        assert_ne!(dedup_key("HBO"), dedup_key("HBO Backup"));
    }

    #[test]
    fn dedup_key_empty_when_only_junk() {
        assert_eq!(dedup_key("HD"), "");
    }

    #[test]
    fn search_title_strips_prefixes_brackets_year_quality() {
        assert_eq!(search_title("VOD| The Matrix 1999 1080p"), "The Matrix");
        assert_eq!(search_title("FR| US| Inception [MULTI] 2010"), "Inception");
        assert_eq!(search_title("EN - Breaking Bad S01E01"), "EN Breaking Bad");
        assert_eq!(
            search_title("Mad Max: Fury Road (2015) FHD"),
            "Mad Max Fury Road"
        );
    }

    #[test]
    fn search_title_keeps_real_titles() {
        assert_eq!(search_title("Spider-Man"), "Spider-Man");
        assert_eq!(search_title("Rocky 2"), "Rocky 2");
    }

    #[test]
    fn channel_brand_folds_families() {
        assert_eq!(channel_brand("beIN Sports 1"), "bein sports");
        assert_eq!(channel_brand("beIN Sports 2 HD"), "bein sports");
        assert_eq!(channel_brand("beIN Sports ᴴᴰ"), "bein sports");
        assert_eq!(channel_brand("Sky Sport 1"), "sky sport");
        assert_eq!(channel_brand("HBO"), "hbo");
    }

    #[test]
    fn channel_brand_keeps_single_word_numbered_brands() {
        // Only one alphabetic word ⇒ the number is part of the brand.
        assert_eq!(channel_brand("France 24"), "france 24");
        assert_eq!(channel_brand("Al Jazeera"), "al jazeera");
    }

    #[test]
    fn search_series_title_strips_season_words() {
        assert_eq!(search_series_title("Game of Thrones Season 8"), "Game of Thrones");
        assert_eq!(search_series_title("مسلسل Breaking Bad"), "Breaking Bad");
        assert_eq!(search_series_title("Dark Saison 2"), "Dark");
    }
}
