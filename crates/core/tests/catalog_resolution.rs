use std::collections::BTreeSet;

use nova_core::catalog::intelligence::IntelligenceError;
use nova_core::catalog::resolution::*;
use nova_core::catalog::{ContentKind, EntityId, EpisodeNumbering};

fn id(value: &str) -> EntityId {
    EntityId::new(value).unwrap()
}

fn movie(entity: &str, created_at: i64, title: &str, year: Option<i32>) -> EntityEvidence {
    let mut e = EntityEvidence::new(id(entity), ContentKind::Movie, created_at);
    e.titles.insert(title.into());
    e.years.extend(year);
    e
}

fn with_tmdb(mut e: EntityEvidence, tmdb: i64) -> EntityEvidence {
    e.trusted_ids.insert(TrustedId::tmdb_movie(tmdb).unwrap());
    e
}

fn episode(entity: &str, series: &str, season: u32, number: u32) -> EntityEvidence {
    let mut e = EntityEvidence::new(id(entity), ContentKind::Episode, 0);
    e.titles.insert(format!("Episode {number}"));
    e.series_key = Some(series.into());
    e.episode = Some(EpisodeNumbering {
        season: Some(season),
        episode: Some(number),
    });
    e
}

fn none() -> BTreeSet<CannotLink> {
    BTreeSet::new()
}

fn pairs(plan: &ResolutionPlan) -> Vec<(String, String, MergeRule)> {
    plan.merges
        .iter()
        .map(|m| {
            (
                m.survivor().as_str().to_owned(),
                m.absorbed().as_str().to_owned(),
                m.rule(),
            )
        })
        .collect()
}

#[test]
fn identical_title_and_year_merge_into_the_oldest_entity() {
    let plan = resolve(
        &[
            movie("b", 20, "Heat FHD", Some(1995)),
            movie("a", 10, "Heat", Some(1995)),
            movie("c", 5, "HEAT 1080p", Some(1995)),
        ],
        &none(),
    );
    assert_eq!(
        pairs(&plan),
        vec![
            ("c".into(), "a".into(), MergeRule::TitleYear),
            ("c".into(), "b".into(), MergeRule::TitleYear),
        ]
    );
    assert!(plan
        .merges
        .iter()
        .all(|m| m.ruleset_version() == RULESET_VERSION));
}

#[test]
fn movie_and_series_never_merge() {
    let mut series = EntityEvidence::new(id("s"), ContentKind::Series, 0);
    series.titles.insert("Fargo".into());
    series.years.insert(1996);
    let film = movie("m", 0, "Fargo", Some(1996));
    assert_eq!(
        assess_pair(&film, &series, &none()),
        PairDecision::Reject(Contradiction::KindMismatch)
    );
    assert!(resolve(&[film, series], &none()).merges.is_empty());
}

#[test]
fn conflicting_trusted_ids_block_an_otherwise_exact_match() {
    let a = with_tmdb(movie("a", 0, "Heat", Some(1995)), 949);
    let b = with_tmdb(movie("b", 0, "Heat", Some(1995)), 1_234);
    assert_eq!(
        assess_pair(&a, &b, &none()),
        PairDecision::Reject(Contradiction::TrustedIdConflict)
    );
    assert!(resolve(&[a, b], &none()).merges.is_empty());
}

#[test]
fn ids_in_different_namespaces_do_not_conflict_or_match() {
    let a = with_tmdb(movie("a", 0, "Heat", Some(1995)), 949);
    let mut b = movie("b", 0, "Heat", Some(1995));
    b.trusted_ids
        .insert(TrustedId::new("imdb:movie", "tt0113277").unwrap());
    // Falls through to the title/year rule.
    assert_eq!(
        assess_pair(&a, &b, &none()),
        PairDecision::Merge(MergeRule::TitleYear)
    );
}

#[test]
fn clearly_incompatible_years_reject_and_near_years_abstain() {
    // Remakes share a title.
    let original = movie("a", 0, "Scarface", Some(1932));
    let remake = movie("b", 0, "Scarface", Some(1983));
    assert_eq!(
        assess_pair(&original, &remake, &none()),
        PairDecision::Reject(Contradiction::IncompatibleYear)
    );
    // Even a shared trusted ID cannot overrule a clear year contradiction.
    assert_eq!(
        assess_pair(&with_tmdb(original, 7), &with_tmdb(remake, 7), &none()),
        PairDecision::Reject(Contradiction::IncompatibleYear)
    );
    let near = assess_pair(
        &movie("c", 0, "Heat", Some(1995)),
        &movie("d", 0, "Heat", Some(1996)),
        &none(),
    );
    assert_eq!(near, PairDecision::Abstain(AbstainReason::NearYear));
}

#[test]
fn a_missing_year_abstains_and_yields_a_bounded_ambiguity_case() {
    let plan = resolve(
        &[
            movie("a", 0, "Heat", Some(1995)),
            movie("b", 0, "Heat", None),
        ],
        &none(),
    );
    assert!(plan.merges.is_empty());
    let case = plan
        .ambiguities
        .iter()
        .find(|c| c.subject().as_str() == "b")
        .unwrap();
    assert_eq!(case.candidates(), &[id("a")]);
    assert_eq!(case.reason(), AbstainReason::MissingYear);
}

#[test]
fn distinguishing_numbers_never_merge() {
    let plan = resolve(
        &[
            movie("a", 0, "Rocky", Some(1976)),
            movie("b", 0, "Rocky 2", Some(1976)),
        ],
        &none(),
    );
    assert!(plan.merges.is_empty());
    assert_eq!(plan.stats.candidate_pairs, 0, "never even compared");
}

#[test]
fn a_shared_trusted_id_links_localized_titles_only_when_corroborated() {
    let fr = with_tmdb(movie("fr", 0, "Le Parrain", Some(1972)), 238);
    let en = with_tmdb(movie("en", 1, "The Godfather", Some(1972)), 238);
    assert_eq!(
        pairs(&resolve(&[fr, en], &none())),
        vec![("fr".into(), "en".into(), MergeRule::TrustedId)]
    );
    // No title in common and no years: a copy-pasted ID alone is not enough.
    let x = with_tmdb(movie("x", 0, "Le Parrain", None), 238);
    let y = with_tmdb(movie("y", 0, "Unrelated Title", None), 238);
    assert_eq!(
        assess_pair(&x, &y, &none()),
        PairDecision::Abstain(AbstainReason::UncorroboratedTrustedId)
    );
}

#[test]
fn episode_coordinates_decide_episode_identity() {
    let a = episode("a", "series-1", 1, 2);
    let b = episode("b", "series-1", 1, 2);
    let other_ep = episode("c", "series-1", 1, 3);
    let other_series = episode("d", "series-2", 1, 2);
    assert_eq!(
        assess_pair(&a, &b, &none()),
        PairDecision::Merge(MergeRule::EpisodeCoordinates)
    );
    assert_eq!(
        assess_pair(&a, &other_ep, &none()),
        PairDecision::Reject(Contradiction::EpisodeCoordinateMismatch)
    );
    assert_eq!(
        assess_pair(&a, &other_series, &none()),
        PairDecision::Reject(Contradiction::SeriesMismatch)
    );
    let mut partial = episode("e", "series-1", 1, 2);
    partial.episode = Some(EpisodeNumbering {
        season: Some(1),
        episode: None,
    });
    assert_eq!(
        assess_pair(&a, &partial, &none()),
        PairDecision::Abstain(AbstainReason::IncompleteEpisode)
    );
    assert_eq!(
        pairs(&resolve(&[a, b, other_ep, other_series], &none())),
        vec![("a".into(), "b".into(), MergeRule::EpisodeCoordinates)]
    );
}

#[test]
fn matches_never_chain_across_a_contradiction() {
    // b matches both a and c by title/year, but a and c carry conflicting
    // IDs: b is ambiguous and nothing merges.
    let a = with_tmdb(movie("a", 0, "Heat", Some(1995)), 1);
    let b = movie("b", 1, "Heat", Some(1995));
    let c = with_tmdb(movie("c", 2, "Heat", Some(1995)), 2);
    let plan = resolve(&[a, b, c], &none());
    assert!(plan.merges.is_empty(), "{:?}", pairs(&plan));
    let case = plan
        .ambiguities
        .iter()
        .find(|c| c.subject().as_str() == "b")
        .unwrap();
    assert_eq!(case.candidates(), &[id("a"), id("c")]);
}

#[test]
fn year_drift_cannot_chain_into_an_incompatible_cluster() {
    // a~b and b~c by shared ID within tolerance, but a and c are two years
    // apart: b is ambiguous rather than bridging them.
    let a = with_tmdb(movie("a", 0, "Film", Some(2000)), 5);
    let b = with_tmdb(movie("b", 1, "Film", Some(2001)), 5);
    let c = with_tmdb(movie("c", 2, "Film", Some(2002)), 5);
    let plan = resolve(&[a, b, c], &none());
    for m in &plan.merges {
        let pair = [m.survivor().as_str(), m.absorbed().as_str()];
        assert!(!(pair.contains(&"a") && pair.contains(&"c")));
    }
    assert!(plan.merges.is_empty());
}

#[test]
fn cannot_link_overrides_every_matching_signal() {
    let a = with_tmdb(movie("a", 0, "Heat", Some(1995)), 949);
    let b = with_tmdb(movie("b", 0, "Heat", Some(1995)), 949);
    let links: BTreeSet<_> = [CannotLink::new(id("b"), id("a")).unwrap()].into();
    assert_eq!(
        assess_pair(&a, &b, &links),
        PairDecision::Reject(Contradiction::CannotLink)
    );
    assert!(resolve(&[a, b], &links).merges.is_empty());
    assert!(CannotLink::new(id("a"), id("a")).is_none());
}

#[test]
fn internally_inconsistent_evidence_takes_part_in_no_merge() {
    let mut a = movie("a", 0, "Heat", Some(1995));
    a.years.insert(1999); // spread beyond tolerance
    let b = movie("b", 0, "Heat", Some(1995));
    assert_eq!(
        assess_pair(&a, &b, &none()),
        PairDecision::Abstain(AbstainReason::InconsistentEvidence)
    );
    let plan = resolve(&[a, b], &none());
    assert!(plan.merges.is_empty());
    assert_eq!(plan.stats.inconsistent_entities, 1);
}

#[test]
fn resolution_is_deterministic_regardless_of_input_order() {
    let mut items = Vec::new();
    for i in 0..60 {
        let title = format!("Title {}", i % 7);
        items.push(movie(
            &format!("e{i:03}"),
            (i % 5) as i64,
            &title,
            Some(2000 + (i % 3)),
        ));
    }
    let forward = resolve(&items, &none());
    items.reverse();
    let backward = resolve(&items, &none());
    assert_eq!(forward.merges, backward.merges);
    assert_eq!(forward.ambiguities, backward.ambiguities);
    // Absorbed entities are never survivors, and each is absorbed once.
    let absorbed: BTreeSet<_> = forward.merges.iter().map(|m| m.absorbed()).collect();
    assert_eq!(absorbed.len(), forward.merges.len());
    assert!(forward
        .merges
        .iter()
        .all(|m| !absorbed.contains(m.survivor())));
}

#[test]
fn oversized_blocks_abstain_instead_of_fanning_out() {
    let items: Vec<_> = (0..(MAX_BLOCK_SIZE + 1))
        .map(|i| movie(&format!("e{i}"), 0, "Home", Some(2000)))
        .collect();
    let plan = resolve(&items, &none());
    assert!(plan.merges.is_empty());
    assert_eq!(plan.stats.oversized_blocks, 1);
    assert_eq!(plan.stats.candidate_pairs, 0);
}

#[test]
fn resolution_scales_to_a_large_catalog_without_pairwise_blowup() {
    const TOTAL: usize = 20_000;
    const DISTINCT: usize = 5_000;
    let items: Vec<_> = (0..TOTAL)
        .map(|i| {
            movie(
                &format!("e{i:05}"),
                i as i64,
                &format!("Title {}", i % DISTINCT),
                Some(1990 + (i % DISTINCT % 30) as i32),
            )
        })
        .collect();
    let start = std::time::Instant::now();
    let plan = resolve(&items, &none());
    let elapsed = start.elapsed();
    assert_eq!(plan.merges.len(), TOTAL - DISTINCT);
    assert!(
        elapsed.as_secs() < 3,
        "resolution took too long: {elapsed:?}"
    );
}

#[test]
fn trusted_ids_are_validated_and_redacted() {
    assert!(TrustedId::tmdb_movie(0).is_none());
    assert!(TrustedId::tmdb_movie(-4).is_none());
    assert!(TrustedId::new("tmdb", "1").is_none(), "kind is required");
    assert!(TrustedId::new("tmdb:movie", "1 OR 1=1").is_none());
    let t = TrustedId::tmdb_movie(603).unwrap();
    assert_eq!(t.encoded(), "tmdb:movie:603");
    assert_eq!(TrustedId::decode(&t.encoded()), Some(t.clone()));
    assert!(!format!("{t:?}").contains("603"));
    let e = with_tmdb(movie("a", 0, "secret-title", None), 603);
    let debug = format!("{e:?}");
    assert!(!debug.contains("secret-title") && !debug.contains("603"));
}

// ── AI boundary ─────────────────────────────────────────────────────────────

fn ambiguity() -> AmbiguityCase {
    resolve(
        &[
            movie("a", 0, "Heat", Some(1995)),
            movie("b", 0, "Heat", None),
        ],
        &none(),
    )
    .ambiguities
    .into_iter()
    .find(|c| c.subject().as_str() == "b")
    .unwrap()
}

#[test]
fn ai_may_pick_a_real_candidate_or_abstain() {
    let case = ambiguity();
    let same = assess_ai_verdict(
        &case,
        br#"{"version":1,"subject":"b","verdict":"same_as","candidate":"a"}"#,
    )
    .unwrap();
    assert_eq!(same.verdict(), &AiVerdict::SameAs(id("a")));
    let abstain =
        assess_ai_verdict(&case, br#"{"version":1,"subject":"b","verdict":"abstain"}"#).unwrap();
    assert_eq!(abstain.verdict(), &AiVerdict::Abstain);
    let distinct = assess_ai_verdict(
        &case,
        br#"{"version":1,"subject":"b","verdict":"distinct_from","candidate":"a"}"#,
    )
    .unwrap();
    assert_eq!(distinct.verdict(), &AiVerdict::DistinctFrom(id("a")));
}

#[test]
fn ai_cannot_invent_candidates_or_change_the_subject() {
    let case = ambiguity();
    let cases: [(&[u8], IntelligenceError); 7] = [
        (
            br#"{"version":1,"subject":"b","verdict":"same_as","candidate":"zzz"}"#,
            IntelligenceError::Evidence,
        ),
        (
            br#"{"version":1,"subject":"a","verdict":"same_as","candidate":"b"}"#,
            IntelligenceError::Context,
        ),
        (
            br#"{"version":2,"subject":"b","verdict":"abstain"}"#,
            IntelligenceError::Context,
        ),
        (
            br#"{"version":1,"subject":"b","verdict":"same_as"}"#,
            IntelligenceError::Schema,
        ),
        (
            br#"{"version":1,"subject":"b","verdict":"abstain","candidate":"a"}"#,
            IntelligenceError::Schema,
        ),
        (
            br#"{"version":1,"subject":"b","verdict":"merge_now","candidate":"a"}"#,
            IntelligenceError::Schema,
        ),
        (
            br#"{"version":1,"subject":"b","verdict":"abstain","survivor":"a"}"#,
            IntelligenceError::Schema,
        ),
    ];
    for (bytes, expected) in cases {
        assert_eq!(
            assess_ai_verdict(&case, bytes).unwrap_err(),
            expected,
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
    let huge = vec![b' '; MAX_AI_VERDICT_BYTES + 1];
    assert_eq!(
        assess_ai_verdict(&case, &huge).unwrap_err(),
        IntelligenceError::Limit
    );
}

#[test]
fn ai_cases_are_bounded() {
    let mut items = vec![movie("subject", 0, "Heat", None)];
    for i in 0..(MAX_AI_CANDIDATES * 2) {
        items.push(movie(
            &format!("c{i:02}"),
            0,
            "Heat",
            Some(1980 + 3 * i as i32),
        ));
    }
    let plan = resolve(&items, &none());
    let case = plan
        .ambiguities
        .iter()
        .find(|c| c.subject().as_str() == "subject")
        .unwrap();
    assert_eq!(case.candidates().len(), MAX_AI_CANDIDATES);
}
