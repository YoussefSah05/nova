-- Cleanup phase: per-profile viewing/behaviour settings.
--
-- A small key/value table in the profile DB. For now a single row under key
-- 'viewing' holds a JSON blob of nova_core::ProfileSettings (autoplay-next,
-- default caption mode, preferred audio/subtitle language, default playback
-- route). KV keeps it extensible without a migration per new setting.
CREATE TABLE IF NOT EXISTS profile_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
