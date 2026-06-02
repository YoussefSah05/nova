//! Shared row/update types for the catalog-enrichment pass (Phase B). Both the
//! channel and VOD repositories expose `list_unenriched` / `apply_enrichment`
//! over these so the orchestrator (in the desktop crate) stays storage-agnostic.

/// A row that still needs an enrichment pass (`enrich_status IS NULL`).
#[derive(Debug, Clone)]
pub struct UnenrichedRow {
    pub id: i64,
    /// Display text to embed (channel name / movie clean title).
    pub text: String,
    /// Current category (empty/None → eligible for zero-shot genre fill).
    pub category: Option<String>,
}

/// The enrichment result to write back for one row.
#[derive(Debug, Clone)]
pub struct EnrichUpdate {
    pub id: i64,
    /// Cluster representative key (variants share one) — `None` if unique.
    pub dedup_group: Option<String>,
    /// Zero-shot genre to fill — `None` leaves the existing category untouched.
    pub category: Option<String>,
}

/// A movie row that still needs a TMDB pass (`tmdb_status IS NULL`).
#[derive(Debug, Clone)]
pub struct TmdbQuery {
    pub id: i64,
    /// Title to search (clean title preferred, raw name otherwise).
    pub title: String,
    pub year: Option<i64>,
}

/// The TMDB result to write back for one movie row. `matched = false` records a
/// negative result (`tmdb_status = 'unmatched'`) so a re-run skips it; a later
/// Phase D / LLM residual can revisit unmatched rows.
#[derive(Debug, Clone, Default)]
pub struct TmdbWrite {
    pub id: i64,
    pub canonical_title: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    /// All matched genres, comma-separated, for the `tmdb_genres` column.
    pub genres_csv: Option<String>,
    /// First matched genre, used only to fill an *empty* `category`.
    pub primary_genre: Option<String>,
    pub matched: bool,
}

/// A series (keyed by title within a source) needing a TMDB pass.
#[derive(Debug, Clone)]
pub struct SeriesTmdbQuery {
    pub series_title: String,
}

/// The TMDB result for one series, written to `series_meta`.
#[derive(Debug, Clone, Default)]
pub struct SeriesTmdbWrite {
    pub series_title: String,
    pub canonical_title: Option<String>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub overview: Option<String>,
    pub genres_csv: Option<String>,
    pub matched: bool,
}
