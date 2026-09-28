-- Playback health of each stream variant, recorded by the playback
-- coordinator: the variant selector demotes a source that failed more
-- recently than it last played, and recovery fails over away from it.
-- Local evidence only; no URLs or provider text.
CREATE TABLE IF NOT EXISTS variant_health (
    variant_id      TEXT    PRIMARY KEY REFERENCES stream_variants(id) ON DELETE CASCADE,
    failures        INTEGER NOT NULL DEFAULT 0,
    last_failure_at INTEGER,
    last_success_at INTEGER
);
