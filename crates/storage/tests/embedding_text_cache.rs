//! Vectors reused across a playlist refresh: keyed by model and exact text,
//! refreshed on use, pruned when stale.
use nova_storage::{init_profile_db, EmbeddingRepository};

#[tokio::test]
async fn vectors_round_trip_per_model_and_exact_text() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = EmbeddingRepository::new(&pool);
    let a = [0.25f32, -1.0, 3.5];
    repo.remember_text_vectors("m1", &[("Inception (2010)", &a)], &[], 100, 0)
        .await
        .unwrap();
    let texts = vec![
        "Inception (2010)".to_string(),
        "inception (2010)".to_string(),
    ];
    let got = repo.cached_text_vectors("m1", &texts).await.unwrap();
    assert_eq!(got, vec![Some(a.to_vec()), None]);
    let other_model = repo.cached_text_vectors("m2", &texts).await.unwrap();
    assert_eq!(other_model, vec![None, None]);
}

#[tokio::test]
async fn use_refreshes_entries_and_stale_ones_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = EmbeddingRepository::new(&pool);
    let v = [1.0f32];
    repo.remember_text_vectors("m", &[("old", &v), ("kept", &v)], &[], 10, 0)
        .await
        .unwrap();
    // A later run reuses "kept"; "old" was not seen since before 50.
    repo.remember_text_vectors("m", &[], &["kept"], 100, 50)
        .await
        .unwrap();
    let texts = vec!["old".to_string(), "kept".to_string()];
    let got = repo.cached_text_vectors("m", &texts).await.unwrap();
    assert_eq!(got, vec![None, Some(v.to_vec())]);
    // Pruning is per model.
    repo.remember_text_vectors("other", &[("x", &v)], &[], 1, 0)
        .await
        .unwrap();
    repo.remember_text_vectors("m", &[], &[], 1_000, 500)
        .await
        .unwrap();
    let x = repo
        .cached_text_vectors("other", &["x".to_string()])
        .await
        .unwrap();
    assert_eq!(x, vec![Some(v.to_vec())]);
}

#[tokio::test]
async fn oversized_texts_and_empty_vectors_are_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_profile_db(dir.path().join("profile.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = EmbeddingRepository::new(&pool);
    let long = "x".repeat(nova_storage::MAX_CACHED_TEXT + 1);
    repo.remember_text_vectors("m", &[(long.as_str(), &[1.0]), ("empty", &[])], &[], 1, 0)
        .await
        .unwrap();
    let got = repo
        .cached_text_vectors("m", &[long, "empty".to_string()])
        .await
        .unwrap();
    assert_eq!(got, vec![None, None]);
}
