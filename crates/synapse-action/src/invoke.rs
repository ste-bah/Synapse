#[cfg(any(test, windows))]
use std::fmt::Display;

use synapse_core::ElementId;

use crate::{ActionError, ActionResult};

#[cfg(windows)]
use synapse_a11y::uiautomation::patterns::UIInvokePattern;

/// Re-resolves a Synapse accessibility element and invokes its UIA
/// `InvokePattern` without moving the cursor.
///
/// # Errors
///
/// On Windows, `synapse_a11y::re_resolve` failures are mapped to
/// `ACTION_ELEMENT_NOT_RESOLVED`. Missing or failing `InvokePattern` calls are
/// reported as `ACTION_TARGET_INVALID` so the higher-level click path can fall
/// through to coordinate click handling.
#[cfg(windows)]
pub fn invoke_element(element_id: &ElementId) -> ActionResult<()> {
    let element = synapse_a11y::re_resolve(element_id).map_err(element_not_resolved)?;
    let pattern: UIInvokePattern = element
        .get_pattern()
        .map_err(|err| invoke_pattern_unavailable(element_id, err))?;

    pattern
        .invoke()
        .map_err(|err| invoke_pattern_failed(element_id, err))
}

/// Non-Windows builds expose the same API but fail closed before any action is
/// attempted.
///
/// # Errors
///
/// Always returns `ACTION_BACKEND_UNAVAILABLE` because UI Automation
/// `InvokePattern` dispatch is Windows-only.
#[cfg(not(windows))]
pub fn invoke_element(element_id: &ElementId) -> ActionResult<()> {
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation InvokePattern requires Windows for element {element_id}"),
    })
}

#[must_use]
#[cfg(any(test, windows))]
fn element_not_resolved(error: impl Display) -> ActionError {
    ActionError::ElementNotResolved {
        detail: error.to_string(),
    }
}

#[must_use]
#[cfg(any(test, windows))]
fn invoke_pattern_unavailable(element_id: &ElementId, error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("element {element_id} does not expose InvokePattern: {error}"),
    }
}

#[must_use]
#[cfg(any(test, windows))]
fn invoke_pattern_failed(element_id: &ElementId, error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("InvokePattern.invoke failed for element {element_id}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use synapse_core::ElementId;

    #[cfg(not(windows))]
    use super::invoke_element;
    use super::{element_not_resolved, invoke_pattern_failed, invoke_pattern_unavailable};
    #[cfg(not(windows))]
    use crate::ActionError;

    #[test]
    fn re_resolve_failures_map_to_element_not_resolved() {
        let before = "synthetic stale element";
        let after = element_not_resolved(before);
        assert_eq!(
            after.code(),
            synapse_core::error_codes::ACTION_ELEMENT_NOT_RESOLVED
        );
        assert_eq!(after.detail(), before);
        println!(
            "source_of_truth=invoke_error_mapping edge=re_resolve_failure before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[test]
    fn missing_invoke_pattern_maps_to_target_invalid_for_coordinate_fallback() {
        let element_id = synthetic_element_id();
        let before = "pattern not available";
        let after = invoke_pattern_unavailable(&element_id, before);
        assert_eq!(
            after.code(),
            synapse_core::error_codes::ACTION_TARGET_INVALID
        );
        assert!(after.detail().contains(element_id.as_str()));
        assert!(after.detail().contains("InvokePattern"));
        println!(
            "source_of_truth=invoke_error_mapping edge=missing_invoke_pattern before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[test]
    fn invoke_failures_map_to_target_invalid_without_cursor_fallback_in_bridge() {
        let element_id = synthetic_element_id();
        let before = "blocked by modal";
        let after = invoke_pattern_failed(&element_id, before);
        assert_eq!(
            after.code(),
            synapse_core::error_codes::ACTION_TARGET_INVALID
        );
        assert!(after.detail().contains(element_id.as_str()));
        assert!(after.detail().contains("InvokePattern.invoke failed"));
        println!(
            "source_of_truth=invoke_error_mapping edge=invoke_failure before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_stub_fails_closed() {
        let element_id = synthetic_element_id();
        let before = format!("os={} element_id={element_id}", std::env::consts::OS);
        let after = invoke_element(&element_id);
        let Err(ActionError::BackendUnavailable { detail }) = after else {
            panic!("expected non-Windows invoke_element to fail closed");
        };
        assert_eq!(
            ActionError::BackendUnavailable {
                detail: detail.clone()
            }
            .code(),
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert!(detail.contains("requires Windows"));
        println!(
            "source_of_truth=invoke_non_windows_stub edge=non_windows before={before:?} after_code={} after_detail={detail:?}",
            synapse_core::error_codes::ACTION_BACKEND_UNAVAILABLE
        );
    }

    fn synthetic_element_id() -> ElementId {
        match ElementId::parse("0x1234:0000002a00000001") {
            Ok(element_id) => element_id,
            Err(error) => panic!("synthetic element id must parse: {error}"),
        }
    }
}
