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

#[cfg(test)]
mod tests {
    use super::*;

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
