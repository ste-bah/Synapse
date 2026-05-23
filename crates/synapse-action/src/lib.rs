#![allow(unsafe_code)]

pub mod backend;
pub mod curve;
pub mod emitter;
pub mod error;
pub mod handle;

pub use backend::{
    ActionBackend, ResolvedBackend,
    recording::{RecordedInput, RecordingBackend},
    resolve_backend,
    unavailable::HardwareUnavailableBackend,
};
pub use curve::sample_curve;
pub use emitter::{
    ActionEmitter, ActionEmitterSnapshotHandle, ActionSnapshotMessage, ActionStateSnapshot,
    EmitState,
};
pub use error::{ActionError, ActionResult};
pub use handle::{ACTION_QUEUE_CAPACITY, ActionHandle, ActionMessage, RELEASE_ALL_HANDLE};
