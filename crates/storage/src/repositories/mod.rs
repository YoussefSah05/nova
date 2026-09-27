pub mod canonical;
pub mod caption_style;
pub mod catalog_search;
pub mod channels;
pub mod embeddings;
pub mod enrich;
pub mod epg_sources;
pub mod external_matches;
pub mod jobs;
pub mod models;
pub mod playback;
pub mod profile_settings;
pub mod profiles;
pub mod programs;
pub mod publication;
pub mod search;
pub mod series;
pub mod sources;
pub mod title_art;
pub mod tmdb_cache;
pub mod transcripts;
pub mod translations;
pub mod vod;
pub mod watch_history;

pub use canonical::{
    CanonicalRepository, CompileStats, LineageEntry, MovieFilters, ResolutionReport,
    MAX_REDIRECT_HOPS,
};
pub use caption_style::CaptionStyleRepository;
pub use catalog_search::{CatalogSearchRepository, MovieSearchCandidate, MOVIE_SEARCH_VERSION};
pub use channels::ChannelRepository;
pub use embeddings::{from_blob, to_blob, EmbeddingRepository};
pub use enrich::{EnrichUpdate, SeriesTmdbQuery, SeriesTmdbWrite, UnenrichedRow};
pub use epg_sources::{EpgSource, EpgSourcesRepository};
pub use external_matches::{
    ExternalMatch, ExternalMatchRepository, ExternalMatchWrite, MatchMethod, MatchPriority,
    MatchedMetadata, MovieMatchWork, TMDB_MOVIE,
};
pub use jobs::{Job, JobRepository};
pub use models::{
    ExternalLibraryRecord, ExternalRegisteredModel, ModelRepository, RegisteredModel,
};
pub use playback::{PlaybackRepository, ResolvedStreamUrl};
pub use profile_settings::ProfileSettingsRepository;
pub use profiles::ProfileRepository;
pub use programs::{NowAndNext, ProgramsRepository};
pub use publication::{
    FailedAttempt, FailureStatus, Publication, PublicationMode, PublicationRepository,
};
pub use search::{SearchHit, SearchRepository};
pub use series::{SeriesFlagSnapshot, SeriesRepository};
pub use sources::SourceRepository;
pub use title_art::{StoredTitleArt, TitleArtMedia, TitleArtRepository};
pub use tmdb_cache::TmdbCacheRepository;
pub use transcripts::{SourceSegment, StoredTranscript, TranscriptLine, TranscriptRepository};
pub use translations::{TranslationRepository, TranslationRow};
pub use vod::VodRepository;
pub use watch_history::{ResumePosition, WatchHistoryRepository, WatchSnapshot};
