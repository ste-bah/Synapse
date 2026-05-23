macro_rules! assert_literal {
    ($name:ident) => {
        assert_eq!(synapse_core::error_codes::$name, stringify!($name));
    };
}

#[test]
fn error_codes_match_literal_names() {
    assert_literal!(OBSERVE_NO_PERCEPTION_AVAILABLE);
    assert_literal!(OBSERVE_INTERNAL);
    assert_literal!(CAPTURE_GRAPHICS_API_UNSUPPORTED);
    assert_literal!(CAPTURE_TARGET_LOST);
    assert_literal!(CAPTURE_NO_DIRTY_REGIONS);
    assert_literal!(A11Y_NOT_AVAILABLE);
    assert_literal!(A11Y_ELEMENT_STALE);
    assert_literal!(A11Y_NO_FOREGROUND);
    assert_literal!(A11Y_CDP_UNREACHABLE);
    assert_literal!(DETECTION_MODEL_NOT_LOADED);
    assert_literal!(DETECTION_MODEL_INFER_FAILED);
    assert_literal!(DETECTION_NO_FRAME);
    assert_literal!(OCR_NO_TEXT);
    assert_literal!(OCR_BACKEND_UNAVAILABLE);
    assert_literal!(HUD_NO_ACTIVE_PROFILE);
    assert_literal!(HUD_FIELD_NOT_DEFINED);
    assert_literal!(HUD_EXTRACTION_FAILED);
    assert_literal!(AUDIO_DEVICE_LOST);
    assert_literal!(AUDIO_LOOPBACK_INIT_FAILED);
    assert_literal!(AUDIO_STT_MODEL_NOT_LOADED);

    assert_literal!(ACTION_QUEUE_FULL);
    assert_literal!(ACTION_RATE_LIMITED);
    assert_literal!(ACTION_BACKEND_UNAVAILABLE);
    assert_literal!(ACTION_TARGET_INVALID);
    assert_literal!(ACTION_HOLD_EXCEEDED_MAX);
    assert_literal!(ACTION_HID_PORT_DISCONNECTED);
    assert_literal!(ACTION_VIGEM_NOT_INSTALLED);
    assert_literal!(ACTION_VIGEM_PLUGIN_FAILED);
    assert_literal!(ACTION_ELEMENT_NOT_RESOLVED);
    assert_literal!(ACTION_FOREGROUND_LOST);
    assert_literal!(ACTION_UNSUPPORTED_KEY);
    assert_literal!(ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT);
    assert_literal!(STUCK_KEY_AUTO_RELEASED);
    assert_literal!(SAFETY_RELEASE_ALL_FIRED);
    assert_literal!(SAFETY_OPERATOR_HOTKEY_FIRED);

    assert_literal!(REFLEX_CAP_REACHED);
    assert_literal!(REFLEX_KIND_INVALID);
    assert_literal!(REFLEX_PARAMS_INVALID);
    assert_literal!(REFLEX_TARGET_INVALID);
    assert_literal!(REFLEX_FILTER_INVALID);
    assert_literal!(REFLEX_PRIORITY_INVALID);
    assert_literal!(REFLEX_TICK_LATE);
    assert_literal!(REFLEX_TRACK_LOST);
    assert_literal!(REFLEX_STARVED);
    assert_literal!(REFLEX_DISABLED_BY_OPERATOR);
    assert_literal!(REFLEX_LIFETIME_EXPIRED);

    assert_literal!(PROFILE_NOT_FOUND);
    assert_literal!(PROFILE_PARSE_ERROR);
    assert_literal!(PROFILE_VERSION_INCOMPATIBLE);
    assert_literal!(PROFILE_KEYMAP_INVALID);
    assert_literal!(PROFILE_HUD_REGION_INVALID);
    assert_literal!(CAPTURE_TARGET_INVALID);
    assert_literal!(PERCEPTION_MODE_INVALID);

    assert_literal!(SESSION_NOT_FOUND);
    assert_literal!(SESSION_EXPIRED);
    assert_literal!(SUBSCRIPTION_NOT_FOUND);
    assert_literal!(SUBSCRIPTION_CAP_REACHED);
    assert_literal!(TOOL_NOT_FOUND);
    assert_literal!(TOOL_PARAMS_INVALID);
    assert_literal!(TOOL_INTERNAL_ERROR);

    assert_literal!(STORAGE_OPEN_FAILED);
    assert_literal!(STORAGE_WRITE_FAILED);
    assert_literal!(STORAGE_READ_FAILED);
    assert_literal!(STORAGE_CORRUPTED);
    assert_literal!(STORAGE_SCHEMA_MISMATCH);

    assert_literal!(MODEL_DOWNLOAD_FAILED);
    assert_literal!(MODEL_HASH_MISMATCH);
    assert_literal!(MODEL_LOAD_FAILED);
    assert_literal!(MODEL_BACKEND_UNAVAILABLE);

    assert_literal!(HID_PORT_NOT_FOUND);
    assert_literal!(HID_PORT_OPEN_FAILED);
    assert_literal!(HID_PROTOCOL_HANDSHAKE_FAILED);
    assert_literal!(HID_FIRMWARE_VERSION_MISMATCH);
    assert_literal!(HID_COMMAND_REJECTED);
    assert_literal!(HID_LINK_TIMEOUT);

    assert_literal!(SAFETY_KILLSWITCH_ACTIVE);
    assert_literal!(SAFETY_PROCESS_DENYLISTED);
    assert_literal!(SAFETY_SHELL_DENIED_BY_POLICY);
    assert_literal!(SAFETY_LAUNCH_DENIED_BY_POLICY);
    assert_literal!(SAFETY_SECRET_REDACTED);
}
