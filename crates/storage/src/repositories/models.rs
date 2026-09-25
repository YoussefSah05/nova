//! App-wide metadata for the internal model library.

use nova_core::models::{
    ModelAvailability, ModelDescriptor, ModelId, ModelLibrary, ModelLibraryId, ModelLocation,
    RelativeModelPath,
};
use sqlx::SqlitePool;

use crate::error::StorageError;

#[derive(Debug, Clone)]
pub struct RegisteredModel {
    pub library: ModelLibrary,
    pub descriptor: ModelDescriptor,
    pub location: ModelLocation,
    pub availability: ModelAvailability,
    pub observed_at: i64,
}

/// The bookmark is a local access capability. Never serialize, log, or expose it
/// through IPC; only the macOS resolver should receive its bytes.
pub struct ExternalLibraryRecord {
    pub library: ModelLibrary,
    pub display_name: String,
    pub bookmark_data: Vec<u8>,
    pub online: bool,
    pub observed_at: i64,
}

#[derive(Debug, Clone)]
pub struct ExternalRegisteredModel {
    pub model: RegisteredModel,
    pub display_name: String,
}

pub struct ModelRepository<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ModelRepository<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Store one complete offline observation. A single transaction prevents
    /// readers from seeing a new descriptor without its relative location.
    pub async fn record_internal(
        &self,
        library: &ModelLibrary,
        models: &[RegisteredModel],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        let library_id = library.id.as_str();
        let root = library.relative_root.as_str();
        sqlx::query!(
            "INSERT INTO model_libraries (id, kind, relative_root) VALUES (?, 'internal', ?)
             ON CONFLICT(id) DO UPDATE SET relative_root = excluded.relative_root",
            library_id,
            root,
        )
        .execute(&mut *tx)
        .await?;

        for model in models {
            if model.library != *library
                || model.location.model_id != model.descriptor.id
                || model.location.library_id != library.id
            {
                return Err(StorageError::InvalidModelRecord);
            }
            let model_id = model.descriptor.id.as_str();
            let capability = &model.descriptor.capability;
            let backend = &model.descriptor.backend;
            let revision = model.descriptor.artifact_revision.as_deref();
            sqlx::query!(
                "INSERT INTO model_descriptors (id, capability, backend, artifact_revision)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET capability = excluded.capability,
                    backend = excluded.backend, artifact_revision = excluded.artifact_revision",
                model_id,
                capability,
                backend,
                revision,
            )
            .execute(&mut *tx)
            .await?;
            let relative_path = model.location.relative_path.as_str();
            let availability = model.availability.as_str();
            let observed_at = model.observed_at;
            sqlx::query!(
                "INSERT INTO model_locations
                    (model_id, library_id, relative_path, availability, observed_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(model_id) DO UPDATE SET library_id = excluded.library_id,
                    relative_path = excluded.relative_path,
                    availability = excluded.availability, observed_at = excluded.observed_at",
                model_id,
                library_id,
                relative_path,
                availability,
                observed_at,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_internal(&self) -> Result<Vec<RegisteredModel>, StorageError> {
        let rows = sqlx::query!(
            "SELECT d.id, d.capability, d.backend, d.artifact_revision,
                    l.library_id, b.relative_root, l.relative_path, l.availability, l.observed_at
             FROM model_descriptors d
             JOIN model_locations l ON l.model_id = d.id
             JOIN model_libraries b ON b.id = l.library_id
             WHERE b.kind = 'internal'
             ORDER BY d.id"
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id = ModelId::new(row.id)?;
                let library_id = ModelLibraryId::new(row.library_id)?;
                Ok(RegisteredModel {
                    library: ModelLibrary {
                        id: library_id.clone(),
                        relative_root: RelativeModelPath::new(row.relative_root)?,
                    },
                    descriptor: ModelDescriptor {
                        id: id.clone(),
                        capability: row.capability,
                        backend: row.backend,
                        artifact_revision: row.artifact_revision,
                    },
                    location: ModelLocation {
                        model_id: id,
                        library_id,
                        relative_path: RelativeModelPath::new(row.relative_path)?,
                    },
                    availability: ModelAvailability::parse(&row.availability)?,
                    observed_at: row.observed_at,
                })
            })
            .collect()
    }

    /// Persist one complete observation of a user-selected external folder.
    /// An offline observation keeps descriptors, bookmarks and relative paths.
    pub async fn record_external(
        &self,
        record: &ExternalLibraryRecord,
        models: &[RegisteredModel],
    ) -> Result<(), StorageError> {
        if record.library.id.as_str() == "internal"
            || record.bookmark_data.is_empty()
            || record.bookmark_data.len() > 1_048_576
            || record.display_name.is_empty()
            || record.display_name.len() > 128
            || record.display_name.chars().any(char::is_control)
        {
            return Err(StorageError::InvalidModelRecord);
        }
        let mut tx = self.pool.begin().await?;
        let library_id = record.library.id.as_str();
        let display_name = &record.display_name;
        let relative_root = record.library.relative_root.as_str();
        let bookmark_data = &record.bookmark_data;
        let availability = if record.online {
            "available"
        } else {
            "unavailable"
        };
        let observed_at = record.observed_at;
        sqlx::query!(
            "INSERT INTO external_model_libraries
                (id, display_name, relative_root, bookmark_data, availability, observed_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name,
                 relative_root = excluded.relative_root,
                 bookmark_data = excluded.bookmark_data,
                 availability = excluded.availability, observed_at = excluded.observed_at",
            library_id,
            display_name,
            relative_root,
            bookmark_data,
            availability,
            observed_at,
        )
        .execute(&mut *tx)
        .await?;

        // Every previously registered location belongs to this library. If a
        // drive vanishes (or a layout shrinks), no old available row survives.
        let fallback = if record.online {
            "missing"
        } else {
            "unavailable"
        };
        sqlx::query!(
            "UPDATE external_model_locations
             SET availability = ?, observed_at = ? WHERE library_id = ?",
            fallback,
            observed_at,
            library_id,
        )
        .execute(&mut *tx)
        .await?;

        for model in models {
            if model.library != record.library
                || model.location.library_id != record.library.id
                || model.location.model_id != model.descriptor.id
                || (record.online && model.availability == ModelAvailability::Unavailable)
                || (!record.online && model.availability != ModelAvailability::Unavailable)
            {
                return Err(StorageError::InvalidModelRecord);
            }
            let model_id = model.descriptor.id.as_str();
            let capability = &model.descriptor.capability;
            let backend = &model.descriptor.backend;
            let revision = model.descriptor.artifact_revision.as_deref();
            sqlx::query!(
                "INSERT INTO model_descriptors (id, capability, backend, artifact_revision)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET capability = excluded.capability,
                     backend = excluded.backend, artifact_revision = excluded.artifact_revision",
                model_id,
                capability,
                backend,
                revision,
            )
            .execute(&mut *tx)
            .await?;
            let relative_path = model.location.relative_path.as_str();
            let model_availability = model.availability.as_str();
            sqlx::query!(
                "INSERT INTO external_model_locations
                    (library_id, model_id, relative_path, availability, observed_at)
                 VALUES (?, ?, ?, ?, ?)
                 ON CONFLICT(library_id, model_id) DO UPDATE SET
                     relative_path = excluded.relative_path,
                     availability = excluded.availability,
                     observed_at = excluded.observed_at",
                library_id,
                model_id,
                relative_path,
                model_availability,
                observed_at,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_external_libraries(
        &self,
    ) -> Result<Vec<ExternalLibraryRecord>, StorageError> {
        let rows = sqlx::query!(
            "SELECT id, display_name, relative_root, bookmark_data, availability, observed_at
             FROM external_model_libraries ORDER BY id"
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let online = match row.availability.as_str() {
                    "available" => true,
                    "unavailable" => false,
                    _ => return Err(StorageError::InvalidModelRecord),
                };
                Ok(ExternalLibraryRecord {
                    library: ModelLibrary {
                        id: ModelLibraryId::new(row.id)?,
                        relative_root: RelativeModelPath::new(row.relative_root)?,
                    },
                    display_name: row.display_name,
                    bookmark_data: row.bookmark_data,
                    online,
                    observed_at: row.observed_at,
                })
            })
            .collect()
    }

    pub async fn list_external_models(&self) -> Result<Vec<ExternalRegisteredModel>, StorageError> {
        let rows = sqlx::query!(
            "SELECT d.id, d.capability, d.backend, d.artifact_revision,
                    l.library_id, b.relative_root, b.display_name, l.relative_path,
                    l.availability, l.observed_at
             FROM external_model_locations l
             JOIN external_model_libraries b ON b.id = l.library_id
             JOIN model_descriptors d ON d.id = l.model_id
             ORDER BY b.id, d.id"
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id = ModelId::new(row.id)?;
                let library_id = ModelLibraryId::new(row.library_id)?;
                let library = ModelLibrary {
                    id: library_id.clone(),
                    relative_root: RelativeModelPath::new(row.relative_root)?,
                };
                Ok(ExternalRegisteredModel {
                    model: RegisteredModel {
                        library,
                        descriptor: ModelDescriptor {
                            id: id.clone(),
                            capability: row.capability,
                            backend: row.backend,
                            artifact_revision: row.artifact_revision,
                        },
                        location: ModelLocation {
                            model_id: id,
                            library_id,
                            relative_path: RelativeModelPath::new(row.relative_path)?,
                        },
                        availability: ModelAvailability::parse(&row.availability)?,
                        observed_at: row.observed_at,
                    },
                    display_name: row.display_name,
                })
            })
            .collect()
    }
}
