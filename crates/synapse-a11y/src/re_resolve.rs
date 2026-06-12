use serde::{Deserialize, Serialize};
use synapse_core::{ElementId, Rect};

use crate::{A11yResult, UIElement, platform};

/// Re-resolves a composite Synapse element id back to a live UIA element.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the runtime id cannot be found under the
/// HWND, `OBSERVE_INTERNAL` for invalid ids, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
    platform::re_resolve(id)
}

/// Result of preparing an element-target click entirely on the UIA worker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ElementClickAction {
    Invoked,
    Toggled {
        before_state: String,
        after_state: String,
    },
    Selected {
        was_selected: bool,
        is_selected: bool,
    },
    Expanded {
        before_state: ExpandState,
        after_state: ExpandState,
    },
    Collapsed {
        before_state: ExpandState,
        after_state: ExpandState,
    },
    LegacyDefaultAction {
        default_action: Option<String>,
    },
}

/// Readback from setting an element's native text/value on the UIA worker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ElementValueSetReadback {
    pub method: String,
    pub before_value: String,
    pub after_value: String,
    pub expected_after_value: Option<String>,
    pub is_password: bool,
    pub before_password_len: Option<usize>,
    pub after_password_len: Option<usize>,
}

/// Readback from an element's native text/value pattern without mutating it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ElementValueReadback {
    pub method: String,
    pub value: String,
    pub is_readonly: bool,
    pub is_password: bool,
    pub password_len: Option<usize>,
}

/// Live metadata read from a re-resolved element on the UIA worker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ElementMetadataReadback {
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub bbox: Rect,
    pub enabled: bool,
    pub keyboard_focusable: bool,
    pub patterns: Vec<synapse_core::UiaPattern>,
    pub value: Option<String>,
}

/// UIA scroll state read separately from the target element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ElementScrollStateReadback {
    pub bbox: Rect,
    pub horizontal_scroll_percent: Option<f64>,
    pub vertical_scroll_percent: Option<f64>,
    pub horizontal_view_size: Option<f64>,
    pub vertical_view_size: Option<f64>,
    pub horizontally_scrollable: Option<bool>,
    pub vertically_scrollable: Option<bool>,
}

/// Readback from scrolling an element through UIA control patterns.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ElementScrollReadback {
    pub method: String,
    pub before: ElementScrollStateReadback,
    pub after: ElementScrollStateReadback,
    pub requested_dy: i32,
    pub requested_dx: i32,
    pub scroll_call_count: u32,
}

/// Resolves an element and reads its current bounding rectangle as plain data.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA error for OS failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn element_bounding_rect(id: &ElementId) -> A11yResult<Rect> {
    platform::element_bounding_rect(id)
}

/// Attempts a semantic UIA click for an element using supported UIA control
/// patterns.
///
/// No coordinate fallback is synthesized here; unsupported elements return
/// `ACTION_ELEMENT_PATTERN_UNSUPPORTED` so the caller can decide the next
/// explicit delivery tier.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// typed unsupported-pattern error when no click-like UIA pattern is exposed, a
/// structured UIA error for pattern method failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn click_element_action(id: &ElementId) -> A11yResult<ElementClickAction> {
    platform::click_element_action(id)
}

/// Sets UIA focus on a re-resolved element without returning the COM element.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA error for focus failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn focus_element(id: &ElementId) -> A11yResult<()> {
    platform::focus_element(id)
}

/// Sets a re-resolved element's text/value and reads it back.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA/native-message error when the element exposes neither
/// writable `ValuePattern` nor a native edit HWND text-message route, and
/// `A11Y_NOT_AVAILABLE` on non-Windows.
pub fn set_element_value(id: &ElementId, value: &str) -> A11yResult<ElementValueSetReadback> {
    platform::set_element_value(id, value)
}

/// Reads a re-resolved element's current text/value.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA/native-message error when the element exposes neither
/// `ValuePattern` nor a native edit HWND text-message readback route, and
/// `A11Y_NOT_AVAILABLE` on non-Windows.
pub fn element_value(id: &ElementId) -> A11yResult<ElementValueReadback> {
    platform::element_value(id)
}

/// Reads live metadata for a re-resolved element without mutating it.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA error for OS failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows.
pub fn element_metadata(id: &ElementId) -> A11yResult<ElementMetadataReadback> {
    platform::element_metadata(id)
}

/// Scrolls a re-resolved element into its container's viewport through UIA
/// `ScrollItemPattern.ScrollIntoView` (#882). Composite tools call this before
/// a coordinate click so an off-viewport target's stale bbox cannot steer the
/// click into another window.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// typed unsupported-pattern error when `ScrollItemPattern` is not exposed, a
/// structured UIA error for pattern method failures, or `A11Y_NOT_AVAILABLE`
/// on non-Windows platforms.
pub fn scroll_element_into_view(id: &ElementId) -> A11yResult<()> {
    platform::scroll_element_into_view(id)
}

/// Scrolls a re-resolved element through UIA `ScrollPattern` or
/// `ScrollItemPattern` and returns before/after target readback.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// typed unsupported-pattern error when neither scroll pattern is available, a
/// structured UIA error for pattern method failures, or `A11Y_NOT_AVAILABLE`
/// on non-Windows platforms.
pub fn scroll_element(id: &ElementId, dy: i32, dx: i32) -> A11yResult<ElementScrollReadback> {
    platform::scroll_element(id, dy, dx)
}

/// Reads a re-resolved element's current UIA scroll state without mutating it.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA error for state read failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn element_scroll_state(id: &ElementId) -> A11yResult<ElementScrollStateReadback> {
    platform::element_scroll_state(id)
}

/// Read-only mirror of `uiautomation::types::ExpandCollapseState`. Kept
/// independent of the underlying crate so callers don't need a uiautomation
/// dependency just to compare against a literal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandState {
    Collapsed,
    Expanded,
    PartiallyExpanded,
    LeafNode,
}

/// Reads `ExpandCollapsePattern::CurrentExpandCollapseState` from the given
/// element.
///
/// Used by `act_click(use_invoke_pattern=true)` manual verification tests to
/// assert menu/expander state flipped after an invoke.
///
/// # Errors
///
/// Returns the same structured UIA errors as the other a11y accessors;
/// `A11Y_PATTERN_UNAVAILABLE`-class error when the element does not expose
/// `ExpandCollapsePattern`; `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn expand_state_of(element: &UIElement) -> A11yResult<ExpandState> {
    platform::expand_state_of(element)
}

/// Reads `ExpandCollapsePattern::CurrentExpandCollapseState` for a re-resolved
/// element without returning the COM element.
///
/// # Errors
///
/// Returns the same structured UIA errors as `expand_state_of`, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn expand_state_of_id(id: &ElementId) -> A11yResult<ExpandState> {
    platform::expand_state_of_id(id)
}
