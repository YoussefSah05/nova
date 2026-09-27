//! Channel-logo directory: the public-domain iptv-org database
//! (<https://github.com/iptv-org/api>), used — opt-in — to fill logos a
//! provider and its EPG did not supply.
//!
//! Downloaded as a whole (never queried per channel), reduced to a compact
//! index kept on device, and matched locally:
//!
//! 1. **tvg-id**: many playlists and EPGs use iptv-org ids (`CNN.us`, with an
//!    optional `@feed` suffix). Exact, case-insensitive. Confidence 1.0.
//! 2. **Name**: the channel's quality-stripped, number-preserving name key
//!    (`nova_core::clean::dedup_key`) equals the key of the directory name or
//!    an alternative name of *exactly one* channel — within the channel's
//!    country when known (0.9), worldwide otherwise (0.75). Open channels win
//!    over closed ones; anything still ambiguous is left unmatched rather than
//!    guessed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Bump when the compact format or the logo choice changes; an older index
/// file is rebuilt.
pub const DIRECTORY_FORMAT: u32 = 1;
pub const CHANNELS_URL: &str = "https://iptv-org.github.io/api/channels.json";
pub const LOGOS_URL: &str = "https://iptv-org.github.io/api/logos.json";

/// Confidence below which a name match is not used.
pub const MIN_CONFIDENCE: f32 = 0.7;

#[derive(Debug, Deserialize)]
struct RawChannel {
    id: String,
    name: String,
    #[serde(default)]
    alt_names: Vec<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    closed: Option<String>,
    #[serde(default)]
    is_nsfw: bool,
}

#[derive(Debug, Deserialize)]
struct RawLogo {
    channel: String,
    #[serde(default)]
    feed: Option<String>,
    #[serde(default)]
    in_use: Option<bool>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    format: Option<String>,
    url: String,
}

/// One directory channel that has a usable logo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    pub logo: String,
}

/// The compact on-device index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Directory {
    pub format: u32,
    pub built_at: i64,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, thiserror::Error)]
#[error("channel directory: {0}")]
pub struct DirectoryError(String);

/// Logo preference: in use, the channel's own (not a feed's), raster, width.
type LogoRank = (bool, bool, bool, i64);

/// Build the index from the raw `channels.json` and `logos.json` bodies.
/// Channels without a usable logo and adult channels are left out.
pub fn build(
    channels_json: &str,
    logos_json: &str,
    built_at: i64,
) -> Result<Directory, DirectoryError> {
    let channels: Vec<RawChannel> =
        serde_json::from_str(channels_json).map_err(|e| DirectoryError(e.to_string()))?;
    let logos: Vec<RawLogo> =
        serde_json::from_str(logos_json).map_err(|e| DirectoryError(e.to_string()))?;

    // Best logo per channel: in use, the channel's own (not a feed's), raster
    // before SVG, then the widest.
    let mut best: HashMap<&str, (&RawLogo, LogoRank)> = HashMap::new();
    for logo in &logos {
        let Some(url) = nova_core::artwork::plausible_image_url(&logo.url) else {
            continue;
        };
        if url != logo.url {
            continue; // keep the directory's URL verbatim
        }
        let svg = logo
            .format
            .as_deref()
            .is_some_and(|f| f.eq_ignore_ascii_case("svg"));
        let rank = (
            logo.in_use.unwrap_or(true),
            logo.feed.is_none(),
            !svg,
            logo.width.unwrap_or(0.0) as i64,
        );
        match best.get(logo.channel.as_str()) {
            Some((_, r)) if *r >= rank => {}
            _ => {
                best.insert(&logo.channel, (logo, rank));
            }
        }
    }
    let entries = channels
        .into_iter()
        .filter(|c| !c.is_nsfw)
        .filter_map(|c| {
            let (logo, _) = best.get(c.id.as_str())?;
            Some(DirectoryEntry {
                logo: logo.url.clone(),
                id: c.id,
                name: c.name,
                alt_names: c.alt_names,
                country: c.country.map(|s| match s.to_ascii_uppercase().as_str() {
                    "GB" => "UK".to_string(),
                    other => other.to_string(),
                }),
                closed: c.closed.is_some(),
            })
        })
        .collect();
    Ok(Directory {
        format: DIRECTORY_FORMAT,
        built_at,
        entries,
    })
}

/// A channel to find a logo for.
#[derive(Debug, Clone)]
pub struct ChannelQuery {
    /// Caller's key for the result (echoed back).
    pub key: String,
    pub tvg_id: Option<String>,
    pub name: String,
    /// Provider country code, as imported.
    pub country: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMethod {
    TvgId,
    Name,
}

impl MatchMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TvgId => "tvg_id",
            Self::Name => "name",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryMatch {
    pub key: String,
    pub directory_id: String,
    pub logo_url: String,
    pub method: MatchMethod,
    pub confidence: f32,
}

/// The directory country an IPTV country code stands for, when unambiguous.
/// iptv-org writes Great Britain as `UK` (never `GB`); IPTV providers use `AR`
/// for *Arabic* content (not Argentina) and language-like codes that are not
/// countries.
pub fn query_country(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();
    match code.as_str() {
        "GB" | "UK" => Some("UK".into()),
        "AR" | "EN" | "EU" | "INT" | "LAT" | "EXYU" => None,
        c if c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()) => Some(code),
        _ => None,
    }
}

/// The key a channel's directory logo is stored under: its tvg-id, else
/// `name:` + its name, ASCII-lowercased (SQLite's `lower()` folding, which the
/// channel read path applies to the same expression).
pub fn channel_key(tvg_id: Option<&str>, name: &str) -> String {
    match tvg_id.filter(|t| !t.is_empty()) {
        Some(t) => t.to_ascii_lowercase(),
        None => format!("name:{}", name.to_ascii_lowercase()),
    }
}

/// The directory id a tvg-id names (`CNN.us@HD` → `cnn.us`), lowercased.
fn tvg_directory_key(tvg_id: &str) -> String {
    tvg_id
        .split('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Match channels against the directory (see the module docs).
pub fn match_channels(directory: &Directory, channels: &[ChannelQuery]) -> Vec<DirectoryMatch> {
    let mut by_id: HashMap<String, &DirectoryEntry> = HashMap::new();
    let mut by_name: HashMap<String, Vec<&DirectoryEntry>> = HashMap::new();
    for e in &directory.entries {
        by_id.entry(e.id.to_ascii_lowercase()).or_insert(e);
        let mut keys: Vec<String> = std::iter::once(&e.name)
            .chain(&e.alt_names)
            .map(|n| nova_core::clean::dedup_key(n))
            .filter(|k| !k.is_empty())
            .collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            by_name.entry(k).or_default().push(e);
        }
    }

    let found = |q: &ChannelQuery, e: &DirectoryEntry, method, confidence| DirectoryMatch {
        key: q.key.clone(),
        directory_id: e.id.clone(),
        logo_url: e.logo.clone(),
        method,
        confidence,
    };
    let mut out = Vec::new();
    for q in channels {
        if let Some(e) = q
            .tvg_id
            .as_deref()
            .map(tvg_directory_key)
            .filter(|k| !k.is_empty())
            .and_then(|k| by_id.get(&k))
        {
            out.push(found(q, e, MatchMethod::TvgId, 1.0));
            continue;
        }
        let key = nova_core::clean::dedup_key(&q.name);
        let Some(candidates) = by_name.get(&key).filter(|_| !key.is_empty()) else {
            continue;
        };
        let country = q.country.as_deref().and_then(query_country);
        let (pool, confidence): (Vec<&DirectoryEntry>, f32) = match &country {
            Some(c) => (
                candidates
                    .iter()
                    .copied()
                    .filter(|e| e.country.as_deref() == Some(c.as_str()))
                    .collect(),
                0.9,
            ),
            None => (candidates.clone(), 0.75),
        };
        let open: Vec<&DirectoryEntry> = pool.iter().copied().filter(|e| !e.closed).collect();
        let unique = match (open.len(), pool.len()) {
            (1, _) => Some(open[0]),
            (0, 1) => Some(pool[0]),
            _ => None,
        };
        if let Some(e) = unique.filter(|_| confidence >= MIN_CONFIDENCE) {
            out.push(found(q, e, MatchMethod::Name, confidence));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNELS: &str = r#"[
      {"id": "CNN.us", "name": "CNN", "alt_names": ["Cable News Network"], "country": "US", "closed": null, "is_nsfw": false},
      {"id": "BBCOne.uk", "name": "BBC One", "alt_names": [], "country": "UK", "closed": null, "is_nsfw": false},
      {"id": "beINSports1.qa", "name": "beIN Sports 1", "alt_names": [], "country": "QA", "closed": null, "is_nsfw": false},
      {"id": "beINSports2.qa", "name": "beIN Sports 2", "alt_names": [], "country": "QA", "closed": null, "is_nsfw": false},
      {"id": "M6.fr", "name": "M6", "alt_names": [], "country": "FR", "closed": null, "is_nsfw": false},
      {"id": "Canal1.co", "name": "Canal 1", "alt_names": [], "country": "CO", "closed": null, "is_nsfw": false},
      {"id": "Canal1.es", "name": "Canal 1", "alt_names": [], "country": "ES", "closed": null, "is_nsfw": false},
      {"id": "OldNews.fr", "name": "Info", "alt_names": [], "country": "FR", "closed": "2019-01-01", "is_nsfw": false},
      {"id": "NewNews.fr", "name": "Info", "alt_names": [], "country": "FR", "closed": null, "is_nsfw": false},
      {"id": "NoLogo.fr", "name": "No Logo", "alt_names": [], "country": "FR", "closed": null, "is_nsfw": false},
      {"id": "Adult.xx", "name": "Adult", "alt_names": [], "country": "US", "closed": null, "is_nsfw": true}
    ]"#;
    const LOGOS: &str = r#"[
      {"channel": "CNN.us", "feed": "HD", "in_use": true, "width": 1000, "format": "PNG", "url": "https://l/cnn-feed.png"},
      {"channel": "CNN.us", "feed": null, "in_use": true, "width": 512, "format": "PNG", "url": "https://l/cnn.png"},
      {"channel": "CNN.us", "feed": null, "in_use": true, "width": 900, "format": "SVG", "url": "https://l/cnn.svg"},
      {"channel": "BBCOne.uk", "feed": null, "in_use": false, "width": 900, "format": "PNG", "url": "https://l/bbc-old.png"},
      {"channel": "BBCOne.uk", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/bbc.png"},
      {"channel": "beINSports1.qa", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/bein1.png"},
      {"channel": "beINSports2.qa", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/bein2.png"},
      {"channel": "M6.fr", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/m6.png"},
      {"channel": "Canal1.co", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/c1co.png"},
      {"channel": "Canal1.es", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/c1es.png"},
      {"channel": "OldNews.fr", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/old.png"},
      {"channel": "NewNews.fr", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/new.png"},
      {"channel": "NoLogo.fr", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "not a url"},
      {"channel": "Adult.xx", "feed": null, "in_use": true, "width": 300, "format": "PNG", "url": "https://l/x.png"}
    ]"#;

    fn dir() -> Directory {
        build(CHANNELS, LOGOS, 1).unwrap()
    }

    fn q(key: &str, tvg: Option<&str>, name: &str, country: Option<&str>) -> ChannelQuery {
        ChannelQuery {
            key: key.into(),
            tvg_id: tvg.map(str::to_owned),
            name: name.into(),
            country: country.map(str::to_owned),
        }
    }

    fn logo_for(matches: &[DirectoryMatch], key: &str) -> Option<String> {
        matches
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.logo_url.clone())
    }

    #[test]
    fn builds_a_compact_index_with_the_best_logo_per_channel() {
        let d = dir();
        let ids: Vec<&str> = d.entries.iter().map(|e| e.id.as_str()).collect();
        assert!(!ids.contains(&"NoLogo.fr"), "no usable logo");
        assert!(!ids.contains(&"Adult.xx"), "adult channels are left out");
        let logo = |id: &str| d.entries.iter().find(|e| e.id == id).unwrap().logo.clone();
        assert_eq!(
            logo("CNN.us"),
            "https://l/cnn.png",
            "own raster logo over feed/SVG"
        );
        assert_eq!(logo("BBCOne.uk"), "https://l/bbc.png", "in-use logo first");
        assert_eq!(
            d.entries
                .iter()
                .find(|e| e.id == "BBCOne.uk")
                .unwrap()
                .country
                .as_deref(),
            Some("UK")
        );
        let round: Directory = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(round, d);
    }

    #[test]
    fn matches_by_tvg_id_first() {
        let m = match_channels(
            &dir(),
            &[
                q("a", Some("cnn.us@SD"), "Some Other Name", None),
                q("b", Some("unknown.id"), "M6 HD", Some("FR")),
            ],
        );
        assert_eq!(m[0].method, MatchMethod::TvgId);
        assert_eq!(m[0].confidence, 1.0);
        assert_eq!(logo_for(&m, "a").as_deref(), Some("https://l/cnn.png"));
        assert_eq!(
            m[1].method,
            MatchMethod::Name,
            "unknown tvg-id falls back to the name"
        );
        assert_eq!(logo_for(&m, "b").as_deref(), Some("https://l/m6.png"));
    }

    #[test]
    fn name_matches_keep_numbers_and_need_a_unique_candidate() {
        let m = match_channels(
            &dir(),
            &[
                q("bein2", None, "beIN Sports 2 FHD", Some("AR")),
                q("canal-co", None, "Canal 1", Some("CO")),
                q("canal-any", None, "Canal 1", None),
                q("cnn-alt", None, "Cable News Network", Some("US")),
                q("info", None, "Info", Some("FR")),
                q("wrong-country", None, "M6", Some("DE")),
                q("bbc", None, "BBC One HD", Some("UK")),
            ],
        );
        assert_eq!(
            logo_for(&m, "bein2").as_deref(),
            Some("https://l/bein2.png"),
            "AR (Arabic) is no country; the number keeps 1 and 2 apart"
        );
        assert_eq!(
            logo_for(&m, "canal-co").as_deref(),
            Some("https://l/c1co.png")
        );
        assert_eq!(logo_for(&m, "canal-any"), None, "ambiguous worldwide");
        assert_eq!(
            logo_for(&m, "cnn-alt").as_deref(),
            Some("https://l/cnn.png")
        );
        assert_eq!(
            logo_for(&m, "info").as_deref(),
            Some("https://l/new.png"),
            "open wins"
        );
        assert_eq!(logo_for(&m, "wrong-country"), None);
        assert_eq!(logo_for(&m, "bbc").as_deref(), Some("https://l/bbc.png"));
    }

    #[test]
    fn iptv_country_codes_are_normalized() {
        assert_eq!(query_country("gb").as_deref(), Some("UK"));
        assert_eq!(query_country("UK").as_deref(), Some("UK"));
        assert_eq!(query_country("FR").as_deref(), Some("FR"));
        assert_eq!(query_country("AR"), None);
        assert_eq!(query_country("EXYU"), None);
    }
}
