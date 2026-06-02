//! SQLite storage layer using sqlx with compile-time checked queries.
//!
//! Phase 1: app.db — profiles, app_settings.
//! Phase 2: per-profile DBs — sources, channels, vod_items, series_episodes, jobs.
//! All writes go through repository functions; WAL mode, PRAGMA synchronous=NORMAL.

pub mod db;
pub mod error;
pub mod repositories;

pub use db::{init_app_db, init_profile_db};
pub use error::StorageError;
pub use repositories::{
    CaptionStyleRepository, ChannelRepository, EnrichUpdate, EpgSource, EpgSourcesRepository, Job,
    JobRepository, NowAndNext, PlaybackRepository, ProfileRepository, ProfileSettingsRepository,
    ProgramsRepository, ResolvedStreamUrl, ResumePosition, SeriesRepository, SeriesTmdbQuery,
    SeriesTmdbWrite, SourceRepository, StoredTranscript, TmdbCacheRepository, TmdbQuery, TmdbWrite,
    TranscriptRepository, UnenrichedRow, VodRepository, WatchHistoryRepository, WatchSnapshot,
};
