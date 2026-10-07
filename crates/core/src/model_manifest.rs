//! The bundled model manifest: which models Nova knows about, what each one
//! does, which backend runs it, and which files make up an install.
//!
//! The manifest is data, not code. Adding a model (or changing a default
//! after a benchmark) edits the manifest; discovery, placement and the
//! runtime read it instead of hardcoding model ids. A manifest entry is a
//! claim about an artifact, never proof that it is installed: availability is
//! always observed on disk.

use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::models::{ModelAvailability, ModelId, RelativeModelPath};

/// The manifest schema this build understands.
pub const MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("model manifest is not valid JSON: {0}")]
    Parse(String),
    #[error("unsupported model manifest schema {0}")]
    Schema(u32),
    #[error("model manifest entry {0}: {1}")]
    Entry(String, &'static str),
}

/// What a model is for. Stored as the registry's `capability` column, so the
/// string forms are a persisted contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    Transcription,
    Translation,
    Embedding,
    Generation,
    VoiceActivity,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Transcription => "transcription",
            Self::Translation => "translation",
            Self::Embedding => "embedding",
            Self::Generation => "generation",
            Self::VoiceActivity => "voice-activity",
        }
    }
}

/// The engine family that loads a model. Stored as the registry's `backend`
/// column, so the string forms are a persisted contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum BackendKind {
    #[serde(rename = "whisper.cpp")]
    WhisperCpp,
    #[serde(rename = "fastembed")]
    FastEmbed,
    /// Encoder-decoder translation (Marian, NLLB) on the app's single ONNX Runtime.
    #[serde(rename = "onnx-seq2seq")]
    OnnxSeq2Seq,
    /// A GGUF model served by a llama.cpp `llama-server` subprocess.
    #[serde(rename = "llama-server")]
    LlamaServer,
    /// Apple's on-device frameworks; the OS owns the assets.
    #[serde(rename = "apple")]
    Apple,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WhisperCpp => "whisper.cpp",
            Self::FastEmbed => "fastembed",
            Self::OnnxSeq2Seq => "onnx-seq2seq",
            Self::LlamaServer => "llama-server",
            Self::Apple => "apple",
        }
    }
}

/// How discovery decides whether an install is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Probe {
    /// Every listed file must exist with its pinned size.
    Files,
    /// The legacy whisper.cpp layout (GGML weights plus a Core ML encoder).
    WhisperLayout,
    /// FastEmbed's own cache layout, which Nova does not pin file by file.
    FastembedCache,
}

/// How quickly a model answers, which decides where a pipeline may use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LatencyClass {
    /// Fit for live captions and per-segment translation.
    Streaming,
    /// Background or on-request work only.
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    /// Relative to the model's own directory.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// A translation direction, as ISO 639 codes ("en", "fr", "ar").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguagePair {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub id: String,
    /// The upstream artifact revision the pinned files come from.
    pub revision: String,
    pub capability: Capability,
    pub backend: BackendKind,
    /// The model's directory, relative to a library's model root.
    pub relative_path: String,
    pub probe: Probe,
    #[serde(default)]
    pub files: Vec<ManifestFile>,
    /// Spoken or written languages; empty means language-independent.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Translation directions; only meaningful for translation models.
    #[serde(default)]
    pub pairs: Vec<LanguagePair>,
    pub licence: String,
    /// Non-commercial licence: allowed while Nova is non-commercial, and
    /// always labelled in Settings.
    #[serde(default)]
    pub non_commercial: bool,
    pub latency_class: LatencyClass,
    /// A resource-policy estimate, not a measurement. None means unknown.
    #[serde(default)]
    pub est_resident_bytes: Option<u64>,
    /// The model this one stands in for when that model is unreachable (for
    /// example a small internal model while the external SSD is offline).
    #[serde(default)]
    pub fallback_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub schema: u32,
    pub models: Vec<ManifestEntry>,
}

fn valid_language(code: &str) -> bool {
    (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl ModelManifest {
    /// Parse and validate. An invalid manifest is a build defect, so callers
    /// treat the error as fatal for discovery rather than skipping entries.
    pub fn parse(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self =
            serde_json::from_str(json).map_err(|e| ManifestError::Parse(e.to_string()))?;
        if manifest.schema != MANIFEST_SCHEMA {
            return Err(ManifestError::Schema(manifest.schema));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        let mut ids = HashSet::new();
        for entry in &self.models {
            let fail = |why| Err(ManifestError::Entry(entry.id.clone(), why));
            if ModelId::new(entry.id.as_str()).is_err() {
                return fail("invalid id");
            }
            if !ids.insert(entry.id.as_str()) {
                return fail("duplicate id");
            }
            if entry.revision.is_empty() {
                return fail("empty revision");
            }
            if RelativeModelPath::new(entry.relative_path.as_str()).is_err() {
                return fail("invalid relative_path");
            }
            match entry.probe {
                Probe::Files if entry.files.is_empty() => {
                    return fail("files probe lists no files")
                }
                Probe::Files => {}
                _ if !entry.files.is_empty() => return fail("only the files probe pins files"),
                _ => {}
            }
            let mut paths = HashSet::new();
            for file in &entry.files {
                if RelativeModelPath::new(file.path.as_str()).is_err() {
                    return fail("invalid file path");
                }
                if !paths.insert(file.path.as_str()) {
                    return fail("duplicate file path");
                }
                if !valid_sha256(&file.sha256) {
                    return fail("sha256 must be 64 lowercase hex digits");
                }
                if file.bytes == 0 {
                    return fail("file size must be positive");
                }
            }
            let probe_fits = match entry.probe {
                Probe::Files => true,
                Probe::WhisperLayout => entry.backend == BackendKind::WhisperCpp,
                Probe::FastembedCache => entry.backend == BackendKind::FastEmbed,
            };
            if !probe_fits {
                return fail("probe does not match the backend");
            }
            if !entry.languages.iter().all(|l| valid_language(l)) {
                return fail("invalid language code");
            }
            let is_translation = entry.capability == Capability::Translation;
            if is_translation == entry.pairs.is_empty() {
                return fail("pairs are required for, and only for, translation models");
            }
            if entry.pairs.iter().any(|p| {
                !valid_language(&p.source) || !valid_language(&p.target) || p.source == p.target
            }) {
                return fail("invalid language pair");
            }
        }
        for entry in &self.models {
            let Some(primary) = &entry.fallback_of else {
                continue;
            };
            let fail = |why| Err(ManifestError::Entry(entry.id.clone(), why));
            if primary == &entry.id {
                return fail("a model cannot be its own fallback");
            }
            let Some(p) = self.get(primary) else {
                return fail("fallback_of names an unknown model");
            };
            if p.capability != entry.capability {
                return fail("fallback has a different capability");
            }
            // One level only: a resolver never walks a chain or a cycle.
            if p.fallback_of.is_some() {
                return fail("a fallback cannot stand in for another fallback");
            }
            if !p.pairs.iter().all(|pair| entry.pairs.contains(pair)) {
                return fail("fallback does not cover the primary's language pairs");
            }
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ManifestEntry> {
        self.models.iter().find(|entry| entry.id == id)
    }

    /// Models able to translate `source` → `target` directly.
    pub fn translators_for<'a>(
        &'a self,
        source: &'a str,
        target: &'a str,
    ) -> impl Iterator<Item = &'a ManifestEntry> + 'a {
        self.models.iter().filter(move |entry| {
            entry
                .pairs
                .iter()
                .any(|p| p.source == source && p.target == target)
        })
    }

    /// Fallbacks registered for `id`, in manifest order.
    pub fn fallbacks_of<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a ManifestEntry> + 'a {
        self.models
            .iter()
            .filter(move |entry| entry.fallback_of.as_deref() == Some(id))
    }
}

/// Cheap, offline availability for a `files` probe: presence and pinned size
/// only. Hashing gigabytes on every status request would be far too slow, so
/// a matching size is `Available` only after [`verify_files`] has attested
/// the content; until then a complete-looking install is `Unverified`.
///
/// `attested` is the caller's record that the current files were verified.
pub fn files_availability(
    model_dir: &Path,
    files: &[ManifestFile],
    attested: bool,
) -> ModelAvailability {
    let mut present = 0usize;
    let mut sizes_match = true;
    for file in files {
        match model_dir.join(&file.path).metadata() {
            Ok(meta) if meta.is_file() => {
                present += 1;
                sizes_match &= meta.len() == file.bytes;
            }
            Ok(_) => sizes_match = false,
            // Permission or I/O errors are evidence of something on disk.
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                present += 1;
                sizes_match = false;
            }
            Err(_) => {}
        }
    }
    if present == 0 {
        ModelAvailability::Missing
    } else if present == files.len() && sizes_match && attested {
        ModelAvailability::Available
    } else {
        ModelAvailability::Unverified
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("model file {0} is missing or unreadable")]
    Unreadable(String),
    #[error("model file {0} does not match its pinned checksum")]
    Mismatch(String),
}

/// Full content verification against the pinned sha256 values. Blocking and
/// proportional to model size: run it off the async executor, once per
/// install, never on a status poll.
pub fn verify_files(model_dir: &Path, files: &[ManifestFile]) -> Result<(), VerifyError> {
    for file in files {
        let path = model_dir.join(&file.path);
        let unreadable = || VerifyError::Unreadable(file.path.clone());
        let mut reader = std::fs::File::open(&path).map_err(|_| unreadable())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let read = reader.read(&mut buffer).map_err(|_| unreadable())?;
            if read == 0 {
                break;
            }
            total += read as u64;
            hasher.update(&buffer[..read]);
        }
        let digest: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if total != file.bytes || digest != file.sha256 {
            return Err(VerifyError::Mismatch(file.path.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn entry_json(extra: &str) -> String {
        format!(
            r#"{{"schema":1,"models":[{{"id":"opus-mt-en-fr","revision":"r1",
            "capability":"translation","backend":"onnx-seq2seq","relative_path":"opus/en-fr",
            "probe":"files","files":[{{"path":"model.onnx","sha256":"{HELLO_SHA}","bytes":5}}],
            "pairs":[{{"source":"en","target":"fr"}}],"licence":"CC-BY-4.0",
            "latency_class":"streaming"{extra}}}]}}"#
        )
    }

    #[test]
    fn parses_a_valid_entry() {
        let manifest = ModelManifest::parse(&entry_json("")).unwrap();
        let entry = manifest.get("opus-mt-en-fr").unwrap();
        assert_eq!(entry.backend, BackendKind::OnnxSeq2Seq);
        assert_eq!(manifest.translators_for("en", "fr").count(), 1);
        assert_eq!(manifest.translators_for("fr", "en").count(), 0);
    }

    #[test]
    fn rejects_unknown_fields_bad_schema_and_bad_references() {
        assert!(matches!(
            ModelManifest::parse(&entry_json(r#","surprise":1"#)),
            Err(ManifestError::Parse(_))
        ));
        assert_eq!(
            ModelManifest::parse(&entry_json("").replace(r#""schema":1"#, r#""schema":2"#)),
            Err(ManifestError::Schema(2))
        );
        assert!(matches!(
            ModelManifest::parse(&entry_json(r#","fallback_of":"nope""#)),
            Err(ManifestError::Entry(
                _,
                "fallback_of names an unknown model"
            ))
        ));
        assert!(matches!(
            ModelManifest::parse(&entry_json("").replace("opus/en-fr", "../escape")),
            Err(ManifestError::Entry(_, "invalid relative_path"))
        ));
    }

    #[test]
    fn fallbacks_are_one_level_and_cover_the_primary() {
        let two = |second: &str| {
            entry_json("").replace(
                r#""streaming"}]}"#,
                &format!(
                    r#""streaming"}},{{"id":"b","revision":"r","capability":"translation",
                    "backend":"onnx-seq2seq","relative_path":"b","probe":"files",
                    "files":[{{"path":"m","sha256":"{HELLO_SHA}","bytes":5}}],
                    "licence":"x","latency_class":"batch",{second}}}]}}"#
                ),
            )
        };
        let covering =
            two(r#""pairs":[{"source":"en","target":"fr"}],"fallback_of":"opus-mt-en-fr""#);
        assert!(ModelManifest::parse(&covering).is_ok(), "{covering}");
        let narrow =
            two(r#""pairs":[{"source":"fr","target":"en"}],"fallback_of":"opus-mt-en-fr""#);
        assert!(matches!(
            ModelManifest::parse(&narrow),
            Err(ManifestError::Entry(
                _,
                "fallback does not cover the primary's language pairs"
            ))
        ));
        let cycle = two(r#""pairs":[{"source":"en","target":"fr"}],"fallback_of":"opus-mt-en-fr""#)
            .replace(
                r#""latency_class":"streaming""#,
                r#""latency_class":"streaming","fallback_of":"b""#,
            );
        assert!(matches!(
            ModelManifest::parse(&cycle),
            Err(ManifestError::Entry(
                _,
                "a fallback cannot stand in for another fallback"
            ))
        ));
    }

    #[test]
    fn legacy_probes_only_fit_their_backend() {
        let json = entry_json("")
            .replace(r#""probe":"files","files":[{"path":"model.onnx","sha256":"2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824","bytes":5}]"#, r#""probe":"whisper-layout""#);
        assert!(matches!(
            ModelManifest::parse(&json),
            Err(ManifestError::Entry(_, "probe does not match the backend"))
        ));
    }

    #[test]
    fn translation_models_must_declare_pairs() {
        let json = entry_json("").replace(r#""pairs":[{"source":"en","target":"fr"}],"#, "");
        assert!(matches!(
            ModelManifest::parse(&json),
            Err(ManifestError::Entry(
                _,
                "pairs are required for, and only for, translation models"
            ))
        ));
    }

    #[test]
    fn bundled_manifest_is_valid() {
        let json = include_str!("../../../apps/desktop/src-tauri/resources/models.manifest.json");
        ModelManifest::parse(json).unwrap();
    }

    #[test]
    fn availability_is_cheap_and_never_claims_unattested_content() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![ManifestFile {
            path: "model.onnx".into(),
            sha256: HELLO_SHA.into(),
            bytes: 5,
        }];
        assert_eq!(
            files_availability(dir.path(), &files, false),
            ModelAvailability::Missing
        );
        std::fs::write(dir.path().join("model.onnx"), b"hell").unwrap();
        assert_eq!(
            files_availability(dir.path(), &files, true),
            ModelAvailability::Unverified
        );
        std::fs::write(dir.path().join("model.onnx"), b"hello").unwrap();
        assert_eq!(
            files_availability(dir.path(), &files, false),
            ModelAvailability::Unverified
        );
        assert_eq!(
            files_availability(dir.path(), &files, true),
            ModelAvailability::Available
        );
    }

    #[test]
    fn verification_checks_content_not_just_size() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![ManifestFile {
            path: "model.onnx".into(),
            sha256: HELLO_SHA.into(),
            bytes: 5,
        }];
        assert_eq!(
            verify_files(dir.path(), &files),
            Err(VerifyError::Unreadable("model.onnx".into()))
        );
        std::fs::write(dir.path().join("model.onnx"), b"jello").unwrap();
        assert_eq!(
            verify_files(dir.path(), &files),
            Err(VerifyError::Mismatch("model.onnx".into()))
        );
        std::fs::write(dir.path().join("model.onnx"), b"hello").unwrap();
        assert_eq!(verify_files(dir.path(), &files), Ok(()));
    }
}
