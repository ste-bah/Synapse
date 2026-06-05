use synapse_core::{ElementId, MouseButton, Point};

#[cfg(not(windows))]
use crate::ActionError;
use crate::{ActionBackend, ActionResult, EmitState};

#[cfg(any(test, windows))]
mod dispatch;
#[cfg(any(test, windows))]
mod resolver;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ElementClickOutcome {
    Invoked,
    Toggled,
    CoordinateFallback(CoordinateFallbackPlan),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CoordinateFallbackPlan {
    pub screen_point: Point,
    pub window_point: Point,
}

/// Re-resolves a Synapse accessibility element and dispatches a semantic UIA
/// click (`InvokePattern` or `TogglePattern`) without moving the cursor.
///
/// # Errors
///
/// On Windows, `synapse_a11y::re_resolve` failures are mapped to
/// `ACTION_ELEMENT_NOT_RESOLVED`. Missing semantic patterns are reported as
/// `ACTION_TARGET_INVALID` so the higher-level click path can fall through to
/// coordinate click handling.
#[cfg(windows)]
pub fn invoke_element(element_id: &ElementId) -> ActionResult<()> {
    match synapse_a11y::click_element_action(element_id).map_err(a11y_error_to_action)? {
        synapse_a11y::ElementClickAction::Invoked | synapse_a11y::ElementClickAction::Toggled => {
            Ok(())
        }
        synapse_a11y::ElementClickAction::CoordinateFallback { .. } => Err(
            resolver::invoke_pattern_unavailable(element_id, "pattern not available"),
        ),
    }
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

/// Attempts a semantic UIA click first, then falls back to a coordinate click at
/// the resolved element's bounding-rectangle center when no supported semantic
/// pattern is available.
///
/// # Errors
///
/// Returns `ACTION_ELEMENT_NOT_RESOLVED` when UIA re-resolution fails,
/// `ACTION_TARGET_INVALID` when the element cannot produce a usable click
/// target, or a backend-specific action error if the coordinate click cannot be
/// emitted.
#[cfg(windows)]
pub fn click_element_or_fallback<B>(
    element_id: &ElementId,
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
{
    match synapse_a11y::click_element_action(element_id).map_err(a11y_error_to_action)? {
        synapse_a11y::ElementClickAction::Invoked => Ok(ElementClickOutcome::Invoked),
        synapse_a11y::ElementClickAction::Toggled => Ok(ElementClickOutcome::Toggled),
        synapse_a11y::ElementClickAction::CoordinateFallback { bbox } => {
            let plan = resolver::coordinate_fallback_plan(element_id, bbox)?;
            dispatch::emit_coordinate_fallback_click(backend, state, button, plan)?;
            Ok(ElementClickOutcome::CoordinateFallback(plan))
        }
    }
}

/// Non-Windows builds expose the same API but fail closed before any action is
/// attempted.
///
/// # Errors
///
/// Always returns `ACTION_BACKEND_UNAVAILABLE` because UI Automation
/// `InvokePattern` dispatch and bounding-rectangle fallback are Windows-only.
#[cfg(not(windows))]
pub fn click_element_or_fallback<B>(
    element_id: &ElementId,
    _backend: &B,
    _state: &mut EmitState,
    _button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
{
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation element click requires Windows for element {element_id}"),
    })
}

#[cfg(windows)]
#[expect(
    dead_code,
    reason = "reserved for element-target action paths that need screen-point readback"
)]
pub(crate) fn element_screen_point(element_id: &ElementId) -> ActionResult<Point> {
    let rect = synapse_a11y::element_bounding_rect(element_id).map_err(a11y_error_to_action)?;
    resolver::coordinate_fallback_plan(element_id, rect).map(|plan| plan.screen_point)
}

#[cfg(not(windows))]
#[expect(
    dead_code,
    reason = "reserved for element-target action paths that need screen-point readback"
)]
pub(crate) fn element_screen_point(element_id: &ElementId) -> ActionResult<Point> {
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation element target requires Windows for element {element_id}"),
    })
}

#[cfg(windows)]
fn a11y_error_to_action(error: synapse_a11y::A11yError) -> crate::ActionError {
    match error {
        synapse_a11y::A11yError::ElementStale { .. }
        | synapse_a11y::A11yError::InvalidElementId { .. }
        | synapse_a11y::A11yError::NoForeground { .. } => resolver::element_not_resolved(error),
        synapse_a11y::A11yError::NotAvailable { detail } => {
            crate::ActionError::BackendUnavailable { detail }
        }
        synapse_a11y::A11yError::CdpUnreachable { .. }
        | synapse_a11y::A11yError::CdpAttachFailed { .. }
        | synapse_a11y::A11yError::CdpAxtreeFailed { .. }
        | synapse_a11y::A11yError::Internal { .. } => resolver::target_invalid(error),
    }
}
