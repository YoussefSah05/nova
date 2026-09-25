use nova_core::models::{
    ModelAvailability, ModelDescriptor, ModelId, ModelLibrary, ModelLibraryId, ModelLocation,
    RelativeModelPath,
};
use nova_storage::{init_app_db, ModelRepository, RegisteredModel};
use sqlx::migrate::Migrator;

fn library() -> ModelLibrary {
    ModelLibrary {
        id: ModelLibraryId::new("internal").unwrap(),
        relative_root: RelativeModelPath::new("models").unwrap(),
    }
}

fn model(id: &str, availability: ModelAvailability) -> RegisteredModel {
    RegisteredModel {
        library: library(),
        descriptor: ModelDescriptor {
            id: ModelId::new(id).unwrap(),
            capability: "translation".into(),
            backend: "ctranslate2".into(),
            artifact_revision: None,
        },
        location: ModelLocation {
            model_id: ModelId::new(id).unwrap(),
            library_id: library().id,
            relative_path: RelativeModelPath::new("nllb").unwrap(),
        },
        availability,
        observed_at: 42,
    }
}

#[tokio::test]
async fn fresh_app_db_persists_internal_model_observations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let pool = init_app_db(path.to_str().unwrap()).await.unwrap();
    let repo = ModelRepository::new(&pool);
    repo.record_internal(
        &library(),
        &[model("nllb-200-distilled-600M", ModelAvailability::Missing)],
    )
    .await
    .unwrap();
    pool.close().await;

    let reopened = init_app_db(path.to_str().unwrap()).await.unwrap();
    let repo = ModelRepository::new(&reopened);
    let list = repo.list_internal().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].descriptor.id.as_str(), "nllb-200-distilled-600M");
    assert_eq!(list[0].availability, ModelAvailability::Missing);
    assert_eq!(list[0].library, library());
    assert_eq!(list[0].location.relative_path.as_str(), "nllb");
    assert_eq!(
        list[0].location.resolve(dir.path(), &list[0].library),
        Some(dir.path().join("models/nllb"))
    );

    repo.record_internal(
        &library(),
        &[model(
            "nllb-200-distilled-600M",
            ModelAvailability::Available,
        )],
    )
    .await
    .unwrap();
    assert_eq!(
        repo.list_internal().await.unwrap()[0].availability,
        ModelAvailability::Available
    );
    repo.record_internal(
        &library(),
        &[model("nllb-200-distilled-600M", ModelAvailability::Missing)],
    )
    .await
    .unwrap();
    assert_eq!(
        repo.list_internal().await.unwrap()[0].availability,
        ModelAvailability::Missing
    );
    let mut relocated_library = library();
    relocated_library.relative_root = RelativeModelPath::new("relocated-models").unwrap();
    let mut relocated = model("nllb-200-distilled-600M", ModelAvailability::Missing);
    relocated.library = relocated_library.clone();
    relocated.location.relative_path = RelativeModelPath::new("translation").unwrap();
    repo.record_internal(&relocated_library, &[relocated])
        .await
        .unwrap();
    let persisted = repo.list_internal().await.unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(
        persisted[0].descriptor.id.as_str(),
        "nllb-200-distilled-600M"
    );
    assert_eq!(
        persisted[0]
            .location
            .resolve(dir.path(), &persisted[0].library),
        Some(dir.path().join("relocated-models/translation"))
    );
    reopened.close().await;
}

#[tokio::test]
async fn existing_app_db_upgrades_without_changing_profiles_or_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old-app.db");
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .unwrap();
    let mut old: Migrator = sqlx::migrate!("./migrations");
    old.migrations =
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version == 1).cloned().collect());
    old.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO profiles (id, name, avatar_color, created_at) VALUES ('p1', 'Fixture', '#fff', 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_settings (key, value) VALUES ('fixture', 'preserved')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let upgraded = init_app_db(path.to_str().unwrap()).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM profiles WHERE id = 'p1'")
        .fetch_one(&upgraded)
        .await
        .unwrap();
    let value: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'fixture'")
        .fetch_one(&upgraded)
        .await
        .unwrap();
    assert_eq!((name.as_str(), value.as_str()), ("Fixture", "preserved"));
    assert!(ModelRepository::new(&upgraded)
        .list_internal()
        .await
        .unwrap()
        .is_empty());
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&upgraded)
        .await
        .unwrap()
        .is_empty());
    upgraded.close().await;
    init_app_db(path.to_str().unwrap())
        .await
        .unwrap()
        .close()
        .await;
}

#[tokio::test]
async fn invalid_batch_does_not_publish_partial_registry_state() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = ModelRepository::new(&pool);
    let mut bad = model("wrong", ModelAvailability::Unverified);
    bad.location.model_id = ModelId::new("different").unwrap();
    assert!(repo
        .record_internal(
            &library(),
            &[model("good", ModelAvailability::Available), bad]
        )
        .await
        .is_err());
    assert!(repo.list_internal().await.unwrap().is_empty());

    let mut bad_library = model("wrong-library", ModelAvailability::Unverified);
    bad_library.library.relative_root = RelativeModelPath::new("elsewhere").unwrap();
    assert!(repo
        .record_internal(&library(), &[bad_library])
        .await
        .is_err());
    assert!(repo.list_internal().await.unwrap().is_empty());
    pool.close().await;
}

#[tokio::test]
async fn corrupted_persisted_location_is_rejected_before_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = ModelRepository::new(&pool);
    repo.record_internal(
        &library(),
        &[model("fixture", ModelAvailability::Available)],
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE model_locations SET relative_path = '../outside' WHERE model_id = 'fixture'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo.list_internal().await.is_err());
    sqlx::query("UPDATE model_locations SET relative_path = 'nllb' WHERE model_id = 'fixture'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE model_libraries SET relative_root = '../outside' WHERE id = 'internal'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.list_internal().await.is_err());
    pool.close().await;
}
