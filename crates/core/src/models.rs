//! Logical model identities and paths within a registered model library.
//! The internal library is rooted at the application's data directory; no
//! machine-specific absolute path is part of a model's persisted identity.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ModelContractError {
    #[error("invalid model or library identifier")]
    InvalidId,
    #[error("model location must be a normalized relative path")]
    InvalidRelativePath,
    #[error("unknown model contract value")]
    UnknownValue,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelId(String);

impl ModelId {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelContractError> {
        let value = value.into();
        if !valid_id(&value) {
            return Err(ModelContractError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelLibraryId(String);

impl ModelLibraryId {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelContractError> {
        let value = value.into();
        if !valid_id(&value) {
            return Err(ModelContractError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativeModelPath(String);

impl RelativeModelPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelContractError> {
        let value = value.into();
        let path = Path::new(&value);
        if value.is_empty()
            || value.len() > 512
            || value.contains('\\')
            || value.chars().any(char::is_control)
            || value
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(ModelContractError::InvalidRelativePath);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn resolve(&self, root: &Path) -> PathBuf {
        root.join(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAvailability {
    Missing,
    Unverified,
    Available,
    /// The library is disconnected or its persisted access grant cannot be used.
    Unavailable,
}

impl ModelAvailability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unverified => "unverified",
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ModelContractError> {
        match value {
            "missing" => Ok(Self::Missing),
            "unverified" => Ok(Self::Unverified),
            "available" => Ok(Self::Available),
            "unavailable" => Ok(Self::Unavailable),
            _ => Err(ModelContractError::UnknownValue),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLibrary {
    pub id: ModelLibraryId,
    pub relative_root: RelativeModelPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: ModelId,
    pub capability: String,
    pub backend: String,
    /// None while the installed artifact revision is not pinned/attested.
    pub artifact_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelLocation {
    pub model_id: ModelId,
    pub library_id: ModelLibraryId,
    pub relative_path: RelativeModelPath,
}

impl ModelLocation {
    pub fn resolve(&self, app_data_dir: &Path, library: &ModelLibrary) -> Option<PathBuf> {
        (self.library_id == library.id).then(|| {
            self.relative_path
                .resolve(&library.relative_root.resolve(app_data_dir))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_paths_reject_escape_or_ambiguous_values() {
        assert!(ModelId::new("nllb-200-distilled-600M").is_ok());
        for id in ["", "../outside", "with space", "name/child"] {
            assert!(ModelId::new(id).is_err());
        }
        for path in ["", "/tmp/model", "../model", "models/../model", ".", "x\\y"] {
            assert!(RelativeModelPath::new(path).is_err(), "{path}");
        }
        assert!(RelativeModelPath::new("models/whisper").is_ok());
    }

    #[test]
    fn model_location_resolves_relative_to_its_library() {
        let library = ModelLibrary {
            id: ModelLibraryId::new("internal").unwrap(),
            relative_root: RelativeModelPath::new("models").unwrap(),
        };
        let location = ModelLocation {
            model_id: ModelId::new("whisper").unwrap(),
            library_id: library.id.clone(),
            relative_path: RelativeModelPath::new("whisper").unwrap(),
        };
        assert_eq!(
            location.resolve(Path::new("/app-data"), &library),
            Some(PathBuf::from("/app-data/models/whisper"))
        );
    }
}
