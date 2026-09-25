-- App-wide logical model metadata. Internal paths are relative to Nova's app
-- data directory, never absolute paths or credentials. Availability is the
-- last observed local layout state, refreshed by discovery before serving it.
CREATE TABLE model_libraries (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('internal')),
    relative_root TEXT NOT NULL
);

CREATE TABLE model_descriptors (
    id TEXT PRIMARY KEY NOT NULL,
    capability TEXT NOT NULL,
    backend TEXT NOT NULL,
    artifact_revision TEXT
);

CREATE TABLE model_locations (
    model_id TEXT PRIMARY KEY NOT NULL REFERENCES model_descriptors(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES model_libraries(id),
    relative_path TEXT NOT NULL,
    availability TEXT NOT NULL CHECK (availability IN ('missing', 'unverified', 'available')),
    observed_at INTEGER NOT NULL
);

CREATE INDEX model_locations_library_idx ON model_locations(library_id);
