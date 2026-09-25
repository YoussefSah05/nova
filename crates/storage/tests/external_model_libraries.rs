use nova_core::models::{
    ModelAvailability, ModelDescriptor, ModelId, ModelLibrary, ModelLibraryId, ModelLocation,
    RelativeModelPath,
};
use nova_storage::{init_app_db, ExternalLibraryRecord, ModelRepository, RegisteredModel};
use sqlx::migrate::Migrator;

fn library(id: &str) -> ModelLibrary {
    ModelLibrary {
        id: ModelLibraryId::new(id).unwrap(),
        relative_root: RelativeModelPath::new("models").unwrap(),
    }
}

fn model(library: &ModelLibrary, availability: ModelAvailability) -> RegisteredModel {
    let id = ModelId::new("nllb-200-distilled-600M").unwrap();
    RegisteredModel {
        library: library.clone(),
        descriptor: ModelDescriptor {
            id: id.clone(),
            capability: "translation".into(),
            backend: "ctranslate2".into(),
            artifact_revision: None,
        },
        location: ModelLocation {
            model_id: id,
            library_id: library.id.clone(),
            relative_path: RelativeModelPath::new("nllb").unwrap(),
        },
        availability,
        observed_at: 42,
    }
}

fn grant(id: &str, online: bool) -> ExternalLibraryRecord {
    ExternalLibraryRecord {
        library: library(id),
        display_name: "Fixture library".into(),
        bookmark_data: b"opaque bookmark fixture".to_vec(),
        online,
        observed_at: 42,
    }
}

#[tokio::test]
async fn disconnect_and_reconnect_preserve_logical_identity_and_other_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let pool = init_app_db(path.to_str().unwrap()).await.unwrap();
    sqlx::query("INSERT INTO profiles (id, name, avatar_color, created_at) VALUES ('p1', 'Fixture', '#fff', 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_settings (key, value) VALUES ('fixture', 'preserved')")
        .execute(&pool)
        .await
        .unwrap();
    let repo = ModelRepository::new(&pool);
    let internal = library("internal");
    repo.record_internal(&internal, &[model(&internal, ModelAvailability::Available)])
        .await
        .unwrap();
    let first = grant("external-fixture", true);
    repo.record_external(
        &first,
        &[model(&first.library, ModelAvailability::Available)],
    )
    .await
    .unwrap();
    let other = grant("external-other", true);
    repo.record_external(
        &other,
        &[model(&other.library, ModelAvailability::Available)],
    )
    .await
    .unwrap();
    pool.close().await;

    let reopened = init_app_db(path.to_str().unwrap()).await.unwrap();
    let repo = ModelRepository::new(&reopened);
    let disconnected = grant("external-fixture", false);
    repo.record_external(
        &disconnected,
        &[model(&disconnected.library, ModelAvailability::Unavailable)],
    )
    .await
    .unwrap();
    let offline = repo.list_external_models().await.unwrap();
    assert_eq!(offline.len(), 2);
    let offline_model = offline
        .iter()
        .find(|m| m.model.library.id.as_str() == "external-fixture")
        .unwrap();
    let other_model = offline
        .iter()
        .find(|m| m.model.library.id.as_str() == "external-other")
        .unwrap();
    assert_eq!(
        offline_model.model.availability,
        ModelAvailability::Unavailable
    );
    assert_eq!(other_model.model.availability, ModelAvailability::Available);
    assert_eq!(
        offline_model.model.descriptor.id.as_str(),
        "nllb-200-distilled-600M"
    );
    assert_eq!(
        repo.list_internal().await.unwrap()[0].availability,
        ModelAvailability::Available
    );

    let restored = grant("external-fixture", true);
    repo.record_external(
        &restored,
        &[model(&restored.library, ModelAvailability::Available)],
    )
    .await
    .unwrap();
    let online = repo.list_external_models().await.unwrap();
    assert_eq!(online.len(), 2);
    let online_model = online
        .iter()
        .find(|m| m.model.library.id.as_str() == "external-fixture")
        .unwrap();
    assert_eq!(
        online_model.model.availability,
        ModelAvailability::Available
    );
    assert_eq!(
        online_model.model.descriptor.id,
        offline_model.model.descriptor.id
    );
    assert_eq!(online_model.model.location, offline_model.model.location);
    assert_eq!(
        repo.list_external_libraries().await.unwrap()[0].bookmark_data,
        b"opaque bookmark fixture"
    );
    let settings: String =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'fixture'")
            .fetch_one(&reopened)
            .await
            .unwrap();
    let profile: String = sqlx::query_scalar("SELECT name FROM profiles WHERE id = 'p1'")
        .fetch_one(&reopened)
        .await
        .unwrap();
    assert_eq!(
        (settings.as_str(), profile.as_str()),
        ("preserved", "Fixture")
    );
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&reopened)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn app_schema_two_upgrades_without_rewriting_existing_model_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
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
        std::borrow::Cow::Owned(old.iter().filter(|m| m.version <= 2).cloned().collect());
    old.run(&pool).await.unwrap();
    let internal = library("internal");
    ModelRepository::new(&pool)
        .record_internal(&internal, &[model(&internal, ModelAvailability::Missing)])
        .await
        .unwrap();
    sqlx::query("INSERT INTO app_settings (key, value) VALUES ('fixture', 'preserved')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let upgraded = init_app_db(path.to_str().unwrap()).await.unwrap();
    let repo = ModelRepository::new(&upgraded);
    assert_eq!(
        repo.list_internal().await.unwrap()[0].availability,
        ModelAvailability::Missing
    );
    assert!(repo.list_external_libraries().await.unwrap().is_empty());
    let setting: String =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'fixture'")
            .fetch_one(&upgraded)
            .await
            .unwrap();
    assert_eq!(setting, "preserved");
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&upgraded)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn invalid_external_observation_rolls_back_without_losing_grant() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init_app_db(dir.path().join("app.db").to_str().unwrap())
        .await
        .unwrap();
    let repo = ModelRepository::new(&pool);
    let first = grant("external-fixture", true);
    repo.record_external(
        &first,
        &[model(&first.library, ModelAvailability::Available)],
    )
    .await
    .unwrap();
    let changed = grant("external-fixture", false);
    assert!(repo
        .record_external(
            &changed,
            &[model(&changed.library, ModelAvailability::Available)]
        )
        .await
        .is_err());
    let persisted = repo.list_external_libraries().await.unwrap();
    assert!(persisted[0].online);
    assert_eq!(persisted[0].bookmark_data, first.bookmark_data);
    assert_eq!(
        repo.list_external_models().await.unwrap()[0]
            .model
            .availability,
        ModelAvailability::Available
    );
}
