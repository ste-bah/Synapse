use serde::{Deserialize, Serialize};
use synapse_core::DetectionBatch;

mod download;
mod ep;
mod error;
mod session;
mod verify;

pub use download::{ModelDescriptor, default_model_dir, model_download_failed};
pub use ep::{ModelBackend, default_provider_order};
pub use error::{
    ModelError, ModelResult, detection_infer_failed, detection_model_not_loaded, detection_no_frame,
};
pub use session::{
    LoadedModel, ModelLoader, OrtSessionFactory, SessionBuildResult, SessionFactory, SessionHandle,
};
pub use verify::{normalize_sha256, sha256_file};

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

impl DetectionFrame {
    /// Validates that a detection frame carries image pixels.
    ///
    /// # Errors
    ///
    /// Returns `DETECTION_NO_FRAME` when the frame has a zero width or height.
    pub fn validate(self) -> ModelResult<Self> {
        if self.width == 0 || self.height == 0 {
            return Err(detection_no_frame(format!(
                "frame {} has invalid dimensions {}x{}",
                self.frame_seq, self.width, self.height
            )));
        }
        Ok(self)
    }
}

pub trait Detector: Send + Sync {
    /// Runs object detection for one frame.
    ///
    /// # Errors
    ///
    /// Implementations return `DETECTION_MODEL_NOT_LOADED` when no model is
    /// loaded, `DETECTION_NO_FRAME` when no image pixels are available, and
    /// `DETECTION_MODEL_INFER_FAILED` when model execution fails.
    fn infer(&self, frame: DetectionFrame, opts: DetectOpts) -> ModelResult<DetectionBatch>;
}
