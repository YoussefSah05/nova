//! Source-scoped live identity: family membership never implies interchangeable streams.
use crate::clean::{parse_source_title, TitleKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const LIVE_RULESET: i64 = 1;

#[derive(Debug, Clone)]
pub struct LiveEvidence {
    pub raw_name: String,
    pub country: Option<String>,
    pub group: Option<String>,
    pub tvg_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveIdentity {
    pub family: String,
    pub family_title: String,
    pub channel_key: String,
    pub title: String,
    pub edition: String,
    pub language: Option<String>,
    pub country: Option<String>,
    pub quality: Option<String>,
    pub codecs: Vec<String>,
    pub number: Option<u32>,
    pub sort_key: String,
}

macro_rules! dto {
    ($name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[cfg_attr(test, derive(ts_rs::TS))]
        #[cfg_attr(test, ts(export, export_to = "../../../apps/desktop/src/types/generated/"))]
        pub struct $name { $($(#[$attr])* pub $field: $ty),* }
    };
}
dto!(LiveStreamVariant {
    variant_key: String, #[cfg_attr(test, ts(type = "number"))] channel_id: i64, raw_name: String, quality: Option<String>,
    codecs: Vec<String>, recently_failed: bool, #[cfg_attr(test, ts(type = "number | null"))] last_success_at: Option<i64>,
});
dto!(LiveChannelSummary {
    key: String, #[cfg_attr(test, ts(type = "number"))] source_id: i64, family: String, title: String, edition: String,
    language: Option<String>, country: Option<String>, number: Option<u32>,
    #[cfg_attr(test, ts(type = "number"))] representative_id: i64, logo_url: Option<String>, category: Option<String>,
    region: Option<String>, is_favorite: bool, is_in_watchlist: bool,
    variants: Vec<LiveStreamVariant>,
});
dto!(LivePlaybackTarget {
    channel: LiveChannelSummary,
    #[cfg_attr(test, ts(type = "number"))]
    channel_id: i64,
    variant_key: String,
});

pub fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p.as_bytes());
    }
    format!("{:x}", h.finalize())
}
fn normalized(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn language_tag(token: &str) -> Option<&'static str> {
    match token.to_ascii_uppercase().as_str() {
        "EN" | "ENG" | "ENGLISH" => Some("EN"),
        "FR" | "FRA" | "FRENCH" | "VF" => Some("FR"),
        "AR" | "ARA" | "ARABIC" => Some("AR"),
        "ES" | "SPA" | "SPANISH" => Some("ES"),
        "DE" | "GER" | "GERMAN" => Some("DE"),
        "IT" | "ITA" | "ITALIAN" => Some("IT"),
        "PT" | "POR" | "PORTUGUESE" => Some("PT"),
        "TR" | "TUR" | "TURKISH" => Some("TR"),
        "NL" | "DUT" | "DUTCH" => Some("NL"),
        "RU" | "RUS" | "RUSSIAN" => Some("RU"),
        _ => None,
    }
}
fn prepared(e: &LiveEvidence) -> (String, Option<String>, Option<String>, Vec<String>) {
    let p = parse_source_title(&e.raw_name, TitleKind::Live);
    let mut langs = HashSet::new();
    let mut words = Vec::new();
    for w in p.title.split_whitespace() {
        // Explicit edition tags must be standalone. Never substring-match AR/EN.
        let token = w.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some(l) = language_tag(token) {
            langs.insert(l);
        } else {
            words.push(w);
        }
    }
    let language = if langs.len() == 1 {
        langs.iter().next().map(|s| s.to_string())
    } else if langs.len() > 1 {
        None
    } else {
        p.country_prefix
            .as_deref()
            .and_then(language_tag)
            .or_else(|| {
                e.group.as_deref().and_then(|g| {
                    let tags: HashSet<_> = g
                        .split(|c: char| !c.is_alphanumeric())
                        .filter_map(language_tag)
                        .collect();
                    (tags.len() == 1)
                        .then(|| tags.iter().next().copied())
                        .flatten()
                })
            })
            .map(str::to_string)
    };
    let title = if words.is_empty() {
        p.title
    } else {
        words.join(" ")
    };
    let quality = p
        .quality_hints
        .iter()
        .find_map(|q| match q.as_str() {
            "1080P" | "1080I" | "FHD" => Some("FHD"),
            "720P" | "HD" => Some("HD"),
            "UHD" | "4K" | "2160P" => Some("4K"),
            "SD" | "480P" | "576P" => Some("SD"),
            "8K" | "4320P" => Some("8K"),
            _ => None,
        })
        .map(str::to_string);
    let codecs = p
        .quality_hints
        .into_iter()
        .filter(|q| {
            matches!(
                q.as_str(),
                "HEVC" | "H265" | "X265" | "H264" | "X264" | "HDR" | "HDR10" | "DV"
            )
        })
        .collect();
    (title, language, quality, codecs)
}

/// Alias rules establish brands only. Remaining words still identify the service.
fn brand_alias(title: &str) -> Option<(&'static str, String)> {
    let words: Vec<_> = title.split_whitespace().collect();
    let mut prefix = String::new();
    for (i, w) in words.iter().enumerate().take(3) {
        prefix.push_str(&normalized(w).replace(' ', ""));
        let brand = match prefix.as_str() {
            "beinsport" | "beinsports" => "beIN Sports",
            "skysport" | "skysports" => "Sky Sports",
            "eurosport" => "Eurosport",
            "espn" => "ESPN",
            "mbc" => "MBC",
            _ => continue,
        };
        return Some((brand, words[i + 1..].join(" ")));
    }
    None
}
fn split_number(title: &str) -> (String, Option<u32>) {
    if let Some((base, last)) = title.rsplit_once(' ') {
        if let Ok(n) = last.parse::<u32>() {
            return (base.to_string(), Some(n));
        }
    }
    (title.to_string(), None)
}

pub fn compile(evidence: &[LiveEvidence]) -> Vec<LiveIdentity> {
    let prepared: Vec<_> = evidence.iter().map(prepared).collect();
    let mut siblings: HashMap<String, HashSet<u32>> = HashMap::new();
    for (title, ..) in &prepared {
        let (base, number) = split_number(title);
        if let Some(n) = number {
            siblings.entry(normalized(&base)).or_default().insert(n);
        }
    }
    evidence
        .iter()
        .zip(prepared)
        .map(|(e, (title, language, quality, codecs))| {
            let (base, number) = split_number(&title);
            let protected = matches!(
                normalized(&title).as_str(),
                "france 24" | "news 24" | "canal 24" | "2m" | "6ter" | "tf1"
            );
            let (family_title, display) = if let Some((brand, service)) = brand_alias(&title) {
                (
                    brand.to_string(),
                    if service.is_empty() {
                        brand.to_string()
                    } else {
                        format!("{brand} {service}")
                    },
                )
            } else if !protected
                && siblings
                    .get(&normalized(&base))
                    .is_some_and(|s| s.len() >= 2)
            {
                (base, title.clone())
            } else {
                (title.clone(), title.clone())
            };
            // Country is never guessed from a language-only tag such as AR or EN.
            let country = e
                .country
                .as_ref()
                .filter(|c| !matches!(c.as_str(), "AR" | "EN"))
                .map(|s| s.to_ascii_uppercase());
            let edition = match (&language, &country) {
                (Some(l), Some(c)) if l != c => format!("{l} · {c}"),
                (Some(l), _) => l.clone(),
                (None, Some(c)) => c.clone(),
                _ => "Unknown".to_string(),
            };
            // Unknown/conflicting metadata must not prove two streams equivalent.
            let unknown = if edition == "Unknown" {
                e.tvg_id.as_deref().unwrap_or(&e.raw_name)
            } else {
                ""
            };
            let family = normalized(&family_title);
            let channel_key = digest(&[
                "live-channel-v1",
                &family,
                &edition,
                &normalized(&display),
                unknown,
            ]);
            LiveIdentity {
                family,
                family_title,
                channel_key,
                title: display.clone(),
                edition,
                language,
                country,
                quality,
                codecs,
                sort_key: sort_key(&display),
                number: (!protected).then_some(number).flatten(),
            }
        })
        .collect()
}

pub fn quality_rank(quality: Option<&str>) -> u8 {
    match quality {
        Some("FHD") => 0,
        Some("HD") => 1,
        Some("4K") => 2,
        Some("SD") => 3,
        None => 4,
        _ => 5,
    }
}

fn sort_key(s: &str) -> String {
    let mut result = String::new();
    let mut digits = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            result.push_str(&format!("{:0>10}", digits));
            digits.clear();
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(s: &str) -> LiveEvidence {
        LiveEvidence {
            raw_name: s.into(),
            country: None,
            group: None,
            tvg_id: None,
        }
    }
    #[test]
    fn aliases_keep_services_editions_and_numbers() {
        let p = compile(&[
            ev("FR - BEINSPORT 1 SD"),
            ev("FR - beIN SPORTS 1 4K"),
            ev("AR - BEINSPORTS 1 EN HD"),
            ev("AR - BEINSPORTS 2 HD"),
            ev("FR - BEINSPORT MAX 4 HD"),
            ev("AR - BEINSPORTS NEWS HD"),
        ]);
        assert!(p.iter().all(|p| p.family == "bein sports"));
        assert_eq!(p[0].channel_key, p[1].channel_key);
        assert_ne!(p[0].channel_key, p[2].channel_key);
        assert_eq!(p[2].language.as_deref(), Some("EN"));
        assert_eq!(p[4].title, "beIN Sports MAX 4");
        assert_eq!(p[5].title, "beIN Sports NEWS");
    }
    #[test]
    fn siblings_and_protected_brands() {
        let p = compile(&[
            ev("France 2 HD"),
            ev("France 3 SD"),
            ev("France 24 HD"),
            ev("Zoom 1"),
            ev("Zoom 2"),
            ev("Zoom 1 G"),
        ]);
        assert_eq!(p[0].family, p[1].family);
        assert_eq!(p[2].family, "france 24");
        assert_eq!(p[3].family, p[4].family);
        assert_ne!(p[3].channel_key, p[5].channel_key);
    }
    #[test]
    fn decorative_quality_and_unknowns_survive() {
        let p = compile(&[
            ev("AR - BEIN SPORTS 1 ᴴᴰ G"),
            ev("AR - BEIN SPORTS 1 ᵁᴴᴰ G"),
            ev("Channel FR EN HD"),
            ev("WELLBEING TV HDR"),
        ]);
        assert_eq!(p[0].quality.as_deref(), Some("HD"));
        assert_eq!(p[1].quality.as_deref(), Some("4K"));
        assert!(p[0].title.ends_with(" G"));
        assert_eq!(p[2].language, None);
        assert_ne!(p[3].family, "bein sports");
    }
}
