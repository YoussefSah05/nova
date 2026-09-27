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
    /// First matched genre, used only to fill an *empty* episode `category` so
    /// the genre-rail path lights up for series (mirrors `TmdbWrite`).
    pub primary_genre: Option<String>,
    pub matched: bool,
    /// Provenance: `trusted_id` | `search`.
    pub method: &'static str,
    pub confidence_bp: Option<u16>,
    pub matcher_version: u32,
    /// Secret-free provenance (JSON).
    pub evidence_json: String,
}
