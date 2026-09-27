//! The single deterministic parser for provider titles.
//!
//! It turns a raw provider name into a display title plus the **separate**
//! structured clues that were embedded in it: country/provider prefix, year,
//! season/episode numbering, and quality/language tags. Nothing it strips is
//! lost; every clue is returned as its own field. The raw string itself is never
//! modified: callers keep it as evidence.
//!
//! Rules are conservative: a token is removed only when it is unambiguously a
//! tag, so real titles survive (`Spider-Man`, `S.W.A.T.`, `Blade Runner 2049`,
//! `1917`, `The Office (US)`). Output is deterministic and versioned by
//! [`PARSER_VERSION`]; bump it whenever any rule changes, so stored parses
//! can be recomputed.

use super::{cleanup, fold, is_junk_token};

/// Version of the parsing rules. Bump on any behaviour change.
pub const PARSER_VERSION: u32 = 1;

/// A bare (unbracketed) trailing number is read as a release year only up to
/// this value, so numbers in titles such as `Blade Runner 2049` survive.
const MAX_BARE_YEAR: u16 = 2030;
const MIN_YEAR: u16 = 1900;
const MAX_YEAR: u16 = 2099;

/// Tags that are language/audio versions rather than picture quality.
const LANGUAGE_TAGS: &[&str] = &[
    "MULTI", "DUAL", "VOSTFR", "VOST", "VFF", "VFQ", "VF", "DUBBED",
];

/// Short uppercase provider tags that look like codes but are not countries.
const NON_COUNTRY_TAGS: &[&str] = &[
    "VOD", "SRS", "NF", "HD", "FHD", "UHD", "SD", "TOP", "NEW", "ALL", "EU", "VIP", "PPV",
];

/// Language codes providers use as prefixes, in addition to country codes.
const LANGUAGE_CODES: &[&str] = &[
    "EN", "AR", "FR", "DE", "ES", "IT", "PT", "NL", "TR", "RU", "PL", "HI", "UR", "FA", "KU", "ZH",
    "JA", "KO", "SV", "NO", "DA", "FI", "EL", "HE", "RO", "HU", "CS", "SK", "UK",
];

/// Words announcing a season number (normalized lowercase).
const SEASON_WORDS: &[&str] = &[
    "season",
    "saison",
    "temporada",
    "staffel",
    "stagione",
    "sezon",
    "seizoen",
    "الموسم",
];

/// Words announcing an episode number (normalized lowercase).
const EPISODE_WORDS: &[&str] = &[
    "episode",
    "épisode",
    "ep",
    "folge",
    "capitulo",
    "capítulo",
    "episodio",
    "bölüm",
    "bolum",
    "aflevering",
    "الحلقة",
    "حلقة",
];

/// Leading words that only say "series" and are not part of the name.
const SERIES_PREFIX_WORDS: &[&str] = &["مسلسل"];

/// What the title belongs to. It decides which clues are extracted: episodes
/// carry numbering, movies carry a year, live channels keep their numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleKind {
    Movie,
    Series,
    Live,
}

/// A parsed provider title. `title` is display-ready and never empty; the
/// other fields are clues found in (and removed from) the raw text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedTitle {
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    /// Episode number; for absolute numbering (`Folge 1234`) `season` is `None`.
    pub episode: Option<u32>,
    /// First country/language code found in a provider prefix (`FR`, `UK`).
    pub country_prefix: Option<String>,
    /// Picture/codec/source tags, normalized uppercase (`4K`, `1080P`, `HEVC`).
    pub quality_hints: Vec<String>,
    /// Language/audio-version tags, normalized uppercase (`MULTI`, `VOSTFR`).
    pub language_hints: Vec<String>,
}

fn norm_upper(tok: &str) -> String {
    tok.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_uppercase())
        .collect()
}

fn norm_lower(tok: &str) -> String {
    tok.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_separator(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| !c.is_alphanumeric())
}

fn year_value(tok: &str) -> Option<u16> {
    let digits: String = tok
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '[' | ']'))
        .collect();
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits
        .parse::<u16>()
        .ok()
        .filter(|y| (MIN_YEAR..=MAX_YEAR).contains(y))
}

fn number(tok: &str) -> Option<u32> {
    let t = tok.trim_matches(|c: char| !c.is_alphanumeric());
    (!t.is_empty() && t.len() <= 5 && t.bytes().all(|b| b.is_ascii_digit()))
        .then(|| t.parse().ok())
        .flatten()
}

/// `S01E02`, `s1e2`, `1x07` → (season, episode); `S01` → (season, None).
fn season_episode_token(tok: &str) -> Option<(u32, Option<u32>)> {
    let t = tok
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase();
    let digits = |s: &str| {
        (!s.is_empty() && s.len() <= 4 && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    if let Some(rest) = t.strip_prefix('s') {
        if let Some((s, e)) = rest.split_once('e') {
            return Some((digits(s)?, Some(digits(e)?)));
        }
        return digits(rest).map(|s| (s, None));
    }
    if let Some((s, e)) = t.split_once('x') {
        // Needs a short season and a 2+ digit episode: `1x07`, not `4x4`.
        if s.len() <= 2 && e.len() >= 2 {
            return Some((digits(s)?, Some(digits(e)?)));
        }
    }
    None
}

/// `E07` / `Ep07` following a bare season token.
fn episode_token(tok: &str) -> Option<u32> {
    let t = tok
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase();
    let rest = t.strip_prefix("ep").or_else(|| t.strip_prefix('e'))?;
    number(rest)
}

/// Scene names use dots between words: `BREAKING.BAD.S03E07`. Split a token
/// on dots only when at least two parts are real words, so `S.W.A.T.`,
/// `H.264` and `Dr.` stay intact.
fn expand_scene_dots(tok: &str) -> Vec<String> {
    let parts: Vec<&str> = tok.split(['.', '_']).collect();
    let wordy = parts
        .iter()
        .filter(|p| p.chars().filter(|c| c.is_alphanumeric()).count() >= 2)
        .count();
    if parts.len() >= 2 && wordy >= 2 {
        parts
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![tok.to_string()]
    }
}

/// A pipe-delimited provider tag: short, no spaces, no lowercase letters
/// (`FR`, `SRS-FR`, `NF-FR`, `4K`, `VOD`, `EN-AR`).
fn is_tag_segment(seg: &str) -> bool {
    let seg = seg.trim();
    !seg.is_empty()
        && seg.chars().count() <= 12
        && seg
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '-' | '+' | '_'))
}

/// The country/language code inside a tag, e.g. `SRS-FR` → `FR`.
fn tag_code(seg: &str) -> Option<String> {
    seg.trim()
        .split(['-', '_', '+'])
        .rev()
        .find(|c| {
            (2..=3).contains(&c.len())
                && c.bytes().all(|b| b.is_ascii_uppercase())
                && !NON_COUNTRY_TAGS.contains(c)
        })
        .map(str::to_string)
}

fn known_code(code: &str) -> bool {
    crate::region::region_of(code).is_some() || LANGUAGE_CODES.contains(&code)
}

/// Remove leading provider decoration: `FR| `, `FR | `, `|AR| `, `SRS-FR| `,
/// `FR| US| `, and `UK: ` / `FR - ` for known codes. Returns the first code.
pub(super) fn strip_prefixes(s: &str) -> (Option<String>, String) {
    let mut rest = s.trim().to_string();
    let mut country = None;
    for _ in 0..4 {
        let trimmed = rest.trim_start_matches(|c: char| c == '|' || c.is_whitespace());
        if let Some((seg, after)) = trimmed.split_once('|') {
            if is_tag_segment(seg) && !after.trim().is_empty() {
                if country.is_none() {
                    country = tag_code(seg);
                }
                rest = after.trim().to_string();
                continue;
            }
        }
        // `UK: Title`, `FR - Title`: only for a known code, so real words stay.
        let code: String = trimmed
            .chars()
            .take_while(char::is_ascii_uppercase)
            .collect();
        if (2..=3).contains(&code.len()) && known_code(&code) {
            let after = trimmed[code.len()..].trim_start();
            if let Some(tail) = after
                .strip_prefix(':')
                .or_else(|| after.strip_prefix("- "))
                .or_else(|| after.strip_prefix("– "))
            {
                if !tail.trim().is_empty() {
                    country.get_or_insert(code);
                    rest = tail.trim().to_string();
                    continue;
                }
            }
        }
        rest = trimmed.to_string();
        break;
    }
    (country, rest)
}

enum Piece {
    /// A bracketed group kept verbatim, e.g. `(US)`, `(Director's Cut)`.
    Group(String),
    Word(String),
}

/// Parse a raw provider title. Deterministic for a given [`PARSER_VERSION`].
pub fn parse_source_title(raw: &str, kind: TitleKind) -> ParsedTitle {
    let folded = fold(raw);
    let (country_prefix, rest) = strip_prefixes(&folded);
    let mut parsed = ParsedTitle {
        country_prefix,
        ..ParsedTitle::default()
    };

    // 1. Bracketed groups: tag-only or year-only groups become clues; any
    //    other group (`(US)`) is part of the title.
    let mut pieces: Vec<Piece> = Vec::new();
    let mut paren_year: Option<u16> = None;
    let mut buf = String::new();
    let mut group: Option<String> = None;
    let mut depth = 0usize;
    let flush_words = |buf: &mut String, pieces: &mut Vec<Piece>| {
        for tok in buf.split_whitespace() {
            for part in expand_scene_dots(tok) {
                pieces.push(Piece::Word(part));
            }
        }
        buf.clear();
    };
    for c in rest.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    flush_words(&mut buf, &mut pieces);
                    group = Some(String::new());
                } else if let Some(g) = group.as_mut() {
                    g.push(c);
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let inner = group.take().unwrap_or_default();
                    let tokens: Vec<&str> = inner.split_whitespace().collect();
                    let year = (tokens.len() == 1).then(|| year_value(tokens[0])).flatten();
                    if kind != TitleKind::Live && year.is_some() {
                        paren_year = year;
                    } else if !tokens.is_empty() && tokens.iter().all(|t| is_junk_token(t)) {
                        for t in tokens {
                            record_tag(&mut parsed, t);
                        }
                    } else if !tokens.is_empty() {
                        pieces.push(Piece::Group(format!("({})", tokens.join(" "))));
                    }
                } else if let Some(g) = group.as_mut() {
                    g.push(c);
                }
            }
            _ if depth > 0 => {
                if let Some(g) = group.as_mut() {
                    g.push(c);
                }
            }
            _ => buf.push(c),
        }
    }
    if let Some(unclosed) = group.take() {
        buf.push(' ');
        buf.push_str(&unclosed);
    }
    flush_words(&mut buf, &mut pieces);

    // 2. Tags anywhere become hints; pipes are provider syntax, never title.
    let mut words: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match piece {
            Piece::Word(w) if w == "|" => {}
            Piece::Word(w) if !is_separator(&w) && is_junk_token(&w) => record_tag(&mut parsed, &w),
            other => words.push(other),
        }
    }

    // 3. Kind-specific clues.
    let mut cut = words.len();
    if kind == TitleKind::Series {
        if let Some((at, season, episode)) = find_numbering(&words) {
            parsed.season = season;
            parsed.episode = episode;
            cut = at;
            // `S.W.A.T. 2017 S05E01`: a year right before the numbering.
            if at >= 2 {
                if let Piece::Word(w) = &words[at - 1] {
                    if let Some(y) = year_value(w) {
                        parsed.year = Some(y);
                        cut = at - 1;
                    }
                }
            }
        }
    }
    words.truncate(cut);
    if kind == TitleKind::Series {
        while let Some(Piece::Word(w)) = words.first() {
            if SERIES_PREFIX_WORDS.contains(&norm_lower(w).as_str()) && words.len() > 1 {
                words.remove(0);
            } else {
                break;
            }
        }
    }
    if kind != TitleKind::Live {
        if paren_year.is_some() {
            parsed.year = paren_year;
        } else if kind == TitleKind::Movie && words.len() >= 2 {
            // A trailing bare year: `Heat 1995`, but not `Blade Runner 2049`.
            if let Some(Piece::Word(w)) = words.last() {
                if let Some(y) = year_value(w).filter(|y| *y <= MAX_BARE_YEAR) {
                    parsed.year = Some(y);
                    words.pop();
                }
            }
        }
    }

    // 4. Assemble: drop leading/doubled separators, then trim dangling ones.
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for piece in words {
        let text = match piece {
            Piece::Group(g) => g,
            Piece::Word(w) => w,
        };
        let sep = is_separator(&text);
        if sep && out.last().is_none_or(|prev| is_separator(prev)) {
            continue;
        }
        out.push(text);
    }
    let title = cleanup(&out.join(" "));
    parsed.title = if title.is_empty() {
        // Nothing but tags (a channel literally called "HD"): keep the text.
        cleanup(&folded)
    } else {
        title
    };
    parsed
}

fn record_tag(parsed: &mut ParsedTitle, tok: &str) {
    let tag = norm_upper(tok);
    if tag.is_empty() {
        return;
    }
    let bucket = if LANGUAGE_TAGS.contains(&tag.as_str()) {
        &mut parsed.language_hints
    } else {
        &mut parsed.quality_hints
    };
    if !bucket.contains(&tag) {
        bucket.push(tag);
    }
}

/// First season/episode marker among the words: returns where it starts.
fn find_numbering(words: &[Piece]) -> Option<(usize, Option<u32>, Option<u32>)> {
    let word = |i: usize| match words.get(i) {
        Some(Piece::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    for i in 0..words.len() {
        let Some(w) = word(i) else { continue };
        // Never treat the first word as numbering: it is the name.
        if i == 0 {
            continue;
        }
        if let Some((season, episode)) = season_episode_token(w) {
            let episode = episode.or_else(|| word(i + 1).and_then(episode_token));
            return Some((i, Some(season), episode));
        }
        let lw = norm_lower(w);
        if SEASON_WORDS.contains(&lw.as_str()) {
            if let Some(season) = word(i + 1).and_then(number) {
                let mut episode = None;
                if let Some(next) = word(i + 2) {
                    if EPISODE_WORDS.contains(&norm_lower(next).as_str()) {
                        episode = word(i + 3).and_then(number);
                    } else {
                        episode = episode_token(next);
                    }
                }
                return Some((i, Some(season), episode));
            }
        }
        if EPISODE_WORDS.contains(&lw.as_str()) {
            if let Some(episode) = word(i + 1).and_then(number) {
                return Some((i, None, Some(episode)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(raw: &str) -> ParsedTitle {
        parse_source_title(raw, TitleKind::Movie)
    }
    fn series(raw: &str) -> ParsedTitle {
        parse_source_title(raw, TitleKind::Series)
    }
    fn live(raw: &str) -> ParsedTitle {
        parse_source_title(raw, TitleKind::Live)
    }

    #[test]
    fn the_audit_example_keeps_every_clue_separately() {
        let p = movie("FR | OPPENHEIMER [4K] MULTI (2023)");
        assert_eq!(p.title, "OPPENHEIMER");
        assert_eq!(p.year, Some(2023));
        assert_eq!(p.country_prefix.as_deref(), Some("FR"));
        assert_eq!(p.quality_hints, vec!["4K"]);
        assert_eq!(p.language_hints, vec!["MULTI"]);
    }

    #[test]
    fn provider_prefixes_in_every_common_shape() {
        for (raw, code, title) in [
            ("FR| The Matrix", Some("FR"), "The Matrix"),
            ("FR | The Matrix", Some("FR"), "The Matrix"),
            ("|AR| MBC 1", Some("AR"), "MBC 1"),
            ("SRS-FR| HBO", Some("FR"), "HBO"),
            ("FR| US| Inception", Some("FR"), "Inception"),
            ("VOD | NF-FR | Lupin", Some("FR"), "Lupin"),
            ("UK: The Office (US)", Some("UK"), "The Office (US)"),
            ("FR - NOVO 19", Some("FR"), "NOVO 19"),
            ("EU| FRANCE GENERAL", None, "FRANCE GENERAL"),
        ] {
            let p = live(raw);
            assert_eq!(
                (p.country_prefix.as_deref(), p.title.as_str()),
                (code, title),
                "{raw}"
            );
        }
    }

    #[test]
    fn real_words_that_look_like_prefixes_survive() {
        assert_eq!(
            movie("Mission: Impossible - Dead Reckoning").title,
            "Mission: Impossible - Dead Reckoning"
        );
        assert_eq!(
            movie("Up: A Story").title,
            "Up: A Story",
            "UP is not a known code"
        );
        assert_eq!(
            movie("Tom | Jerry").title,
            "Tom Jerry",
            "not a tag segment, pipe dropped"
        );
        assert_eq!(movie("Movie |").title, "Movie");
    }

    #[test]
    fn numbers_that_belong_to_titles_survive() {
        let p = movie("Blade Runner 2049 (2017) 4K");
        assert_eq!(
            (p.title.as_str(), p.year),
            ("Blade Runner 2049", Some(2017))
        );
        let p = movie("Blade Runner 2049");
        assert_eq!((p.title.as_str(), p.year), ("Blade Runner 2049", None));
        let p = movie("1917 (2019)");
        assert_eq!((p.title.as_str(), p.year), ("1917", Some(2019)));
        let p = movie("1917");
        assert_eq!(
            (p.title.as_str(), p.year),
            ("1917", None),
            "a lone number is the title"
        );
        assert_eq!(
            movie("2001: A Space Odyssey").title,
            "2001: A Space Odyssey"
        );
        assert_eq!(movie("Rocky 2").title, "Rocky 2");
        let p = movie("VOD| The Matrix 1999 1080p");
        assert_eq!((p.title.as_str(), p.year), ("The Matrix", Some(1999)));
    }

    #[test]
    fn out_of_range_parenthesized_numbers_are_not_years() {
        let p = movie("Channel (1)");
        assert_eq!((p.title.as_str(), p.year), ("Channel (1)", None));
        let p = movie("Movie (3000)");
        assert_eq!(p.year, None);
    }

    #[test]
    fn scene_names_are_split_but_acronyms_are_not() {
        let p = series("FR| BREAKING.BAD.S03E07.1080P.MULTI");
        assert_eq!(p.title, "BREAKING BAD");
        assert_eq!((p.season, p.episode), (Some(3), Some(7)));
        assert_eq!(p.quality_hints, vec!["1080P"]);
        assert_eq!(p.language_hints, vec!["MULTI"]);
        assert_eq!(movie("S.W.A.T.").title, "S.W.A.T");
        assert_eq!(movie("Some Movie H.264 AAC").title, "Some Movie");
    }

    #[test]
    fn episode_numbering_in_several_languages() {
        for (raw, title, season, episode) in [
            ("Mon Comeback S01 E04", "Mon Comeback", Some(1), Some(4)),
            ("Show s2e10", "Show", Some(2), Some(10)),
            ("Show 1x07", "Show", Some(1), Some(7)),
            ("Game of Thrones Season 8", "Game of Thrones", Some(8), None),
            ("Dark Saison 2 Episode 3", "Dark", Some(2), Some(3)),
            ("مسلسل الحفرة الموسم 2 الحلقة 5", "الحفرة", Some(2), Some(5)),
            ("DE | Tatort - Folge 1234", "Tatort", None, Some(1234)),
            ("S.W.A.T. 2017 S05E01", "S.W.A.T", Some(5), Some(1)),
        ] {
            let p = series(raw);
            assert_eq!(
                (p.title.as_str(), p.season, p.episode),
                (title, season, episode),
                "{raw}"
            );
        }
        assert_eq!(series("S.W.A.T. 2017 S05E01").year, Some(2017));
    }

    #[test]
    fn missing_numbering_stays_unknown() {
        let p = series("Breaking Bad");
        assert_eq!((p.season, p.episode), (None, None));
        // A name starting with S+digits is the name, not numbering.
        assert_eq!(series("S3 Cats").title, "S3 Cats");
    }

    #[test]
    fn distinguishing_groups_are_kept_and_tag_groups_become_hints() {
        assert_eq!(movie("The Office (US)").title, "The Office (US)");
        assert_eq!(
            movie("Blade Runner (Director's Cut)").title,
            "Blade Runner (Director's Cut)"
        );
        let p = movie("Dune [MULTI] [HEVC] FHD");
        assert_eq!(p.title, "Dune");
        assert_eq!(p.language_hints, vec!["MULTI"]);
        assert_eq!(p.quality_hints, vec!["HEVC", "FHD"]);
    }

    #[test]
    fn live_channels_keep_numbers_and_never_become_empty() {
        assert_eq!(live("beIN Sports 1 HD").title, "beIN Sports 1");
        assert_eq!(live("HBO ᴴᴰ").title, "HBO");
        assert_eq!(live("HD").title, "HD");
        let p = live("Channel 2019");
        assert_eq!((p.title.as_str(), p.year), ("Channel 2019", None));
    }

    #[test]
    fn non_latin_titles_pass_through() {
        assert_eq!(live("AR | رحلة 133 FHD").title, "رحلة 133");
        assert_eq!(movie("Été 4 HEVC").title, "Été 4");
    }

    #[test]
    fn parsing_is_deterministic_and_idempotent_on_its_output() {
        for raw in [
            "FR | OPPENHEIMER [4K] MULTI (2023)",
            "UK: The Office (US)",
            "Mission: Impossible - Dead Reckoning",
            "Blade Runner 2049",
        ] {
            let first = movie(raw);
            assert_eq!(first, movie(raw));
            assert_eq!(movie(&first.title).title, first.title, "{raw}");
        }
    }
}
