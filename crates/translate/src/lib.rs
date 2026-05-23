//! Translation pipeline with multiple backends.
//!
//! Implemented in Phase 7:
//!   - `NllbBackend`: NLLB-200 distilled (600M / 1.3B) via CTranslate2 with Metal backend.
//!   - `DeeplBackend`: DeepL REST API (opt-in, requires API key).
//!   - `GoogleBackend`: Google Translate REST API (opt-in, requires API key).
//!
//! Per-segment caching in SQLite means re-translating is instant.
//! See docs/reference/translate-config.md for backend selection, BCP-47 ↔ NLLB code mapping,
//! batching rules, and caching strategy.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
