//! Title artwork on demand (artwork v1): the TMDB title logo and text-free
//! backdrop of the title the UI is showing (hero, detail page, hover preview).
//!
//! Lazy by design: nothing is fetched in bulk. A title's art is read from
//! `tmdb_title_art`; when missing or stale (other metadata language, older
//! `ARTWORK_VERSION`) and the user enabled TMDB, one cached `/images` request
//! fills it. Offline, or with TMDB off, stored art is returned as is. The
//! command never fails the UI: any problem yields whatever art is known.

use nova_enrich::tmdb_art::{parse_images, select_title_art, ARTWORK_VERSION};
use nova_enrich::{TmdbClient, TmdbMediaKind};
use nova_storage::{
    ProfileSettingsRepository, StoredTitleArt, TitleArtMedia, TitleArtRepository,
    TmdbCacheRepository,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::enrich::{CachedTmdb, TmdbFetch, TmdbRequest};
use super::require_profile_pool;
use crate::ipc::ApiError;
use crate::state::AppState;

/// Which title the art is for.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TitleArtSubject {
    #[serde(rename_all = "camelCase")]
    Movie { entity_id: String },
    #[serde(rename_all = "camelCase")]
    Series {
        source_id: i64,
        series_title: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleArtView {
    pub logo_url: Option<String>,
    /// ISO 639-1 of the logo's text (`None`: language-neutral).
    pub logo_language: Option<String>,
    pub logo_width: Option<i64>,
    pub logo_height: Option<i64>,
    /// Text-free backdrop; `None` keeps the title's existing backdrop.
    pub backdrop_url: Option<String>,
}

impl From<StoredTitleArt> for TitleArtView {
    fn from(a: StoredTitleArt) -> Self {
        Self {
            logo_url: a.logo_url,
            logo_language: a.logo_language,
            logo_width: a.logo_width,
            logo_height: a.logo_height,
            backdrop_url: a.backdrop_url,
        }
    }
}

#[tauri::command]
pub async fn get_title_art(
    profile_id: String,
    subject: TitleArtSubject,
    state: State<'_, AppState>,
) -> Result<TitleArtView, ApiError> {
    let pool = require_profile_pool(&profile_id, &state).await?;
    let repo = TitleArtRepository::new(&pool);
    let Some((media, tmdb_id)) = tmdb_work(&pool, &subject).await? else {
        return Ok(TitleArtView::default());
    };
    let (client, language) = tmdb_access(&pool).await?;
    let cache = TmdbCacheRepository::new(&pool);
    let tmdb = client.as_ref().map(|client| CachedTmdb {
        client,
        cache: &cache,
    });
    Ok(resolve_title_art(&repo, tmdb.as_ref(), media, tmdb_id, &language).await)
}

/// The TMDB work a title is confidently matched to, if any.
pub(crate) async fn tmdb_work(
    pool: &sqlx::SqlitePool,
    subject: &TitleArtSubject,
) -> Result<Option<(TitleArtMedia, i64)>, ApiError> {
    let repo = TitleArtRepository::new(pool);
    Ok(match subject {
        TitleArtSubject::Movie { entity_id } => repo
            .movie_tmdb_id(entity_id)
            .await?
            .map(|id| (TitleArtMedia::Movie, id)),
        TitleArtSubject::Series {
            source_id,
            series_title,
        } => repo
            .series_tmdb_id(*source_id, series_title)
            .await?
            .map(|id| (TitleArtMedia::Tv, id)),
    })
}

/// The profile's TMDB client (only when enabled with a credential) and its
/// metadata language.
pub(crate) async fn tmdb_access(
    pool: &sqlx::SqlitePool,
) -> Result<(Option<TmdbClient>, String), ApiError> {
    let settings = ProfileSettingsRepository::new(pool).get().await?;
    let language = nova_core::metadata_language(&settings.metadata_language).to_string();
    let credential = settings
        .tmdb_api_key
        .filter(|k| settings.tmdb_enabled && !k.trim().is_empty());
    let client = credential.and_then(|key| TmdbClient::new(key, &language).ok());
    Ok((client, language))
}

/// Stored art when current; otherwise fetch (if a TMDB client is available),
/// store and return it. Falls back to stored (possibly stale) art on errors.
async fn resolve_title_art(
    repo: &TitleArtRepository<'_>,
    tmdb: Option<&impl TmdbFetch>,
    media: TitleArtMedia,
    tmdb_id: i64,
    language: &str,
) -> TitleArtView {
    let stored = repo.get(media, tmdb_id).await.ok().flatten();
    let current = stored
        .as_ref()
        .is_some_and(|a| a.artwork_version >= ARTWORK_VERSION && a.language == language);
    let Some(tmdb) = tmdb.filter(|_| !current) else {
        return stored.map(Into::into).unwrap_or_default();
    };
    let kind = match media {
        TitleArtMedia::Movie => TmdbMediaKind::Movie,
        TitleArtMedia::Tv => TmdbMediaKind::Tv,
    };
    let body = match tmdb.fetch(&TmdbRequest::Images(kind, tmdb_id)).await {
        Ok(body) => body,
        Err(e) => {
            tracing::warn!(error = ?e, tmdb_id, "title art: TMDB images unavailable");
            return stored.map(Into::into).unwrap_or_default();
        }
    };
    // A 404 or an unparsable body records "no art" (not retried until the
    // language or artwork version changes).
    let art = body
        .as_deref()
        .and_then(|b| parse_images(b).ok())
        .map(|images| select_title_art(&images, language))
        .unwrap_or_default();
    let row = StoredTitleArt {
        language: language.to_string(),
        logo_url: art.logo.as_ref().map(|l| l.url.clone()),
        logo_language: art.logo.as_ref().and_then(|l| l.language.clone()),
        logo_width: art.logo.as_ref().map(|l| l.width).filter(|w| *w > 0),
        logo_height: art.logo.as_ref().map(|l| l.height).filter(|h| *h > 0),
        backdrop_url: art.backdrop.map(|b| b.url),
        artwork_version: ARTWORK_VERSION,
    };
    if let Err(e) = repo.put(media, tmdb_id, &row).await {
        tracing::warn!(error = %e, tmdb_id, "title art: could not store");
    }
    row.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct Fixture {
        language: String,
        bodies: HashMap<String, Option<String>>,
        requested: RefCell<Vec<String>>,
    }

    impl Fixture {
        fn new(language: &str) -> Self {
            Self {
                language: language.into(),
                bodies: HashMap::new(),
                requested: RefCell::new(Vec::new()),
            }
        }
        fn with(mut self, key: &str, body: Option<&str>) -> Self {
            self.bodies.insert(key.into(), body.map(str::to_owned));
            self
        }
    }

    impl TmdbFetch for Fixture {
        fn language(&self) -> &str {
            &self.language
        }
        async fn fetch(&self, request: &TmdbRequest) -> Result<Option<String>, ApiError> {
            let key = request.cache_key(&self.language);
            self.requested.borrow_mut().push(key.clone());
            self.bodies
                .get(&key)
                .cloned()
                .ok_or_else(|| ApiError::Internal(format!("no fixture for {key}")))
        }
    }

    const IMAGES: &str = r#"{
      "logos": [
        {"file_path": "/en.png", "iso_639_1": "en", "vote_average": 5.0, "width": 800, "height": 300},
        {"file_path": "/fr.png", "iso_639_1": "fr", "vote_average": 4.0, "width": 600, "height": 200}
      ],
      "backdrops": [{"file_path": "/clean.jpg", "iso_639_1": null, "vote_average": 5.0}]
    }"#;

    async fn pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = nova_storage::init_profile_db(dir.path().join("p.db").to_str().unwrap())
            .await
            .unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn fetches_once_then_serves_stored_art() {
        let (_dir, pool) = pool().await;
        let repo = TitleArtRepository::new(&pool);
        let tmdb = Fixture::new("fr-FR").with("movie-images-v1::238::fr-FR", Some(IMAGES));

        let art = resolve_title_art(&repo, Some(&tmdb), TitleArtMedia::Movie, 238, "fr-FR").await;
        assert_eq!(
            art.logo_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/fr.png")
        );
        assert_eq!(art.logo_language.as_deref(), Some("fr"));
        assert_eq!((art.logo_width, art.logo_height), (Some(600), Some(200)));
        assert_eq!(
            art.backdrop_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w1280/clean.jpg")
        );

        let again = resolve_title_art(&repo, Some(&tmdb), TitleArtMedia::Movie, 238, "fr-FR").await;
        assert_eq!(again, art);
        assert_eq!(
            tmdb.requested.borrow().len(),
            1,
            "stored art is not refetched"
        );

        // Offline / TMDB off: stored art still shows.
        let offline =
            resolve_title_art(&repo, None::<&Fixture>, TitleArtMedia::Movie, 238, "fr-FR").await;
        assert_eq!(offline, art);
    }

    #[tokio::test]
    async fn a_language_change_refetches_and_a_404_is_remembered() {
        let (_dir, pool) = pool().await;
        let repo = TitleArtRepository::new(&pool);
        let fr = Fixture::new("fr-FR").with("movie-images-v1::238::fr-FR", Some(IMAGES));
        resolve_title_art(&repo, Some(&fr), TitleArtMedia::Movie, 238, "fr-FR").await;

        let en = Fixture::new("en-US").with("movie-images-v1::238", Some(IMAGES));
        let art = resolve_title_art(&repo, Some(&en), TitleArtMedia::Movie, 238, "en-US").await;
        assert_eq!(
            art.logo_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/en.png")
        );

        let gone = Fixture::new("en-US").with("tv-images-v1::9", None);
        let none = resolve_title_art(&repo, Some(&gone), TitleArtMedia::Tv, 9, "en-US").await;
        assert_eq!(none, TitleArtView::default());
        resolve_title_art(&repo, Some(&gone), TitleArtMedia::Tv, 9, "en-US").await;
        assert_eq!(gone.requested.borrow().len(), 1, "404 not retried");
    }

    #[tokio::test]
    async fn fetch_errors_fall_back_to_stored_art_without_overwriting_it() {
        let (_dir, pool) = pool().await;
        let repo = TitleArtRepository::new(&pool);
        let fr = Fixture::new("fr-FR").with("movie-images-v1::238::fr-FR", Some(IMAGES));
        let stored = resolve_title_art(&repo, Some(&fr), TitleArtMedia::Movie, 238, "fr-FR").await;

        // English wanted, network down: the French art is better than nothing.
        let failing = Fixture::new("en-US");
        let art =
            resolve_title_art(&repo, Some(&failing), TitleArtMedia::Movie, 238, "en-US").await;
        assert_eq!(art, stored);
        let row = repo.get(TitleArtMedia::Movie, 238).await.unwrap().unwrap();
        assert_eq!(row.language, "fr-FR", "a failed fetch never overwrites");
    }
}
