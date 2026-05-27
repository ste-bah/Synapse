pub mod defaults;
pub mod error_codes;
pub mod filter;
pub mod firmware_version;
pub mod retention;
pub mod types;
pub mod usb_identity;

pub use defaults::SCHEMA_VERSION;
pub use firmware_version::{
    EXPECTED_FW_MAJOR, SYNAPSE_PICO_HID_BUILD_HASH_LEN, SYNAPSE_PICO_HID_FW_MAJOR,
    SYNAPSE_PICO_HID_FW_MINOR, SYNAPSE_PICO_HID_FW_PATCH,
};
pub use types::{
    AccessibleNode, AccessibleQuery, AccessibleQueryScope, AccessibleSubtree, Action, AimCurve,
    AimNaturalParams, AimStyle, AimTarget, AudioContext, AudioCue, AudioEvent, Backend,
    ButtonAction, ClipboardSummary, ComboInput, ComboStep, DataPredicate, DetectedEntity,
    Detection, DetectionBatch, DirectionEstimate, EVENT_FILTER_MAX_DEPTH, ElementId,
    ElementIdParseError, ElementIdParts, EntityId, Event, EventExtension, EventFilter,
    EventFilterValidationError, EventRef, EventSource, EventSummary, FocusedElement,
    ForegroundContext, FsEvent, FsEventKind, GamepadController, GamepadReport, Health,
    HudExtractor, HudField, HudFieldSpec, HudParser, HudReading, HudReadings, HudRegion, HudValue,
    Key, KeyCode, KeystrokeDynamics, KeystrokeNaturalParams, MouseButton, MouseTarget, Observation,
    ObservationDiagnostics, OcrBackend, OcrResult, OcrWord, PadButton, PadId, PerceptionMode,
    Point, Profile, ProfileBackends, ProfileCapture, ProfileCaptureTarget, ProfileDetection,
    ProfileId, ProfileMatch, ProfileOcr, ProfileUseScope, Rect, ReflexAimAxis, ReflexButtonTarget,
    ReflexId, ReflexKind, ReflexLifetime, ReflexRegistration, ReflexState, ReflexStatus,
    ReflexThen, SensorStatus, SessionId, Size, Stick, StoredAppContext, StoredAuditContext,
    StoredBackendPolicy, StoredEvent, StoredObservation, StoredProfileHistoryEntry,
    StoredRedaction, StoredReflexAudit, StoredReflexStep, StoredSession, SubscriptionId,
    SubsystemHealth, Trigger, UiaPattern, WindowEdge, element_id, entity_id, new_reflex_id,
    new_session_id, new_subscription_id,
};
pub use usb_identity::{
    SYNAPSE_PICO_HID_MANUFACTURER, SYNAPSE_PICO_HID_PRODUCT, SYNAPSE_PICO_HID_SERIAL_PREFIX,
    SYNAPSE_PICO_HID_USB_PID, SYNAPSE_PICO_HID_USB_VID,
};
