//! Category normalization — turn noisy provider `group-title` strings into
//! clean, deduplicated, canonical category labels for browsing and filtering.
//!
//! IPTV providers ship short, multilingual, inconsistent labels:
//! `"DE| GERMANY MOVIES"`, `"FR| -ACTION"`, `"SRS-FR| HBO"`, `"AR| اكشن"`. The
//! browse filters surface these raw, which feels primitive (hundreds of
//! near-duplicate entries). [`normalize_category`] strips the country/junk
//! prefix, then maps the common cases to a **canonical genre** via a curated,
//! multilingual keyword taxonomy; anything unrecognized is cleaned (title-cased,
//! acronyms preserved) and kept as-is.
//!
//! Why not an ML classifier: these labels are too terse and noisy for a model
//! to beat curated keywords reliably, and a model would add a runtime + weights
//! for little gain. [`canonical_genre`] is the single seam where a probabilistic
//! or ML backend could later slot in if ever justified.

/// Normalize a raw provider `group-title` into a clean category label.
///
/// Returns an empty string for empty input. Recognized genres collapse to a
/// canonical name (e.g. `"GERMANY MOVIES"` → `"Movies"`, `"COMEDIE"` →
/// `"Comedy"`); unrecognized labels are cleaned and title-cased.
pub fn normalize_category(raw_group: &str) -> String {
    let cleaned = clean_label(raw_group);
    if cleaned.is_empty() {
        return String::new();
    }
    if is_adult(&cleaned) {
        return "Adult".to_string();
    }
    if let Some(genre) = canonical_genre(&cleaned) {
        return genre.to_string();
    }
    title_case(&cleaned)
}

/// Strip a leading country/region prefix (`"DE| "`, `"SRS-FR| "`) and leading
/// separators, and collapse whitespace.
fn clean_label(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(pipe) = s.find('|') {
        let prefix = s[..pipe].trim().trim_start_matches("SRS-");
        if (2..=3).contains(&prefix.len()) && prefix.chars().all(|c| c.is_ascii_uppercase()) {
            s = &s[pipe + 1..];
        }
    }
    let s = s.trim().trim_start_matches(['-', '|', ':']).trim();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_adult(label: &str) -> bool {
    let l = label.to_lowercase();
    ["xxx", "porn", "adult", "+18", "18+", "porno", "erotic"]
        .iter()
        .any(|k| l.contains(k))
}

/// Curated genre taxonomy. Ordered **specific → generic** so that e.g.
/// `"ACTION MOVIES"` maps to `Action` (specific) rather than `Movies` (generic).
/// Aliases are lowercased substrings; multilingual variants included.
const TAXONOMY: &[(&str, &[&str])] = &[
    // Specific film/TV genres first.
    ("Action", &["action", "اكشن", "akshن"]),
    (
        "Comedy",
        &["comedy", "comedie", "comédie", "comedia", "كوميدي"],
    ),
    ("Horror", &["horror", "horreur", "terror", "رعب"]),
    ("Thriller", &["thriller", "suspense"]),
    (
        "Romance",
        &["romance", "romantic", "romantique", "romantik"],
    ),
    (
        "Animation",
        &["animation", "anime", "animated", "animacion"],
    ),
    (
        "Sci-Fi",
        &["sci-fi", "scifi", "science fiction", "science-fiction"],
    ),
    ("Fantasy", &["fantasy", "fantastique", "fantastico"]),
    ("Crime", &["crime", "polar", "policier"]),
    ("Adventure", &["adventure", "aventure", "aventura"]),
    ("Family", &["family", "famille", "familia"]),
    ("War", &["war", "guerre", "guerra"]),
    ("Western", &["western"]),
    ("Drama", &["drama", "drame", "دراما"]),
    // Topical channel groups.
    (
        "Sports",
        &[
            "sport",
            "deporte",
            "futbol",
            "football",
            "calcio",
            "bein",
            "espn",
            "nba",
            "nfl",
            "ufc",
            "motogp",
            "formula",
            "رياضة",
            "tennis",
            "golf",
        ],
    ),
    (
        "News",
        &[
            "news",
            "noticias",
            "info",
            "actualit",
            "nachrichten",
            "اخبار",
            "akhbar",
        ],
    ),
    (
        "Kids",
        &[
            "kids",
            "kid",
            "enfant",
            "infantil",
            "niños",
            "ninos",
            "cartoon",
            "dibujos",
            "baby",
            "اطفال",
            "atfal",
            "junior",
        ],
    ),
    (
        "Documentary",
        &[
            "document",
            "docu",
            "discovery",
            "nat geo",
            "natgeo",
            "documentaire",
            "documental",
            "وثائقي",
        ],
    ),
    (
        "Music",
        &["music", "musique", "musica", "mtv", "clips", "موسيقى"],
    ),
    (
        "Religion",
        &[
            "religion", "islam", "quran", "coran", "church", "gospel", "قرآن", "دين",
        ],
    ),
    // Generic catch-alls last.
    (
        "Movies",
        &[
            "movie",
            "movies",
            "vod",
            "film",
            "films",
            "cine",
            "cinema",
            "cinéma",
            "pelicula",
            "peliculas",
            "أفلام",
            "aflam",
        ],
    ),
    (
        "Series",
        &[
            "series",
            "serie",
            "séries",
            "shows",
            "tv show",
            "مسلسلات",
            "mosalsalat",
        ],
    ),
    (
        "Entertainment",
        &[
            "entertainment",
            "divertissement",
            "general",
            "général",
            "generale",
            "variety",
            "lifestyle",
        ],
    ),
];

/// Best-effort canonical genre for a cleaned label. Substring match against the
/// ordered taxonomy. `None` when nothing matches (caller keeps the cleaned label).
pub fn canonical_genre(cleaned_label: &str) -> Option<&'static str> {
    let l = cleaned_label.to_lowercase();
    for (genre, aliases) in TAXONOMY {
        if aliases.iter().any(|a| l.contains(a)) {
            return Some(genre);
        }
    }
    None
}

/// Title-case a label while preserving short all-caps acronyms (HBO, MBC, BBC).
fn title_case(label: &str) -> String {
    label
        .split_whitespace()
        .map(|word| {
            let is_acronym = word.len() <= 4
                && word
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_uppercase());
            if is_acronym {
                return word.to_string();
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_country_prefix_and_titlecases_unknown() {
        assert_eq!(normalize_category("SRS-FR| HBO"), "HBO");
        assert_eq!(normalize_category("DE| Sky Atlantic"), "Sky Atlantic");
    }

    #[test]
    fn maps_generic_movie_and_series_labels() {
        assert_eq!(normalize_category("DE| GERMANY MOVIES"), "Movies");
        assert_eq!(normalize_category("EN| TV SHOWS"), "Series");
        assert_eq!(normalize_category("VOD"), "Movies");
    }

    #[test]
    fn specific_genre_beats_generic() {
        // "ACTION MOVIES" should classify as Action, not Movies.
        assert_eq!(normalize_category("FR| ACTION MOVIES"), "Action");
        assert_eq!(normalize_category("ES| COMEDIA"), "Comedy");
        assert_eq!(normalize_category("FR| -COMÉDIE"), "Comedy");
    }

    #[test]
    fn topical_groups() {
        assert_eq!(normalize_category("AR| BeIN SPORTS 1"), "Sports");
        assert_eq!(normalize_category("FR| INFO 24/7"), "News");
        assert_eq!(normalize_category("KIDS & CARTOON"), "Kids");
        assert_eq!(normalize_category("Discovery Channel"), "Documentary");
    }

    #[test]
    fn adult_flagged() {
        assert_eq!(normalize_category("XXX| Adult 18+"), "Adult");
        assert_eq!(normalize_category("EN| PORN HD"), "Adult");
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(normalize_category(""), "");
        assert_eq!(normalize_category("FR| "), "");
    }

    #[test]
    fn acronyms_preserved() {
        assert_eq!(normalize_category("US| CNN"), "CNN");
        assert_eq!(normalize_category("UK| BBC One"), "BBC One");
    }
}
