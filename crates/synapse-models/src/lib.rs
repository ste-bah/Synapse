use std::{
    env, fmt,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use synapse_core::{DetectionBatch, error_codes};
use thiserror::Error;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDescriptor {
    pub id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub input_shape: Vec<usize>,
    pub class_map: Vec<String>,
}

impl ModelDescriptor {
    #[must_use]
    pub fn yolov10n_general(sha256: impl Into<String>, class_map: Vec<String>) -> Self {
        Self {
            id: "yolov10n_general".to_owned(),
            path: default_model_dir().join("yolov10n_general.onnx"),
            sha256: sha256.into(),
            input_shape: vec![1, 3, 640, 640],
            class_map,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelBackend {
    Cuda,
    DirectMl,
    #[default]
    Cpu,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectOpts {
    pub confidence_threshold: u16,
    pub max_detections: usize,
}

impl Default for DetectOpts {
    fn default() -> Self {
        Self {
            confidence_threshold: 50,
            max_detections: 100,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectionFrame {
    pub frame_seq: u64,
    pub width: u32,
    pub height: u32,
}

pub trait Detector: Send + Sync {
    /// Runs object detection for one frame.
    ///
    /// # Errors
    ///
    /// Implementations return `DETECTION_MODEL_NOT_LOADED` when no model is
    /// loaded and `DETECTION_MODEL_INFER_FAILED` when model execution fails.
    fn infer(&self, frame: DetectionFrame, opts: DetectOpts) -> ModelResult<DetectionBatch>;
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model download failed: {detail}")]
    DownloadFailed { detail: String },
    #[error("model hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("model load failed for {path}: {detail}")]
    LoadFailed { path: PathBuf, detail: String },
    #[error("no model backend was available; attempted {attempted:?}")]
    BackendUnavailable { attempted: Vec<ModelBackend> },
    #[error("detection model is not loaded: {detail}")]
    DetectionModelNotLoaded { detail: String },
    #[error("detection inference failed: {detail}")]
    DetectionInferFailed { detail: String },
}

impl ModelError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DownloadFailed { .. } => error_codes::MODEL_DOWNLOAD_FAILED,
            Self::HashMismatch { .. } => error_codes::MODEL_HASH_MISMATCH,
            Self::LoadFailed { .. } => error_codes::MODEL_LOAD_FAILED,
            Self::BackendUnavailable { .. } => error_codes::MODEL_BACKEND_UNAVAILABLE,
            Self::DetectionModelNotLoaded { .. } => error_codes::DETECTION_MODEL_NOT_LOADED,
            Self::DetectionInferFailed { .. } => error_codes::DETECTION_MODEL_INFER_FAILED,
        }
    }
}

#[must_use]
pub fn model_download_failed(source: &str) -> ModelError {
    let source = source.trim();
    let detail = if source.is_empty() {
        "model download source was empty".to_owned()
    } else {
        format!("model downloads are disabled in M1; side-load a verified model from {source}")
    };
    ModelError::DownloadFailed { detail }
}

#[derive(Debug)]
pub struct LoadedModel {
    descriptor: ModelDescriptor,
    selected_backend: ModelBackend,
    session_id: u64,
    session: SessionHandle,
}

impl LoadedModel {
    #[must_use]
    pub const fn session_id(&self) -> u64 {
        self.session_id
    }

    #[must_use]
    pub const fn selected_backend(&self) -> ModelBackend {
        self.selected_backend
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn session(&self) -> &SessionHandle {
        &self.session
    }
}

impl Detector for LoadedModel {
    fn infer(&self, frame: DetectionFrame, _opts: DetectOpts) -> ModelResult<DetectionBatch> {
        Ok(DetectionBatch {
            model_id: self.descriptor.id.clone(),
            frame_seq: frame.frame_seq,
            inferred_at: Utc::now(),
            items: Vec::new(),
        })
    }
}

pub enum SessionHandle {
    Placeholder,
    #[cfg(feature = "ort")]
    Ort(std::sync::Arc<std::sync::Mutex<ort::session::Session>>),
}

impl fmt::Debug for SessionHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Placeholder => formatter.write_str("Placeholder"),
            #[cfg(feature = "ort")]
            Self::Ort(_session) => formatter.write_str("Ort(Session)"),
        }
    }
}

pub struct SessionBuildResult {
    pub selected_backend: ModelBackend,
    pub session: SessionHandle,
}

pub trait SessionFactory {
    /// Creates one persistent session for a verified model descriptor.
    ///
    /// # Errors
    ///
    /// Returns `MODEL_BACKEND_UNAVAILABLE` if no requested execution provider
    /// can create a session, or `MODEL_LOAD_FAILED` if ONNX Runtime rejects the
    /// verified model file.
    fn create_session(
        &self,
        descriptor: &ModelDescriptor,
        providers: &[ModelBackend],
    ) -> ModelResult<SessionBuildResult>;
}

#[derive(Clone, Debug)]
pub struct ModelLoader {
    providers: Vec<ModelBackend>,
}

impl Default for ModelLoader {
    fn default() -> Self {
        Self {
            providers: default_provider_order(),
        }
    }
}

impl ModelLoader {
    #[must_use]
    pub const fn new(providers: Vec<ModelBackend>) -> Self {
        Self { providers }
    }

    #[must_use]
    pub fn providers(&self) -> &[ModelBackend] {
        &self.providers
    }

    /// Verifies the model file and creates a persistent runtime session.
    ///
    /// # Errors
    ///
    /// Returns `MODEL_HASH_MISMATCH` before session creation when file bytes do
    /// not match the descriptor, or the session factory's structured model
    /// error when runtime creation fails.
    pub fn load_with_factory(
        &self,
        descriptor: ModelDescriptor,
        factory: &dyn SessionFactory,
    ) -> ModelResult<LoadedModel> {
        let actual = sha256_file(&descriptor.path).map_err(|err| ModelError::LoadFailed {
            path: descriptor.path.clone(),
            detail: err.to_string(),
        })?;
        let expected = normalize_sha256(&descriptor.sha256);
        if actual != expected {
            return Err(ModelError::HashMismatch {
                path: descriptor.path,
                expected,
                actual,
            });
        }

        let build = factory.create_session(&descriptor, &self.providers)?;
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        tracing::info!(
            model_id = descriptor.id,
            session_id,
            backend = ?build.selected_backend,
            "loaded ONNX model"
        );
        Ok(LoadedModel {
            descriptor,
            selected_backend: build.selected_backend,
            session_id,
            session: build.session,
        })
    }

    /// Loads the canonical `YOLOv10n` model only if it exists.
    ///
    /// # Errors
    ///
    /// Returns the same structured errors as [`Self::load_with_factory`] when
    /// the file exists but verification or runtime creation fails.
    pub fn load_yolov10n_if_present(
        &self,
        descriptor: ModelDescriptor,
        factory: &dyn SessionFactory,
    ) -> ModelResult<Option<LoadedModel>> {
        if !descriptor.path.exists() {
            return Ok(None);
        }
        self.load_with_factory(descriptor, factory).map(Some)
    }

    /// Uses the built-in ORT session factory.
    ///
    /// # Errors
    ///
    /// Returns `MODEL_BACKEND_UNAVAILABLE` when this build has no ORT runtime
    /// feature or no requested execution provider can create a session.
    pub fn load(&self, descriptor: ModelDescriptor) -> ModelResult<LoadedModel> {
        self.load_with_factory(descriptor, &OrtSessionFactory)
    }
}

pub struct OrtSessionFactory;

#[cfg(not(feature = "ort"))]
impl SessionFactory for OrtSessionFactory {
    fn create_session(
        &self,
        _descriptor: &ModelDescriptor,
        providers: &[ModelBackend],
    ) -> ModelResult<SessionBuildResult> {
        Err(ModelError::BackendUnavailable {
            attempted: providers.to_vec(),
        })
    }
}

#[cfg(feature = "ort")]
impl SessionFactory for OrtSessionFactory {
    fn create_session(
        &self,
        descriptor: &ModelDescriptor,
        providers: &[ModelBackend],
    ) -> ModelResult<SessionBuildResult> {
        if providers.is_empty() {
            return Err(ModelError::BackendUnavailable {
                attempted: Vec::new(),
            });
        }

        let mut backend_failures = Vec::new();
        for provider in providers {
            match create_ort_session(descriptor, *provider) {
                Ok(session) => {
                    return Ok(SessionBuildResult {
                        selected_backend: *provider,
                        session: SessionHandle::Ort(std::sync::Arc::new(std::sync::Mutex::new(
                            session,
                        ))),
                    });
                }
                Err(ModelError::LoadFailed { .. }) if *provider == ModelBackend::Cpu => {
                    return Err(ModelError::LoadFailed {
                        path: descriptor.path.clone(),
                        detail: "CPU provider rejected verified model".to_owned(),
                    });
                }
                Err(err) => backend_failures.push((*provider, err.to_string())),
            }
        }

        tracing::warn!(failures = ?backend_failures, "all model backends failed");
        Err(ModelError::BackendUnavailable {
            attempted: providers.to_vec(),
        })
    }
}

#[cfg(feature = "ort")]
fn create_ort_session(
    descriptor: &ModelDescriptor,
    provider: ModelBackend,
) -> ModelResult<ort::session::Session> {
    use ort::{ep, session::Session};

    let mut builder = Session::builder().map_err(|err| ModelError::LoadFailed {
        path: descriptor.path.clone(),
        detail: err.to_string(),
    })?;
    let execution_provider = match provider {
        ModelBackend::Cuda => ep::CUDA::default().build().error_on_failure(),
        ModelBackend::DirectMl => ep::DirectML::default().build().error_on_failure(),
        ModelBackend::Cpu => ep::CPU::default()
            .with_arena_allocator(false)
            .build()
            .error_on_failure(),
    };
    builder = match builder.with_execution_providers([execution_provider]) {
        Ok(builder) => builder,
        Err(err) => {
            tracing::warn!(backend = ?provider, error = %err, "execution provider unavailable");
            return Err(ModelError::BackendUnavailable {
                attempted: vec![provider],
            });
        }
    };
    let model_bytes = std::fs::read(&descriptor.path).map_err(|err| ModelError::LoadFailed {
        path: descriptor.path.clone(),
        detail: err.to_string(),
    })?;
    builder
        .commit_from_memory(&model_bytes)
        .map_err(|err| ModelError::LoadFailed {
            path: descriptor.path.clone(),
            detail: err.to_string(),
        })
}

#[must_use]
pub fn default_provider_order() -> Vec<ModelBackend> {
    vec![
        ModelBackend::Cuda,
        ModelBackend::DirectMl,
        ModelBackend::Cpu,
    ]
}

#[must_use]
pub fn default_model_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("synapse")
        .join("models")
}

/// Computes a streaming SHA-256 digest for a model file.
///
/// # Errors
///
/// Returns an I/O error if the file cannot be opened or read.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

#[must_use]
pub fn normalize_sha256(value: &str) -> String {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("sha256:")
        .unwrap_or(trimmed)
        .to_ascii_lowercase()
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        if write!(&mut output, "{byte:02x}").is_err() {
            break;
        }
    }
    output
}
