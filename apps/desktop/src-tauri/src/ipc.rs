// IPC types shared between the Rust backend and the TypeScript frontend.
// IPC contract: docs/ipc-contract.md

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum ApiError {
    NotFound(String),
    StorageError(String),
    Conflict(String),
    Internal(String),
}

impl From<nova_storage::StorageError> for ApiError {
    fn from(e: nova_storage::StorageError) -> Self {
        match e {
            nova_storage::StorageError::NotFound => ApiError::NotFound("not found".into()),
            other => ApiError::Internal(other.to_string()),
        }
    }
}
