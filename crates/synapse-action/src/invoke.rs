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
    CoordinateFallback(CoordinateFallbackPlan),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CoordinateFallbackPlan {
    pub screen_point: Point,
    pub window_point: Point,
}

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
    let element = resolver::resolve_element(element_id)?;
    dispatch::invoke_resolved_element(element_id, &element)
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

/// Attempts semantic UIA invoke first, then falls back to a coordinate click at
/// the resolved element's bounding-rectangle center when `InvokePattern` is not
/// available.
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
    let element = resolver::resolve_element(element_id)?;

    dispatch::complete_click_attempt(
        dispatch::try_invoke_resolved_element(element_id, &element),
        || resolver::coordinate_fallback_plan(element_id, &element),
        backend,
        state,
        button,
    )
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
pub(crate) fn element_screen_point(element_id: &ElementId) -> ActionResult<Point> {
    let element = resolver::resolve_element(element_id)?;
    resolver::coordinate_fallback_plan(element_id, &element).map(|plan| plan.screen_point)
}

#[cfg(not(windows))]
pub(crate) fn element_screen_point(element_id: &ElementId) -> ActionResult<Point> {
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation element target requires Windows for element {element_id}"),
    })
}
