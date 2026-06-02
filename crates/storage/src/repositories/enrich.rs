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
