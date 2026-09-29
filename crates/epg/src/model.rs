//! Domain types for parsed EPG programmes.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One scheduled programme on one channel. `start_utc` / `stop_utc` are
/// always normalized to UTC epoch seconds before construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Program {
    pub channel_tvg_id: String,
    pub title: String,
    pub sub_title: Option<String>,
    pub description: Option<String>,
    pub start_utc: i64,
    pub stop_utc: i64,
    pub category: Option<String>,
    pub icon_url: Option<String>,
}

/// The logo an XMLTV `<channel id>` declares with its first `<icon src>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelIcon {
    pub channel_id: String,
    pub icon_url: String,
}

#[derive(Debug, Error)]
pub enum EpgError {
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("XMLTV attribute error: {0}")]
    Attr(String),
    #[error("UTF-8 decode error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl EpgError {
    /// The byte stream failed (a truncated or corrupt download, a gzip error,
    /// a size cap): the parser repeats it forever, so parsing must stop.
    pub fn is_io(&self) -> bool {
        matches!(self, Self::Io(_) | Self::Xml(quick_xml::Error::Io(_)))
    }
}

/// Case-insensitive match between an M3U `tvg_id` and an XMLTV `channel.id`.
/// Used by the refresh job when stitching programmes to channels.
pub fn channel_id_matches(m3u: &str, xmltv: &str) -> bool {
    if m3u.eq_ignore_ascii_case(xmltv) {
        return true;
    }
    // Fallback: strip trailing `.<country>` domain (e.g. `bbc1.uk` vs `bbc1`).
    let strip = |s: &str| -> String {
        match s.rfind('.') {
            Some(i) if s.len() - i <= 4 => s[..i].to_string(),
            _ => s.to_string(),
        }
    };
    strip(m3u).eq_ignore_ascii_case(&strip(xmltv))
}

/// The lookup key of a channel's EPG logo: its `tvg-id`, ASCII-lowercased (the
/// same folding as SQLite's `lower()`, which the read path applies).
pub fn channel_logo_key(tvg_id: &str) -> String {
    tvg_id.to_ascii_lowercase()
}

/// Pair each channel `tvg-id` with the icon of the XMLTV channel it matches
/// under [`channel_id_matches`] (exact, case-insensitive first; then with a
/// trailing country domain stripped). Returns `(channel_logo_key, icon_url)`
/// once per key; the first icon declared for an id wins.
pub fn match_channel_icons(tvg_ids: &[String], icons: &[ChannelIcon]) -> Vec<(String, String)> {
    use std::collections::{HashMap, HashSet};
    let strip = |s: &str| -> String {
        match s.rfind('.') {
            Some(i) if s.len() - i <= 4 => s[..i].to_ascii_lowercase(),
            _ => s.to_ascii_lowercase(),
        }
    };
    let mut exact: HashMap<String, &str> = HashMap::new();
    let mut stripped: HashMap<String, &str> = HashMap::new();
    for icon in icons {
        exact
            .entry(icon.channel_id.to_ascii_lowercase())
            .or_insert(&icon.icon_url);
        stripped
            .entry(strip(&icon.channel_id))
            .or_insert(&icon.icon_url);
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tvg in tvg_ids {
        let key = channel_logo_key(tvg);
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        if let Some(url) = exact.get(&key).or_else(|| stripped.get(&strip(tvg))) {
            out.push((key, (*url).to_string()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(id: &str, url: &str) -> ChannelIcon {
        ChannelIcon {
            channel_id: id.into(),
            icon_url: url.into(),
        }
    }

    #[test]
    fn matches_icons_like_programmes_match_channels() {
        let icons = [
            icon("BBC1.uk", "https://l/bbc1.png"),
            icon("CNN", "https://l/cnn.png"),
            icon("cnn", "https://l/cnn-dup.png"),
        ];
        let tvg = [
            "bbc1".to_string(),
            "cnn".to_string(),
            "CNN".to_string(),
            "espn".to_string(),
            String::new(),
        ];
        assert_eq!(
            match_channel_icons(&tvg, &icons),
            vec![
                ("bbc1".to_string(), "https://l/bbc1.png".to_string()),
                ("cnn".to_string(), "https://l/cnn.png".to_string()),
            ]
        );
    }

    #[test]
    fn exact_match() {
        assert!(channel_id_matches("BBC1", "BBC1"));
    }

    #[test]
    fn case_insensitive() {
        assert!(channel_id_matches("bbc1", "BBC1"));
    }

    #[test]
    fn suffix_stripped() {
        assert!(channel_id_matches("bbc1", "BBC1.uk"));
        assert!(channel_id_matches("BBC1.uk", "bbc1"));
    }

    #[test]
    fn distinct_channels_no_match() {
        assert!(!channel_id_matches("CNN", "BBC1"));
    }
}
