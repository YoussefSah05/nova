//! SQLite storage layer using sqlx with compile-time checked queries.
//!
//! Phase 1: app.db — profiles, app_settings.
//! Phase 2: per-profile DBs — sources, channels, vod_items, series_episodes, jobs.
//! All writes go through repository functions; WAL mode, PRAGMA synchronous=NORMAL.

pub mod db;
pub mod error;
#[cfg(unix)]
pub mod profile_keys;
pub mod repositories;
#[cfg(unix)]
pub mod snapshots;

pub use db::{init_app_db, init_profile_db};
pub use error::StorageError;
pub use repositories::{
    from_blob, to_blob, CanonicalRepository, CaptionStyleRepository, CatalogSearchRepository,
    ChannelLogoRepository, ChannelNowAndNext, ChannelRepository, CompileStats, DirectoryLogoWrite,
    EmbeddingRepository, EnrichUpdate, EpgFeedStatus, EpgSource, EpgSourcesRepository,
    ExternalLibraryRecord, ExternalMatch, ExternalMatchRepository, ExternalMatchWrite,
    ExternalRegisteredModel, FailedAttempt, FailureStatus, Job, JobRepository, LineageEntry,
    LogoCandidate, MatchMethod, MatchPriority, MatchedMetadata, ModelRepository, MovieFilters,
    MovieMatchWork, MovieSearchCandidate, NowAndNext, PlaybackRepository, ProfileRepository,
    ProfileSettingsRepository, ProgramsRepository, Publication, PublicationMode,
    PublicationRepository, RegisteredModel, ResolutionReport, ResolvedStreamUrl, ResumePosition,
    SearchHit, SearchRepository, SeriesFlagSnapshot, SeriesRepository, SeriesTmdbQuery,
    SeriesTmdbWrite, SourceRepository, SourceSegment, StoredFacts, StoredTitleArt,
    StoredTranscript, TitleArtMedia, TitleArtRepository, TitleFactsRepository, TmdbCacheRepository,
    TranscriptLine, TranscriptRepository, TranslationRepository, TranslationRow, UnenrichedRow,
    VodRepository, WatchHistoryRepository, WatchSnapshot, TMDB_MOVIE,
};
