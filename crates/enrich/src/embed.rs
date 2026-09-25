//! Embedding backend. `Embedder` is the seam the pipeline depends on so jobs +
//! tests can use a mock; `FastEmbedder` is the on-device fastembed/ONNX impl.

use std::path::{Path, PathBuf};

use fastembed::{
    EmbeddingModel, InitOptions, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
};

use crate::EnrichError;

/// The model used for catalog text. Multilingual (worldwide IPTV), small + fast
/// (~118M, 384-dim). e5 models expect a task prefix; we use `passage:` for titles.
pub const MODEL: EmbeddingModel = EmbeddingModel::MultilingualE5Small;
/// Stable logical identity already used for persisted catalog embeddings.
pub const MODEL_ID: &str = "multilingual-e5-small";

/// Produces embedding vectors for short catalog strings. `Send + Sync` so it can
/// live in a background job behind an `Arc`.
pub trait Embedder: Send + Sync {
    /// Embed documents (catalog titles, transcript lines) — the corpus side.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EnrichError>;

    /// Embed a *search query*. e5 is asymmetric: the corpus is embedded with a
    /// `passage:` prefix (see [`Embedder::embed`]) and queries with a `query:`
    /// prefix. Using the wrong prefix measurably degrades recall, so semantic
    /// search must route the user's text through here, not `embed`.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EnrichError>;
}

/// On-device embedder (fastembed + ONNX Runtime). The model is downloaded to
/// `cache_dir` on first load (mirrors the whisper model flow) and reused after.
pub struct FastEmbedder {
    model: TextEmbedding,
}

impl FastEmbedder {
    /// Load only files already in this library's FastEmbed cache. This does not
    /// construct an HTTP client, consult HF_HOME, or write to the library.
    pub fn load_local(cache_dir: &Path) -> Result<Self, EnrichError> {
        let paths = local_artifact_paths(cache_dir)?;
        let read = |index: usize| {
            std::fs::read(&paths[index])
                .map_err(|_| EnrichError::Model("model cache could not be read".into()))
        };
        let tokenizer = TokenizerFiles {
            tokenizer_file: read(1)?,
            config_file: read(2)?,
            special_tokens_map_file: read(3)?,
            tokenizer_config_file: read(4)?,
        };
        let mut definition = UserDefinedEmbeddingModel::new(read(0)?, tokenizer)
            .with_quantization(TextEmbedding::get_quantization_mode(&MODEL));
        if let Some(pooling) = TextEmbedding::get_default_pooling_method(&MODEL) {
            definition = definition.with_pooling(pooling);
        }
        let model =
            TextEmbedding::try_new_from_user_defined(definition, InitOptions::new(MODEL).into())
                .map_err(|_| EnrichError::Model("cached embedding model could not load".into()))?;
        Ok(Self { model })
    }

    /// Structural availability only; not digest verification or inference.
    pub fn local_files_present(cache_dir: &Path) -> bool {
        local_artifact_paths(cache_dir).is_ok()
    }

    /// Load (downloading on first use) the embedding model into `cache_dir`.
    pub fn load(cache_dir: &Path, show_progress: bool) -> Result<Self, EnrichError> {
        let opts = InitOptions::new(MODEL)
            .with_cache_dir(cache_dir.to_path_buf())
            .with_show_download_progress(show_progress);
        let model = TextEmbedding::try_new(opts).map_err(|e| EnrichError::Model(e.to_string()))?;
        Ok(Self { model })
    }
}

fn local_artifact_paths(cache_dir: &Path) -> Result<Vec<PathBuf>, EnrichError> {
    let missing = || EnrichError::Model("embedding artifacts unavailable in this library".into());
    let root = cache_dir.canonicalize().map_err(|_| missing())?;
    let repo = hf_hub::Cache::new(root.clone()).model(MODEL.to_string());
    let info = TextEmbedding::get_model_info(&MODEL).map_err(|_| missing())?;
    let mut names = vec![
        info.model_file.as_str(),
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];
    names.extend(info.additional_files.iter().map(String::as_str));
    names
        .into_iter()
        .map(|name| {
            let path = repo
                .get(name)
                .ok_or_else(missing)?
                .canonicalize()
                .map_err(|_| missing())?;
            // HF snapshots normally point to blobs within the same cache. Reject
            // escaped refs/symlinks instead of reading outside the selected library.
            if !path.starts_with(&root)
                || !path.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
            {
                return Err(missing());
            }
            Ok(path)
        })
        .collect()
}

impl Embedder for FastEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EnrichError> {
        let docs: Vec<String> = texts.iter().map(|t| format!("passage: {t}")).collect();
        self.model
            .embed(docs, None)
            .map_err(|e| EnrichError::Model(e.to_string()))
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EnrichError> {
        let one = vec![format!("query: {query}")];
        let mut out = self
            .model
            .embed(one, None)
            .map_err(|e| EnrichError::Model(e.to_string()))?;
        out.pop()
            .ok_or_else(|| EnrichError::Model("empty embedding result".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(root: &Path) -> PathBuf {
        let model = root.join(format!("models--{}", MODEL.to_string().replace('/', "--")));
        std::fs::create_dir_all(model.join("refs")).unwrap();
        std::fs::write(model.join("refs/main"), "fixture-revision").unwrap();
        let snapshot = model.join("snapshots/fixture-revision");
        let info = TextEmbedding::get_model_info(&MODEL).unwrap();
        for file in [
            info.model_file.as_str(),
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            let path = snapshot.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"fixture").unwrap();
        }
        snapshot
    }

    #[test]
    fn offline_load_of_missing_or_marker_only_cache_never_creates_download_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FastEmbedder::load_local(dir.path()).is_err());
        assert_eq!(dir.path().read_dir().unwrap().count(), 0);
        std::fs::write(dir.path().join(".installed"), b"1").unwrap();
        assert!(!FastEmbedder::local_files_present(dir.path()));
        assert!(FastEmbedder::load_local(dir.path()).is_err());
        assert_eq!(dir.path().read_dir().unwrap().count(), 1);
    }

    #[test]
    fn discovery_accepts_complete_cache_and_rejects_missing_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = fixture(dir.path());
        assert!(FastEmbedder::local_files_present(dir.path()));
        std::fs::remove_file(snapshot.join("tokenizer.json")).unwrap();
        assert!(!FastEmbedder::local_files_present(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn cache_symlinks_must_stay_inside_the_selected_library() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let snapshot = fixture(dir.path());
        let path = snapshot.join("tokenizer.json");
        std::fs::remove_file(&path).unwrap();
        let blob = dir.path().join("blob");
        std::fs::write(&blob, b"fixture").unwrap();
        std::os::unix::fs::symlink(&blob, &path).unwrap();
        assert!(FastEmbedder::local_files_present(dir.path()));
        std::fs::remove_file(&path).unwrap();
        let blob = outside.path().join("blob");
        std::fs::write(&blob, b"fixture").unwrap();
        std::os::unix::fs::symlink(&blob, &path).unwrap();
        assert!(!FastEmbedder::local_files_present(dir.path()));
    }
}
