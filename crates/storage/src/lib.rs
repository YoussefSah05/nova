//! SQLite storage layer using sqlx with compile-time checked queries.
//!
//! Phase 1: app.db — profiles, app_settings.
//! Phase 2: per-profile DBs — sources, channels, vod_items, series_episodes, jobs.
//! All writes go through repository functions; WAL mode, PRAGMA synchronous=NORMAL.

pub mod db;
pub mod error;
pub mod repositories;
#[cfg(unix)]
pub mod snapshots;

pub use db::{init_app_db, init_profile_db};
pub use error::StorageError;
pub use repositories::{
    from_blob, to_blob, CaptionStyleRepository, ChannelRepository, EmbeddingRepository,
    EnrichUpdate, EpgSource, EpgSourcesRepository, Job, JobRepository, NowAndNext,
    PlaybackRepository, ProfileRepository, ProfileSettingsRepository, ProgramsRepository,
    ResolvedStreamUrl, ResumePosition, SearchHit, SearchRepository, SeriesRepository,
    SeriesTmdbQuery, SeriesTmdbWrite, SourceRepository, SourceSegment, StoredTranscript,
    TmdbCacheRepository, TmdbQuery, TmdbWrite, TranscriptLine, TranscriptRepository,
    TranslationRepository, TranslationRow, UnenrichedRow, VodRepository, WatchHistoryRepository,
    WatchSnapshot,
};
