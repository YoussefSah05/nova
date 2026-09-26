//! Deterministic candidate blocking — the bounded pre-step before entity
//! resolution.
//!
//! Groups source evidence into small candidate buckets by shared,
//! deterministic signals (a trusted external ID, a series relation, or
//! kind + normalized title, optionally narrowed by year/language) so that
//! later comparison work only ever runs *within* a bucket. Bucket
//! construction is one pass over the input, `O(n * k)` for a small bounded
//! `k` (keys per item) — never a pairwise scan of the whole library.
//!
//! Blocking never decides a match. It only bounds *who gets compared to
//! whom*; the match/no-match decision belongs to entity resolution (a later
//! milestone), which may use AI to disambiguate within a bucket but never
//! across the whole catalog and never as sole authority (`AI proposes ->
//! Nova validates`).

use std::collections::BTreeMap;
use std::fmt;

use super::{ContentKind, EpisodeNumbering};

/// One deterministic candidate-blocking key. Opaque on purpose: the encoding
/// is an internal clustering detail, not a durable identity or IPC value.
///
/// The encoded string embeds raw evidence (a normalized title, and — for a
/// trusted-ID key — the trusted ID's value verbatim, which may be a source's
/// provider-supplied upstream key). That is exactly the category of content
/// `UpstreamKey`/`SourceRecord` redact from `Debug` elsewhere in this module
/// family, so this type does too; use [`Self::as_str`] for the rare
/// legitimate need (persistence, algorithmic comparison), never `Debug`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockingKey(String);

impl BlockingKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BlockingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BlockingKey([redacted])")
    }
}

/// Bounded, normalized evidence used to compute candidate-blocking keys for
/// one piece of source evidence. Built by the caller from a `SourceRecord`
/// plus whatever trusted external IDs are already known for it (its upstream
/// key, a TMDB/IMDB ID, ...); blocking itself does no I/O and trusts nothing
/// beyond what it is given.
#[derive(Clone, Default)]
pub struct BlockingEvidence {
    pub kind: ContentKind,
    /// Raw display title. Normalized internally (folds case/junk/punctuation,
    /// keeps distinguishing numbers) so quality-tag variants of the same
    /// title still block together while genuinely distinct numbered entries
    /// (`Rocky` vs `Rocky 2`) never collapse.
    pub raw_title: String,
    pub year: Option<i32>,
    /// `(namespace, value)` pairs: a source's upstream key, or an external
    /// metadata ID such as `("tmdb", "603")`. The strongest signal — evidence
    /// sharing a trusted ID blocks together regardless of title/year drift.
    pub trusted_ids: Vec<(String, String)>,
    pub episode: Option<EpisodeNumbering>,
    /// A language tag as given by the caller (compared case-insensitively).
    pub language: Option<String>,
    /// Stable key of the parent series, when this evidence is an episode.
    pub series_key: Option<String>,
}

impl fmt::Debug for BlockingEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `raw_title` and `trusted_ids` are provider-supplied text (the same
        // category `SourceRecord`/`UpstreamKey` redact); every other field is
        // either Nova-derived (`series_key`) or non-identifying metadata.
        f.debug_struct("BlockingEvidence")
            .field("kind", &self.kind)
            .field("year", &self.year)
            .field("trusted_id_count", &self.trusted_ids.len())
            .field("episode", &self.episode)
            .field("language", &self.language)
            .field("series_key", &self.series_key)
            .finish_non_exhaustive()
    }
}

/// Upper bound on keys returned per item, so one malformed record (e.g. many
/// trusted IDs) cannot blow up bucket construction cost.
const MAX_KEYS_PER_ITEM: usize = 8;

fn kind_tag(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::LiveChannel => "live",
        ContentKind::Movie => "movie",
        ContentKind::Series => "series",
        ContentKind::Season => "season",
        ContentKind::Episode => "episode",
        ContentKind::Event => "event",
        ContentKind::Radio => "radio",
        ContentKind::Unknown => "unknown",
    }
}

/// Deterministic, bounded set of blocking keys for one evidence record.
/// Multiple keys raise recall (a record is discoverable via any shared
/// signal it carries) while each individual key still buckets tightly.
pub fn blocking_keys(evidence: &BlockingEvidence) -> Vec<BlockingKey> {
    let mut keys = Vec::with_capacity(4);

    for (namespace, value) in &evidence.trusted_ids {
        let namespace = namespace.trim();
        let value = value.trim();
        if !namespace.is_empty() && !value.is_empty() {
            keys.push(BlockingKey(format!("id:{namespace}:{value}")));
        }
    }

    if let Some(series_key) = evidence
        .series_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match evidence.episode {
            Some(EpisodeNumbering {
                season: Some(season),
                episode: Some(episode),
            }) => keys.push(BlockingKey(format!(
                "series:{series_key}:s{season}e{episode}"
            ))),
            _ => keys.push(BlockingKey(format!("series:{series_key}"))),
        }
    }

    let title = crate::clean::dedup_key(&evidence.raw_title);
    if !title.is_empty() {
        let kind = kind_tag(evidence.kind);
        let year = evidence
            .year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "?".into());
        keys.push(BlockingKey(format!("kt:{kind}:{title}:{year}")));
        if let Some(lang) = evidence
            .language
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            keys.push(BlockingKey(format!(
                "ktl:{kind}:{title}:{}",
                lang.to_lowercase()
            )));
        }
    }

    keys.sort();
    keys.dedup();
    keys.truncate(MAX_KEYS_PER_ITEM);
    keys
}

/// Group item identities into candidate buckets sharing at least one
/// blocking key. One pass over `items`; each item contributes at most
/// [`MAX_KEYS_PER_ITEM`] bucket memberships, so total work is `O(n)` in the
/// number of items — never a pairwise scan of the whole set.
pub fn build_candidate_blocks<K: Ord + Clone>(
    items: &[(K, BlockingEvidence)],
) -> BTreeMap<BlockingKey, Vec<K>> {
    let mut blocks: BTreeMap<BlockingKey, Vec<K>> = BTreeMap::new();
    for (id, evidence) in items {
        for key in blocking_keys(evidence) {
            blocks.entry(key).or_default().push(id.clone());
        }
    }
    blocks
}

/// Total candidate-pair count implied by a blocking result (each bucket of
/// size `n` contributes `n * (n - 1) / 2` pairs). Lets a caller check a
/// resolution work budget up front without ever materializing — or paying
/// for — the whole-library pairwise set.
pub fn candidate_pair_count<K>(blocks: &BTreeMap<BlockingKey, Vec<K>>) -> u64 {
    blocks
        .values()
        .map(|members| {
            let n = members.len() as u64;
            n.saturating_mul(n.saturating_sub(1)) / 2
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(kind: ContentKind, title: &str, year: Option<i32>) -> BlockingEvidence {
        BlockingEvidence {
            kind,
            raw_title: title.into(),
            year,
            ..Default::default()
        }
    }

    #[test]
    fn blocking_keys_are_deterministic() {
        let e = evidence(ContentKind::Movie, "Heat", Some(1995));
        assert_eq!(blocking_keys(&e), blocking_keys(&e));
    }

    #[test]
    fn diagnostics_do_not_echo_raw_title_or_trusted_id_values() {
        let mut e = evidence(ContentKind::Movie, "password=do-not-log", Some(1995));
        e.trusted_ids
            .push(("xtream-vod".into(), "do-not-log-either".into()));
        let debug = format!("{e:?}");
        assert!(!debug.contains("do-not-log"));

        for key in blocking_keys(&e) {
            assert!(!format!("{key:?}").contains("do-not-log"));
        }
    }

    #[test]
    fn quality_tag_variants_of_the_same_title_share_a_block() {
        let items = vec![
            (1u32, evidence(ContentKind::Movie, "Heat FHD", Some(1995))),
            (2u32, evidence(ContentKind::Movie, "Heat 1080p", Some(1995))),
        ];
        let blocks = build_candidate_blocks(&items);
        assert!(
            blocks.values().any(|members| members.len() == 2),
            "{blocks:?}"
        );
    }

    #[test]
    fn distinguishing_numbers_never_collapse_into_one_block() {
        let items = vec![
            (1u32, evidence(ContentKind::Movie, "Rocky", None)),
            (2u32, evidence(ContentKind::Movie, "Rocky 2", None)),
        ];
        let blocks = build_candidate_blocks(&items);
        assert!(blocks.values().all(|members| members.len() == 1));
    }

    #[test]
    fn a_shared_trusted_id_blocks_regardless_of_title_drift() {
        let mut a = evidence(ContentKind::Movie, "Le Film", Some(2020));
        a.trusted_ids.push(("tmdb".into(), "603".into()));
        let mut b = evidence(ContentKind::Movie, "The Movie", Some(2021));
        b.trusted_ids.push(("tmdb".into(), "603".into()));
        let items = vec![(1u32, a), (2u32, b)];
        let blocks = build_candidate_blocks(&items);
        assert!(blocks.values().any(|m| m.len() == 2));
    }

    #[test]
    fn different_trusted_id_namespaces_never_collide() {
        let mut a = evidence(ContentKind::Movie, "Film A", None);
        a.trusted_ids.push(("tmdb".into(), "1".into()));
        let mut b = evidence(ContentKind::Movie, "Film B", None);
        b.trusted_ids.push(("imdb".into(), "1".into()));
        let items = vec![(1u32, a), (2u32, b)];
        let blocks = build_candidate_blocks(&items);
        assert!(blocks.values().all(|m| m.len() == 1));
    }

    #[test]
    fn episodes_of_the_same_series_and_coordinate_block_together() {
        let mut a = evidence(ContentKind::Episode, "Show S01E02", None);
        a.series_key = Some("series-1".into());
        a.episode = Some(EpisodeNumbering {
            season: Some(1),
            episode: Some(2),
        });
        let mut b = evidence(ContentKind::Episode, "Show Episode Two", None);
        b.series_key = Some("series-1".into());
        b.episode = Some(EpisodeNumbering {
            season: Some(1),
            episode: Some(2),
        });
        let items = vec![(1u32, a), (2u32, b)];
        let blocks = build_candidate_blocks(&items);
        assert!(blocks.values().any(|m| m.len() == 2));
    }

    #[test]
    fn different_episode_coordinates_of_the_same_series_do_not_block_together() {
        let mut a = evidence(ContentKind::Episode, "Show S01E01", None);
        a.series_key = Some("series-1".into());
        a.episode = Some(EpisodeNumbering {
            season: Some(1),
            episode: Some(1),
        });
        let mut b = evidence(ContentKind::Episode, "Show S01E02", None);
        b.series_key = Some("series-1".into());
        b.episode = Some(EpisodeNumbering {
            season: Some(1),
            episode: Some(2),
        });
        let items = vec![(1u32, a), (2u32, b)];
        let blocks = build_candidate_blocks(&items);
        // Distinct series-coordinate keys never merge; a shared kind+title
        // key is still possible only if titles happen to normalize equal.
        assert!(!blocks
            .iter()
            .any(|(k, m)| k.as_str().starts_with("series:") && m.len() == 2));
    }

    #[test]
    fn different_media_kinds_never_share_a_title_year_block() {
        let items = vec![
            (1u32, evidence(ContentKind::Movie, "Chicago", Some(2002))),
            (
                2u32,
                evidence(ContentKind::LiveChannel, "Chicago", Some(2002)),
            ),
        ];
        let blocks = build_candidate_blocks(&items);
        assert!(blocks.values().all(|m| m.len() == 1));
    }

    #[test]
    fn evidence_with_no_signal_produces_no_keys_rather_than_a_giant_bucket() {
        let e = evidence(ContentKind::Movie, "HD", None); // title is pure junk
        assert!(blocking_keys(&e).is_empty());
    }

    #[test]
    fn candidate_generation_stays_far_below_whole_library_o_n_squared() {
        // 20,000 items across 500 distinct titles x 20 years: a realistic
        // worst case for a large imported catalog with many duplicate
        // provider listings of the same handful of titles.
        const TOTAL: usize = 20_000;
        const DISTINCT_TITLES: usize = 500;
        let mut items = Vec::with_capacity(TOTAL);
        for i in 0..TOTAL {
            let title = format!("Title {}", i % DISTINCT_TITLES);
            let year = 2000 + ((i / DISTINCT_TITLES) % 20) as i32;
            items.push((i, evidence(ContentKind::Movie, &title, Some(year))));
        }

        let start = std::time::Instant::now();
        let blocks = build_candidate_blocks(&items);
        let pairs = candidate_pair_count(&blocks);
        let elapsed = start.elapsed();

        let whole_library_pairs = (TOTAL as u64) * (TOTAL as u64 - 1) / 2;
        assert!(
            pairs < whole_library_pairs / 100,
            "blocking must cut comparisons by orders of magnitude: pairs={pairs} whole={whole_library_pairs}"
        );
        assert!(elapsed.as_secs() < 2, "blocking took too long: {elapsed:?}");
    }
}
