CREATE TABLE IF NOT EXISTS profiles (
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    avatar_color  TEXT NOT NULL DEFAULT '#6366f1',
    created_at    INTEGER NOT NULL,
    last_used_at  INTEGER
);

CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
