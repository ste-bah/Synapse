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

/// Sets a re-resolved element's native `ValuePattern` text and reads it back.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the element id cannot be re-resolved, a
/// structured UIA error when the element does not expose writable `ValuePattern`
/// or SetValue/readback fails, and `A11Y_NOT_AVAILABLE` on non-Windows.
pub fn set_element_value(id: &ElementId, value: &str) -> A11yResult<ElementValueSetReadback> {
    platform::set_element_value(id, value)
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
