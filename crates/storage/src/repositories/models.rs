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
}
