//! Provider facts: stored with their rows, series-level facts survive the
//! catalog row being replaced by real episodes, and fill the series overview
//! and backdrop when TMDB has nothing.

use nova_core::{ContentSort, EpisodeNumberingKind, ProviderFacts, SeriesEpisode, SourceEvidence};
use nova_storage::{init_profile_db, SeriesRepository, SourceRepository};

fn facts(plot: &str) -> ProviderFacts {
    ProviderFacts {
        plot: Some(plot.into()),
        rating: Some(8.0),
        backdrop_url: Some("https://img/back.jpg".into()),
        ..ProviderFacts::default()
    }
}

fn row(
    source: i64,
    season: i64,
    episode: i64,
    url: &str,
    evidence: SourceEvidence,
) -> SeriesEpisode {
    SeriesEpisode {
        id: 0,
        source_id: source,
        series_title: "Dark".into(),
        category: None,
        season,
        episode,
        episode_title: None,
        logo_url: None,
        stream_url: url.into(),
        added_at: None,
        numbering: EpisodeNumberingKind::SeasonEpisode,
        evidence,
    }
}

#[tokio::test]
async fn series_facts_outlive_the_catalog_row_and_fill_the_overview() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("xtream", "p", "https://example.invalid")
        .await
        .unwrap()
        .id;
    let repo = SeriesRepository::new(&pool);

    // Import: the Xtream catalog row carries the series facts and TMDB id.
    let catalog = row(
        source,
        0,
        0,
        "xtream://series/7",
        SourceEvidence {
            provider_facts: Some(facts("Catalog plot")),
            provider_tmdb_id: Some(70523),
            ..SourceEvidence::default()
        },
    );
    repo.insert_batch(&[catalog]).await.unwrap();
    assert_eq!(
        repo.series_provider_facts(source, "Dark").await.unwrap(),
        (Some(facts("Catalog plot")), Some(70523))
    );
    let list = repo
        .list_series(
            source,
            None,
            None,
            false,
            false,
            ContentSort::Default,
            10,
            0,
        )
        .await
        .unwrap();
    assert_eq!(list[0].overview.as_deref(), Some("Catalog plot"));
    assert_eq!(
        list[0].backdrop_url.as_deref(),
        Some("https://img/back.jpg")
    );

    // Opening the series replaces the catalog row with episodes; the
    // get_series_info facts refresh the series facts and fill gaps only.
    let episode_facts = ProviderFacts {
        plot: Some("Episode plot".into()),
        duration_sec: Some(3000),
        ..ProviderFacts::default()
    };
    let e1 = row(
        source,
        1,
        1,
        "http://h/series/u/p/1.mkv",
        SourceEvidence {
            provider_facts: Some(episode_facts.clone()),
            ..SourceEvidence::default()
        },
    );
    let e2 = row(
        source,
        1,
        2,
        "http://h/series/u/p/2.mkv",
        SourceEvidence::default(),
    );
    let info = ProviderFacts {
        plot: Some("Info plot".into()),
        ..ProviderFacts::default()
    };
    repo.replace_series_episodes(source, "Dark", &[e1, e2], Some(&info), None)
        .await
        .unwrap();
    let (series, tmdb) = repo.series_provider_facts(source, "Dark").await.unwrap();
    let series = series.unwrap();
    assert_eq!(series.plot.as_deref(), Some("Info plot"), "fresh facts win");
    assert_eq!(
        series.rating,
        Some(8.0),
        "gaps filled from the catalog facts"
    );
    assert_eq!(tmdb, Some(70523), "a missing id keeps the stored one");

    let by_episode = repo.episode_provider_facts(source, "Dark").await.unwrap();
    assert_eq!(by_episode.len(), 1);
    assert_eq!(by_episode.values().next(), Some(&episode_facts));

    // A confident TMDB overview wins over provider facts.
    sqlx::query(
        "INSERT INTO series_meta (source_id, series_title, overview, tmdb_status)
         VALUES (?, 'Dark', 'TMDB overview', 'matched')",
    )
    .bind(source)
    .execute(&pool)
    .await
    .unwrap();
    let list = repo
        .list_series(
            source,
            None,
            None,
            false,
            false,
            ContentSort::Default,
            10,
            0,
        )
        .await
        .unwrap();
    assert_eq!(list[0].overview.as_deref(), Some("TMDB overview"));
    pool.close().await;
}

#[tokio::test]
async fn vod_facts_merge_richer_details_over_list_facts() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("p.db").to_str().unwrap())
        .await
        .unwrap();
    let source = SourceRepository::new(&pool)
        .create("xtream", "p", "https://example.invalid")
        .await
        .unwrap()
        .id;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO vod_items (source_id, name, stream_url, provider_facts)
         VALUES (?, 'Inception', 'http://h/movie/u/p/5.mkv', '{\"rating\":8.8}') RETURNING id",
    )
    .bind(source)
    .fetch_one(&pool)
    .await
    .unwrap();
    let repo = nova_storage::VodRepository::new(&pool);
    assert_eq!(
        repo.provider_facts(id).await.unwrap().unwrap().rating,
        Some(8.8)
    );
    repo.merge_provider_facts(
        id,
        &ProviderFacts {
            plot: Some("A thief who steals secrets.".into()),
            ..ProviderFacts::default()
        },
    )
    .await
    .unwrap();
    let merged = repo.provider_facts(id).await.unwrap().unwrap();
    assert_eq!(merged.plot.as_deref(), Some("A thief who steals secrets."));
    assert_eq!(merged.rating, Some(8.8), "list facts kept");
    pool.close().await;
}
