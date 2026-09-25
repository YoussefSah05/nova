-- External folder access is an opaque macOS bookmark kept in the app DB.
-- Logical model IDs and relative locations remain separate from the grant.
-- Existing internal registry and profile/user tables are unchanged.
CREATE TABLE external_model_libraries (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    relative_root TEXT NOT NULL,
    bookmark_data BLOB NOT NULL,
    availability TEXT NOT NULL CHECK (availability IN ('available', 'unavailable')),
    observed_at INTEGER NOT NULL
);

CREATE TABLE external_model_locations (
    library_id TEXT NOT NULL REFERENCES external_model_libraries(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL REFERENCES model_descriptors(id),
    relative_path TEXT NOT NULL,
    availability TEXT NOT NULL CHECK (availability IN ('missing', 'unverified', 'available', 'unavailable')),
    observed_at INTEGER NOT NULL,
    PRIMARY KEY (library_id, model_id)
);

CREATE INDEX external_model_locations_model_idx ON external_model_locations(model_id);
