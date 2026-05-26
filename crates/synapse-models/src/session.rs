use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use chrono::Utc;
use synapse_core::DetectionBatch;

use crate::{
    DetectOpts, DetectionFrame, Detector, ModelBackend, ModelDescriptor, ModelError, ModelResult,
    default_provider_order, normalize_sha256, sha256_file,
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

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
        let frame = frame.validate()?;
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
            match crate::ep::create_ort_session(descriptor, *provider) {
                Ok(session) => {
                    return Ok(SessionBuildResult {
                        selected_backend: *provider,
                        session: SessionHandle::Ort(std::sync::Arc::new(std::sync::Mutex::new(
                            session,
                        ))),
                    });
                }
                Err(ModelError::LoadFailed { detail, .. }) if *provider == ModelBackend::Cpu => {
                    return Err(ModelError::LoadFailed {
                        path: descriptor.path.clone(),
                        detail: format!("CPU provider rejected verified model: {detail}"),
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
