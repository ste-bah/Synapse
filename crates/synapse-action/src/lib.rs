#![allow(unsafe_code)]

pub mod backend;
pub mod click_timing;
pub mod clipboard;
pub mod curve;
pub mod dynamics;
pub mod emitter;
pub mod error;
pub mod handle;
pub mod hotkey;
pub mod humanize;
pub mod invoke;
pub mod path;
pub mod rate_limit;
pub mod recovery;
pub mod safety;
pub mod stroke;
pub mod validation;
pub mod velocity;

pub use backend::{
    ActionBackend, BackendResolutionPolicy, ResolvedBackend,
    recording::{RecordedInput, RecordingBackend},
    resolve_backend, resolve_backend_with_policy,
    unavailable::HardwareUnavailableBackend,
    vigem::VigemBackend,
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
    BackendRateLimitControl, BackendRateLimitOverrideReadback, BackendRateLimitSnapshot, Backends,
    EmitState, HELD_KEY_MAX_DURATION_MS,
};
pub use error::{ActionError, ActionResult};
pub use handle::{
    ACTION_QUEUE_CAPACITY, ActionComboScheduler, ActionHandle, ActionMessage, RELEASE_ALL_HANDLE,
    SessionInputSessionSnapshot, SessionInputSnapshot, SessionKeyInput, SessionMouseButtonInput,
    SessionPadInput, SessionReleaseSummary,
};
pub use hotkey::{
    OperatorHotkeyGuard, OperatorHotkeyStatus, install_operator_hotkey, operator_hotkey_status,
    operator_release_epoch, operator_release_requested_since, request_release_interrupt,
    set_operator_hotkey_status,
};
pub use humanize::{HumanizeError, HumanizeResult, humanize_timed_path};
pub use invoke::{
    CoordinateFallbackPlan, ElementClickOutcome, click_element_or_fallback, invoke_element,
};
pub use path::{
    ArcLengthPath, DEFAULT_ARCLEN_LUT_SEGMENTS, PathError, PathResult, SpatialPath, path_length,
    path_point_at, path_point_at_arclen, sample_path, sample_path_arclen,
};
pub use rate_limit::{
    SOFTWARE_RATE_LIMIT_PER_S, TokenBucket, TokenBucketSnapshot, VIGEM_RATE_LIMIT_PER_S,
};
pub use recovery::{
    ActionCrashRecoveryReport, configure_crash_recovery_file,
    recover_stale_inputs_from_configured_path,
};
pub use safety::install_panic_hook;
pub use stroke::{
    STROKE_TICK_MS, StrokeError, StrokePlan, StrokeResult, plan_timed_stroke,
    screen_point_from_path_point,
};
pub use validation::{MAX_DRAG_DISTANCE_PX, validate_action};
pub use velocity::{
    TimedPathPoint, VelocityError, VelocityResult, fitts_law_duration_ms,
    normalized_velocity_at_time, position_at_time, sample_timed_arclen_path, sample_timed_path,
    time_at_position,
};
