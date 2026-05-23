//! Background job scheduler and worker pool.
//!
//! Implemented in Phase 2+: priority queue over the `jobs` SQLite table,
//! concurrency limits per job kind (e.g. max 2 transcription jobs on 16GB+),
//! progress events emitted to the Tauri event bus.
//! Job kinds: transcribe_vod, translate_transcript, refresh_epg, refresh_playlist, cache_logo.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
