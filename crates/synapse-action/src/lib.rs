#![allow(unsafe_code)]

pub mod backend;
pub mod click_timing;
pub mod clipboard;
pub mod curve;
pub mod dynamics;
pub mod emitter;
pub mod error;
pub mod handle;
pub mod invoke;
pub mod rate_limit;
pub mod safety;
pub mod validation;

pub use backend::{
    ActionBackend, ResolvedBackend,
    recording::{RecordedInput, RecordingBackend},
    resolve_backend,
    unavailable::HardwareUnavailableBackend,
};
pub use click_timing::{
    DoubleClickTiming, cached_double_click_timing, initialize_double_click_timing_cache,
    inter_click_delay_ms_for_window,
};
pub use clipboard::{
    ClipboardFormat, clear as clear_clipboard, read_text as read_clipboard_text,
    write_text as write_clipboard_text,
};
pub use curve::sample_curve;
pub use dynamics::{BIGRAMS, KeystrokeEvent, ModifierMask, sample_typing_schedule};
pub use emitter::{
    ActionEmitter, ActionEmitterSnapshotHandle, ActionSnapshotMessage, ActionStateSnapshot,
    EmitState, HELD_KEY_MAX_DURATION_MS,
};
pub use error::{ActionError, ActionResult};
pub use handle::{ACTION_QUEUE_CAPACITY, ActionHandle, ActionMessage, RELEASE_ALL_HANDLE};
pub use invoke::{
    CoordinateFallbackPlan, ElementClickOutcome, click_element_or_fallback, invoke_element,
};
pub use rate_limit::{
    SOFTWARE_RATE_LIMIT_PER_S, TokenBucket, TokenBucketSnapshot, VIGEM_RATE_LIMIT_PER_S,
};
pub use safety::install_panic_hook;
pub use validation::{MAX_DRAG_DISTANCE_PX, validate_action};
