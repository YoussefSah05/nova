//! Deterministic canonical entity resolution.
//!
//! Decides which canonical entities are the same work, from bounded
//! deterministic evidence only. It is pure (no I/O) and conservative. A merge
//! needs positive evidence and no contradiction, and anything uncertain
//! abstains. A false merge corrupts user state and a missed merge only shows a
//! duplicate card, so the rules are biased against merging.
//!
//! Pipeline:
//!
//! 1. **Blocking** ([`super::blocking`]): only entities sharing a trusted ID,
//!    series coordinate or normalized title are ever compared. Oversized
//!    blocks abstain rather than fan out.
//! 2. **Hard contradictions** ([`contradiction`]): a different media kind,
//!    conflicting trusted IDs in one namespace, a different series or episode
//!    coordinate, clearly incompatible years, or a recorded cannot-link
//!    decision. A contradiction always rejects, whatever else matches.
//! 3. **Rules** ([`assess_pair`]): a shared trusted ID corroborated by a
//!    title or year, identical series coordinates, or an identical normalized
//!    title *and* an identical year. Everything else abstains.
//! 4. **Conflict-safe clustering**: an entity whose merge partners contradict
//!    each other is ambiguous and abstains. Components merge only while every
//!    cross-pair stays contradiction-free and bounded in size, so matches are
//!    never chained transitively into a false merge.
//! 5. **Survivor**: the oldest entity (then lowest ID) is kept, so historical
//!    IDs redirect deterministically.
//!
//! Only this module can construct a [`MergeDecision`], which is the sole input
//! storage accepts for an identity merge. A model can only review an
//! [`AmbiguityCase`] the resolver built from real, bounded candidates, via
//! [`assess_ai_verdict`], which yields an advisory [`AiSuggestion`]. There is
//! no conversion from a suggestion to a decision.
//!
//! ```compile_fail
//! use nova_core::catalog::resolution::{AiSuggestion, MergeDecision};
//! fn promote(s: AiSuggestion) -> MergeDecision { s.into() }
//! ```
//!
//! ```compile_fail
//! use nova_core::catalog::{EntityId, resolution::{MergeDecision, MergeRule}};
//! let forged = MergeDecision {
//!     survivor: EntityId::new("a").unwrap(),
//!     absorbed: EntityId::new("b").unwrap(),
//!     partner: EntityId::new("a").unwrap(),
//!     rule: MergeRule::TrustedId,
//! };
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::blocking::{blocking_keys, BlockingEvidence};
use super::intelligence::IntelligenceError;
use super::{ContentKind, EntityId, EpisodeNumbering};

/// Version of the deterministic rule set. Bump whenever a rule, threshold or
/// normalization changes. Persisted state then knows a re-resolution pass is
/// due, and lineage records which rules produced each merge.
pub const RULESET_VERSION: u32 = 1;

/// Years further apart than this are a hard contradiction. One year of slack
/// covers festival-versus-release and regional release dates.
pub const YEAR_TOLERANCE: i32 = 1;

/// A block larger than this is too ambiguous to resolve deterministically
/// (a generic title such as "Home"). All its members abstain.
pub const MAX_BLOCK_SIZE: usize = 64;

/// Upper bound on one resolved cluster. A merge that would exceed it abstains.
pub const MAX_CLUSTER_SIZE: usize = 32;

/// Upper bound on candidates in one [`AmbiguityCase`] offered to a model.
pub const MAX_AI_CANDIDATES: usize = 8;

/// Byte limit on one model verdict.
pub const MAX_AI_VERDICT_BYTES: usize = 4 * 1024;

pub const AI_PROTOCOL_VERSION: u16 = 1;

const MAX_TITLES_PER_ENTITY: usize = 16;

/// An external identifier from a trusted authority, namespaced by authority
/// **and** media kind (`tmdb:movie`), so a TMDB movie ID never collides with a
/// TMDB TV ID of the same number.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TrustedId {
    namespace: String,
    value: String,
}

impl TrustedId {
    /// `namespace` is lowercase `[a-z0-9:_-]`; `value` is a bounded ASCII
    /// alphanumeric token. Anything else is rejected, never coerced.
    pub fn new(namespace: &str, value: &str) -> Option<Self> {
        let ns_ok = !namespace.is_empty()
            && namespace.len() <= 32
            && namespace.contains(':')
            && namespace
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_:-".contains(&b));
        let value_ok = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|b| b.is_ascii_alphanumeric());
        (ns_ok && value_ok).then(|| Self {
            namespace: namespace.to_owned(),
            value: value.to_owned(),
        })
    }

    /// A TMDB movie ID; zero (a common provider placeholder) is rejected.
    pub fn tmdb_movie(id: i64) -> Option<Self> {
        if id <= 0 {
            return None;
        }
        Self::new("tmdb:movie", &id.to_string())
    }

    /// Parse the `namespace:value` form produced by [`Self::encoded`].
    pub fn decode(encoded: &str) -> Option<Self> {
        let (namespace, value) = encoded.rsplit_once(':')?;
        Self::new(namespace, value)
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// `namespace:value`, the durable alias encoding.
    pub fn encoded(&self) -> String {
        format!("{}:{}", self.namespace, self.value)
    }
}

impl fmt::Debug for TrustedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TrustedId({}:[redacted])", self.namespace)
    }
}

/// Everything the resolver may know about one canonical entity. Built by the
/// caller from the entity's current bindings plus its durable aliases, and
/// never from model output.
#[derive(Clone)]
pub struct EntityEvidence {
    pub entity_id: EntityId,
    pub kind: ContentKind,
    /// Creation time; the oldest entity of a cluster survives.
    pub created_at: i64,
    /// Display titles (current bindings and aliases). Normalized internally
    /// with [`crate::clean::dedup_key`], which keeps distinguishing numbers.
    pub titles: BTreeSet<String>,
    pub years: BTreeSet<i32>,
    pub trusted_ids: BTreeSet<TrustedId>,
    /// Stable key of the parent series, for episode evidence.
    pub series_key: Option<String>,
    pub episode: Option<EpisodeNumbering>,
}

impl EntityEvidence {
    pub fn new(entity_id: EntityId, kind: ContentKind, created_at: i64) -> Self {
        Self {
            entity_id,
            kind,
            created_at,
            titles: BTreeSet::new(),
            years: BTreeSet::new(),
            trusted_ids: BTreeSet::new(),
            series_key: None,
            episode: None,
        }
    }
}

impl fmt::Debug for EntityEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Titles are provider text; show only their count.
        f.debug_struct("EntityEvidence")
            .field("entity_id", &self.entity_id)
            .field("kind", &self.kind)
            .field("title_count", &self.titles.len())
            .field("years", &self.years)
            .field("trusted_ids", &self.trusted_ids)
            .field("episode", &self.episode)
            .finish_non_exhaustive()
    }
}

/// A durable decision that two entities are distinct. It is order-independent
/// and always wins over any matching evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CannotLink(EntityId, EntityId);

impl CannotLink {
    /// `None` for a self-pair, which is meaningless.
    pub fn new(a: EntityId, b: EntityId) -> Option<Self> {
        match a.cmp(&b) {
            std::cmp::Ordering::Less => Some(Self(a, b)),
            std::cmp::Ordering::Greater => Some(Self(b, a)),
            std::cmp::Ordering::Equal => None,
        }
    }

    pub fn pair(&self) -> (&EntityId, &EntityId) {
        (&self.0, &self.1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contradiction {
    /// Movie vs series, or any other media-kind difference.
    KindMismatch,
    /// Both sides carry an ID in one namespace and share no value.
    TrustedIdConflict,
    /// Episodes of different series.
    SeriesMismatch,
    /// Both sides give a season (or episode) number and they differ.
    EpisodeCoordinateMismatch,
    /// Every year pair is further apart than [`YEAR_TOLERANCE`].
    IncompatibleYear,
    /// A recorded cannot-link decision.
    CannotLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeRule {
    /// A shared trusted external ID, corroborated by a title or a year.
    TrustedId,
    /// Same series and identical, complete season/episode coordinates.
    EpisodeCoordinates,
    /// Identical normalized title and an identical year.
    TitleYear,
}

impl MergeRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrustedId => "trusted_id",
            Self::EpisodeCoordinates => "episode_coordinates",
            Self::TitleYear => "title_year",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstainReason {
    /// Shared trusted ID, but no title or year corroborates it.
    UncorroboratedTrustedId,
    /// Same normalized title, but a year is missing on at least one side.
    MissingYear,
    /// Same title with years within tolerance but not identical.
    NearYear,
    /// Episode evidence without complete coordinates on both sides.
    IncompleteEpisode,
    /// The entity's merge partners contradict each other.
    AmbiguousPartners,
    /// The entity's own evidence is contradictory (e.g. two IDs in one
    /// namespace); it takes part in no merge until corrected.
    InconsistentEvidence,
    /// Nothing deterministic links them beyond sharing a block.
    InsufficientEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairDecision {
    Merge(MergeRule),
    Abstain(AbstainReason),
    Reject(Contradiction),
}

/// Normalized, bounded per-entity view used for comparisons.
struct Prepared<'a> {
    evidence: &'a EntityEvidence,
    title_keys: BTreeSet<String>,
    ids_by_namespace: BTreeMap<&'a str, BTreeSet<&'a str>>,
    inconsistent: bool,
}

impl<'a> Prepared<'a> {
    fn new(evidence: &'a EntityEvidence) -> Self {
        let title_keys: BTreeSet<String> = evidence
            .titles
            .iter()
            .map(|t| crate::clean::dedup_key(t))
            .filter(|k| !k.is_empty())
            .take(MAX_TITLES_PER_ENTITY)
            .collect();
        let mut ids_by_namespace: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for id in &evidence.trusted_ids {
            ids_by_namespace
                .entry(id.namespace())
                .or_default()
                .insert(id.value());
        }
        let year_spread = match (evidence.years.first(), evidence.years.last()) {
            (Some(lo), Some(hi)) => hi - lo,
            _ => 0,
        };
        let inconsistent =
            ids_by_namespace.values().any(|v| v.len() > 1) || year_spread > YEAR_TOLERANCE;
        Self {
            evidence,
            title_keys,
            ids_by_namespace,
            inconsistent,
        }
    }
}

fn years_compatible(a: &BTreeSet<i32>, b: &BTreeSet<i32>) -> bool {
    a.iter()
        .any(|x| b.iter().any(|y| (x - y).abs() <= YEAR_TOLERANCE))
}

fn hard_contradiction(
    a: &Prepared<'_>,
    b: &Prepared<'_>,
    cannot_link: &BTreeSet<CannotLink>,
) -> Option<Contradiction> {
    let (ea, eb) = (a.evidence, b.evidence);
    if ea.kind != eb.kind {
        return Some(Contradiction::KindMismatch);
    }
    if CannotLink::new(ea.entity_id.clone(), eb.entity_id.clone())
        .is_some_and(|pair| cannot_link.contains(&pair))
    {
        return Some(Contradiction::CannotLink);
    }
    for (namespace, values) in &a.ids_by_namespace {
        if let Some(other) = b.ids_by_namespace.get(namespace) {
            if values.is_disjoint(other) {
                return Some(Contradiction::TrustedIdConflict);
            }
        }
    }
    if let (Some(sa), Some(sb)) = (&ea.series_key, &eb.series_key) {
        if sa != sb {
            return Some(Contradiction::SeriesMismatch);
        }
    }
    if let (Some(na), Some(nb)) = (ea.episode, eb.episode) {
        let differs =
            |x: Option<u32>, y: Option<u32>| matches!((x, y), (Some(x), Some(y)) if x != y);
        if differs(na.season, nb.season) || differs(na.episode, nb.episode) {
            return Some(Contradiction::EpisodeCoordinateMismatch);
        }
    }
    if !ea.years.is_empty() && !eb.years.is_empty() && !years_compatible(&ea.years, &eb.years) {
        return Some(Contradiction::IncompatibleYear);
    }
    None
}

fn decide(a: &Prepared<'_>, b: &Prepared<'_>, cannot_link: &BTreeSet<CannotLink>) -> PairDecision {
    if let Some(c) = hard_contradiction(a, b, cannot_link) {
        return PairDecision::Reject(c);
    }
    if a.inconsistent || b.inconsistent {
        return PairDecision::Abstain(AbstainReason::InconsistentEvidence);
    }
    let (ea, eb) = (a.evidence, b.evidence);
    let shares_title = !a.title_keys.is_disjoint(&b.title_keys);
    let both_years = !ea.years.is_empty() && !eb.years.is_empty();

    // Contradictions are already excluded, so any shared namespace here
    // shares a value.
    let shares_id = a
        .ids_by_namespace
        .keys()
        .any(|ns| b.ids_by_namespace.contains_key(ns));
    if shares_id {
        // Even a strong ID needs one independent corroborating signal: a
        // provider copy-pasting a wrong ID onto an unrelated title must not
        // merge it.
        return if shares_title || both_years {
            PairDecision::Merge(MergeRule::TrustedId)
        } else {
            PairDecision::Abstain(AbstainReason::UncorroboratedTrustedId)
        };
    }

    if ea.kind == ContentKind::Episode || ea.series_key.is_some() || eb.series_key.is_some() {
        let complete = |n: Option<EpisodeNumbering>| {
            matches!(
                n,
                Some(EpisodeNumbering {
                    season: Some(_),
                    episode: Some(_)
                })
            )
        };
        return if ea.series_key.is_some()
            && ea.series_key == eb.series_key
            && complete(ea.episode)
            && ea.episode == eb.episode
        {
            PairDecision::Merge(MergeRule::EpisodeCoordinates)
        } else {
            PairDecision::Abstain(AbstainReason::IncompleteEpisode)
        };
    }

    if shares_title {
        if !both_years {
            return PairDecision::Abstain(AbstainReason::MissingYear);
        }
        return if ea.years.is_disjoint(&eb.years) {
            PairDecision::Abstain(AbstainReason::NearYear)
        } else {
            PairDecision::Merge(MergeRule::TitleYear)
        };
    }
    PairDecision::Abstain(AbstainReason::InsufficientEvidence)
}

/// Hard contradiction between two entities, if any.
pub fn contradiction(
    a: &EntityEvidence,
    b: &EntityEvidence,
    cannot_link: &BTreeSet<CannotLink>,
) -> Option<Contradiction> {
    hard_contradiction(&Prepared::new(a), &Prepared::new(b), cannot_link)
}

/// Deterministic decision for one candidate pair.
pub fn assess_pair(
    a: &EntityEvidence,
    b: &EntityEvidence,
    cannot_link: &BTreeSet<CannotLink>,
) -> PairDecision {
    decide(&Prepared::new(a), &Prepared::new(b), cannot_link)
}

/// Instruction to merge `absorbed` into `survivor`. Only the deterministic
/// resolver constructs one; storage accepts nothing else for an identity merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeDecision {
    survivor: EntityId,
    absorbed: EntityId,
    /// The cluster member `absorbed` was directly matched with.
    partner: EntityId,
    rule: MergeRule,
}

impl MergeDecision {
    pub fn survivor(&self) -> &EntityId {
        &self.survivor
    }
    pub fn absorbed(&self) -> &EntityId {
        &self.absorbed
    }
    pub fn partner(&self) -> &EntityId {
        &self.partner
    }
    pub fn rule(&self) -> MergeRule {
        self.rule
    }
    pub fn ruleset_version(&self) -> u32 {
        RULESET_VERSION
    }
}

/// An entity the resolver could not decide, with the bounded set of real
/// candidates it abstained against. The only input a model may reason over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguityCase {
    subject: EntityId,
    candidates: Vec<EntityId>,
    reason: AbstainReason,
}

impl AmbiguityCase {
    pub fn subject(&self) -> &EntityId {
        &self.subject
    }
    pub fn candidates(&self) -> &[EntityId] {
        &self.candidates
    }
    pub fn reason(&self) -> AbstainReason {
        self.reason
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionStats {
    pub entities: usize,
    pub candidate_pairs: usize,
    pub merges: usize,
    pub rejected: usize,
    pub abstained: usize,
    pub oversized_blocks: usize,
    pub inconsistent_entities: usize,
    /// Merge edges refused because they would join mutually contradictory
    /// members or exceed [`MAX_CLUSTER_SIZE`].
    pub cluster_conflicts: usize,
}

#[derive(Debug, Default)]
pub struct ResolutionPlan {
    /// Sorted by `(survivor, absorbed)`; each absorbed entity appears once and
    /// never as a survivor.
    pub merges: Vec<MergeDecision>,
    /// Sorted by subject; candidates sorted and bounded.
    pub ambiguities: Vec<AmbiguityCase>,
    pub stats: ResolutionStats,
}

struct Edge {
    rule: MergeRule,
    a: usize,
    b: usize,
}

/// Resolve a set of entities. Deterministic: the same evidence yields the
/// same plan regardless of input order. Work is bounded by the blocks, never
/// a whole-library pairwise scan.
pub fn resolve(entities: &[EntityEvidence], cannot_link: &BTreeSet<CannotLink>) -> ResolutionPlan {
    let mut order: Vec<&EntityEvidence> = entities.iter().collect();
    order.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    order.dedup_by(|a, b| a.entity_id == b.entity_id);
    let prepared: Vec<Prepared<'_>> = order.iter().map(|e| Prepared::new(e)).collect();
    let mut stats = ResolutionStats {
        entities: prepared.len(),
        inconsistent_entities: prepared.iter().filter(|p| p.inconsistent).count(),
        ..ResolutionStats::default()
    };

    // 1. Blocking. Titles are keyed without a year so a year-less copy still
    //    meets its dated twin (as an abstention, which feeds ambiguity cases).
    let mut blocks: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, p) in prepared.iter().enumerate() {
        let e = p.evidence;
        let trusted_ids: Vec<(String, String)> = e
            .trusted_ids
            .iter()
            .map(|id| (id.namespace().to_owned(), id.value().to_owned()))
            .collect();
        let mut keys = BTreeSet::new();
        let titles: Vec<&String> = e.titles.iter().take(MAX_TITLES_PER_ENTITY).collect();
        let mut first = true;
        for title in titles
            .iter()
            .copied()
            .map(Some)
            .chain(std::iter::once(None))
        {
            if title.is_none() && !first {
                break;
            }
            let evidence = BlockingEvidence {
                kind: e.kind,
                raw_title: title.cloned().unwrap_or_default(),
                year: None,
                trusted_ids: if first {
                    trusted_ids.clone()
                } else {
                    Vec::new()
                },
                episode: e.episode,
                language: None,
                series_key: e.series_key.clone(),
            };
            first = false;
            keys.extend(
                blocking_keys(&evidence)
                    .into_iter()
                    .map(|k| k.as_str().to_owned()),
            );
        }
        for key in keys {
            blocks.entry(key).or_default().push(index);
        }
    }

    // 2. Pairwise decisions within bounded blocks only.
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut abstentions: BTreeMap<usize, BTreeMap<usize, AbstainReason>> = BTreeMap::new();
    for members in blocks.values() {
        if members.len() > MAX_BLOCK_SIZE {
            stats.oversized_blocks += 1;
            continue;
        }
        for (i, &a) in members.iter().enumerate() {
            for &b in &members[i + 1..] {
                let (a, b) = (a.min(b), a.max(b));
                if a == b || !seen.insert((a, b)) {
                    continue;
                }
                stats.candidate_pairs += 1;
                match decide(&prepared[a], &prepared[b], cannot_link) {
                    PairDecision::Merge(rule) => edges.push(Edge { rule, a, b }),
                    PairDecision::Abstain(reason) => {
                        stats.abstained += 1;
                        abstentions.entry(a).or_default().insert(b, reason);
                    }
                    PairDecision::Reject(_) => stats.rejected += 1,
                }
            }
        }
    }

    // 3. Ambiguous hubs: an entity whose merge partners contradict each other
    //    cannot pick one deterministically, so none of its edges merge.
    let mut partners: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for edge in &edges {
        partners.entry(edge.a).or_default().insert(edge.b);
        partners.entry(edge.b).or_default().insert(edge.a);
    }
    let mut ambiguous_hubs: BTreeSet<usize> = BTreeSet::new();
    for (&hub, set) in &partners {
        let list: Vec<usize> = set.iter().copied().collect();
        let conflicted = list.iter().enumerate().any(|(i, &x)| {
            list[i + 1..]
                .iter()
                .any(|&y| hard_contradiction(&prepared[x], &prepared[y], cannot_link).is_some())
        });
        if conflicted {
            ambiguous_hubs.insert(hub);
        }
    }
    edges.retain(|edge| {
        let keep = !ambiguous_hubs.contains(&edge.a) && !ambiguous_hubs.contains(&edge.b);
        if !keep {
            stats.abstained += 1;
            abstentions
                .entry(edge.a)
                .or_default()
                .insert(edge.b, AbstainReason::AmbiguousPartners);
        }
        keep
    });

    // 4. Conflict-safe clustering, strongest rule first, then by entity order.
    edges.sort_by_key(|e| (e.rule, e.a, e.b));
    let mut parent: Vec<usize> = (0..prepared.len()).collect();
    let mut members: Vec<Vec<usize>> = (0..prepared.len()).map(|i| vec![i]).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut accepted: Vec<&Edge> = Vec::new();
    for edge in &edges {
        let (ra, rb) = (find(&mut parent, edge.a), find(&mut parent, edge.b));
        if ra == rb {
            accepted.push(edge);
            continue;
        }
        let too_big = members[ra].len() + members[rb].len() > MAX_CLUSTER_SIZE;
        let conflict = too_big
            || members[ra].iter().any(|&x| {
                members[rb]
                    .iter()
                    .any(|&y| hard_contradiction(&prepared[x], &prepared[y], cannot_link).is_some())
            });
        if conflict {
            stats.cluster_conflicts += 1;
            continue;
        }
        let (root, child) = (ra.min(rb), ra.max(rb));
        parent[child] = root;
        let moved = std::mem::take(&mut members[child]);
        members[root].extend(moved);
        accepted.push(edge);
    }

    // 5. Survivor per cluster; one decision per absorbed member, attributed
    //    to its strongest accepted edge.
    let mut best_edge: BTreeMap<usize, (MergeRule, usize)> = BTreeMap::new();
    for edge in &accepted {
        for (me, other) in [(edge.a, edge.b), (edge.b, edge.a)] {
            let candidate = (edge.rule, other);
            best_edge
                .entry(me)
                .and_modify(|cur| {
                    if candidate < *cur {
                        *cur = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    let mut merges = Vec::new();
    for (root, cluster) in members.iter().enumerate() {
        if find(&mut parent, root) != root || cluster.len() < 2 {
            continue;
        }
        let survivor = *cluster
            .iter()
            .min_by(|&&x, &&y| {
                let (ex, ey) = (prepared[x].evidence, prepared[y].evidence);
                (ex.created_at, &ex.entity_id).cmp(&(ey.created_at, &ey.entity_id))
            })
            .expect("cluster is non-empty");
        for &member in cluster {
            if member == survivor {
                continue;
            }
            let (rule, partner) = best_edge[&member];
            merges.push(MergeDecision {
                survivor: prepared[survivor].evidence.entity_id.clone(),
                absorbed: prepared[member].evidence.entity_id.clone(),
                partner: prepared[partner].evidence.entity_id.clone(),
                rule,
            });
        }
    }
    merges.sort_by(|x, y| (&x.survivor, &x.absorbed).cmp(&(&y.survivor, &y.absorbed)));
    stats.merges = merges.len();

    // 6. Ambiguity cases for entities that did not merge at all.
    let merged: BTreeSet<&EntityId> = merges
        .iter()
        .flat_map(|m| [&m.survivor, &m.absorbed])
        .collect();
    let mut by_subject: BTreeMap<usize, (AbstainReason, BTreeSet<usize>)> = BTreeMap::new();
    for (&a, others) in &abstentions {
        for (&b, &reason) in others {
            for (subject, candidate) in [(a, b), (b, a)] {
                let entry = by_subject
                    .entry(subject)
                    .or_insert((reason, BTreeSet::new()));
                entry.0 = entry.0.min(reason);
                entry.1.insert(candidate);
            }
        }
    }
    let ambiguities = by_subject
        .into_iter()
        .filter(|(subject, _)| !merged.contains(&prepared[*subject].evidence.entity_id))
        .map(|(subject, (reason, candidates))| AmbiguityCase {
            subject: prepared[subject].evidence.entity_id.clone(),
            candidates: candidates
                .into_iter()
                .take(MAX_AI_CANDIDATES)
                .map(|c| prepared[c].evidence.entity_id.clone())
                .collect(),
            reason,
        })
        .collect();

    ResolutionPlan {
        merges,
        ambiguities,
        stats,
    }
}

// ── AI boundary ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiVerdict {
    /// The model declines to decide. Always a valid answer.
    Abstain,
    SameAs(EntityId),
    DistinctFrom(EntityId),
}

/// Advisory output of a model for one [`AmbiguityCase`]. It carries no
/// authority: it cannot become a [`MergeDecision`], and applying anything
/// based on it is a separate, explicitly confirmed step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSuggestion {
    subject: EntityId,
    verdict: AiVerdict,
}

impl AiSuggestion {
    pub fn subject(&self) -> &EntityId {
        &self.subject
    }
    pub fn verdict(&self) -> &AiVerdict {
        &self.verdict
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum VerdictKind {
    Abstain,
    SameAs,
    DistinctFrom,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerdictProposal {
    version: u16,
    subject: EntityId,
    verdict: VerdictKind,
    #[serde(default)]
    candidate: Option<EntityId>,
}

/// Validate a model's verdict for one case. The model only ever sees the
/// case's real candidates. A verdict naming anything else, another subject, or
/// the wrong protocol is rejected whole.
pub fn assess_ai_verdict(
    case: &AmbiguityCase,
    bytes: &[u8],
) -> Result<AiSuggestion, IntelligenceError> {
    if bytes.len() > MAX_AI_VERDICT_BYTES {
        return Err(IntelligenceError::Limit);
    }
    let proposal: VerdictProposal =
        serde_json::from_slice(bytes).map_err(|_| IntelligenceError::Schema)?;
    if proposal.version != AI_PROTOCOL_VERSION || proposal.subject != case.subject {
        return Err(IntelligenceError::Context);
    }
    let in_case = |id: &EntityId| case.candidates.contains(id);
    let verdict = match (proposal.verdict, proposal.candidate) {
        (VerdictKind::Abstain, None) => AiVerdict::Abstain,
        (VerdictKind::SameAs, Some(c)) if in_case(&c) => AiVerdict::SameAs(c),
        (VerdictKind::DistinctFrom, Some(c)) if in_case(&c) => AiVerdict::DistinctFrom(c),
        (VerdictKind::Abstain, Some(_)) => return Err(IntelligenceError::Schema),
        (_, None) => return Err(IntelligenceError::Schema),
        (_, Some(_)) => return Err(IntelligenceError::Evidence),
    };
    Ok(AiSuggestion {
        subject: proposal.subject,
        verdict,
    })
}
