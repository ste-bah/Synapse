use std::{
    ffi::c_void,
    sync::{Mutex, OnceLock},
};

use synapse_core::{Rect, UiaPattern};
use uiautomation::{
    UIAutomation, UIElement,
    core::UICacheRequest,
    types::{ControlType, ElementMode, TreeScope, UIProperty},
    variants::{Value, Variant},
};
use windows::Win32::Foundation::HWND;

use crate::{A11yError, A11yResult, ElementSearchScope};

static UIA_CLIENT: OnceLock<ProcessUiaClient> = OnceLock::new();

struct ProcessUiaClient {
    automation: Mutex<UIAutomation>,
}

#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl Send for ProcessUiaClient {}
unsafe impl Sync for ProcessUiaClient {}

fn client() -> A11yResult<&'static ProcessUiaClient> {
    if let Some(client) = UIA_CLIENT.get() {
        return Ok(client);
    }

    let automation = UIAutomation::new()
        .or_else(|_err| UIAutomation::new_direct())
        .map_err(map_uia_error)?;
    UIA_CLIENT
        .set(ProcessUiaClient {
            automation: Mutex::new(automation),
        })
        .map_err(|_client| A11yError::internal("UIA client was initialized concurrently"))?;
    UIA_CLIENT
        .get()
        .ok_or_else(|| A11yError::internal("UIA client missing after initialization"))
}

pub(super) fn with_automation<T>(
    action: impl FnOnce(&UIAutomation) -> A11yResult<T>,
) -> A11yResult<T> {
    let guard = client()?
        .automation
        .lock()
        .map_err(|err| A11yError::internal(err.to_string()))?;
    action(&guard)
}

pub(super) fn create_cache_request(
    automation: &UIAutomation,
    depth: u32,
    element_mode: ElementMode,
    tree_view: TreeView,
) -> A11yResult<UICacheRequest> {
    let cache = automation.create_cache_request().map_err(map_uia_error)?;
    for property in [
        UIProperty::RuntimeId,
        UIProperty::BoundingRectangle,
        UIProperty::ProcessId,
        UIProperty::ControlType,
        UIProperty::LocalizedControlType,
        UIProperty::Name,
        UIProperty::HasKeyboardFocus,
        UIProperty::IsEnabled,
        UIProperty::AutomationId,
        UIProperty::ClassName,
        UIProperty::NativeWindowHandle,
        UIProperty::IsInvokePatternAvailable,
        UIProperty::IsTogglePatternAvailable,
        UIProperty::IsValuePatternAvailable,
        UIProperty::IsSelectionPatternAvailable,
        UIProperty::IsExpandCollapsePatternAvailable,
        UIProperty::IsScrollPatternAvailable,
        UIProperty::IsTextPatternAvailable,
        UIProperty::IsWindowPatternAvailable,
        UIProperty::IsTransformPatternAvailable,
        UIProperty::IsRangeValuePatternAvailable,
        UIProperty::ValueValue,
        UIProperty::RangeValueValue,
    ] {
        cache.add_property(property).map_err(map_uia_error)?;
    }
    let tree_filter = match tree_view {
        TreeView::Control => automation.get_control_view_condition(),
        TreeView::Raw => automation.create_true_condition(),
    }
    .map_err(map_uia_error)?;
    cache.set_tree_filter(tree_filter).map_err(map_uia_error)?;
    let scope = if depth == 0 {
        TreeScope::Element
    } else {
        TreeScope::Subtree
    };
    cache.set_tree_scope(scope).map_err(map_uia_error)?;
    cache
        .set_element_mode(element_mode)
        .map_err(map_uia_error)?;
    Ok(cache)
}
pub(super) fn cached_patterns(element: &UIElement) -> Vec<UiaPattern> {
    let mut patterns = Vec::new();
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsInvokePatternAvailable,
        UiaPattern::Invoke,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsTogglePatternAvailable,
        UiaPattern::Toggle,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsValuePatternAvailable,
        UiaPattern::Value,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsSelectionPatternAvailable,
        UiaPattern::Selection,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsExpandCollapsePatternAvailable,
        UiaPattern::ExpandCollapse,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsScrollPatternAvailable,
        UiaPattern::Scroll,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsTextPatternAvailable,
        UiaPattern::Text,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsWindowPatternAvailable,
        UiaPattern::Window,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsTransformPatternAvailable,
        UiaPattern::Transform,
    );
    push_pattern(
        element,
        &mut patterns,
        UIProperty::IsRangeValuePatternAvailable,
        UiaPattern::RangeValue,
    );
    patterns
}

fn push_pattern(
    element: &UIElement,
    patterns: &mut Vec<UiaPattern>,
    property: UIProperty,
    pattern: UiaPattern,
) {
    if cached_bool(element, property) {
        patterns.push(pattern);
    }
}

fn cached_bool(element: &UIElement, property: UIProperty) -> bool {
    element
        .get_cached_property_value(property)
        .ok()
        .and_then(|variant| <&Variant as TryInto<bool>>::try_into(&variant).ok())
        .unwrap_or(false)
}

/// On-screen value source-of-truth: prefer the `ValuePattern` string, then the
/// `RangeValuePattern` number. Returns `None` when neither pattern is exposed or
/// the value is empty, so `AccessibleNode.value` stays absent rather than blank.
pub(super) fn cached_value(element: &UIElement) -> Option<String> {
    if cached_bool(element, UIProperty::IsValuePatternAvailable)
        && let Ok(variant) = element.get_cached_property_value(UIProperty::ValueValue)
        && let Ok(text) = variant.get_string()
        && !text.is_empty()
    {
        return Some(text);
    }
    if cached_bool(element, UIProperty::IsRangeValuePatternAvailable)
        && let Ok(variant) = element.get_cached_property_value(UIProperty::RangeValueValue)
        && let Ok(Value::R8(number)) = variant.get_value()
    {
        return Some(format_range_value(number));
    }
    None
}

fn format_range_value(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        format!("{number}")
    }
}

pub(super) fn cached_rect(element: &UIElement) -> Rect {
    element.get_cached_bounding_rectangle().map_or(
        Rect {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        },
        |rect| Rect {
            x: rect.get_left(),
            y: rect.get_top(),
            w: rect.get_right().saturating_sub(rect.get_left()),
            h: rect.get_bottom().saturating_sub(rect.get_top()),
        },
    )
}

pub(super) fn cached_role(element: &UIElement) -> String {
    let localized = element
        .get_cached_localized_control_type()
        .unwrap_or_default();
    if !localized.is_empty() {
        return localized;
    }

    element
        .get_cached_control_type()
        .map_or_else(|_err| "unknown".to_owned(), control_type_name)
}

fn control_type_name(control_type: ControlType) -> String {
    format!("{control_type:?}")
}

pub(super) fn cached_hwnd(element: &UIElement) -> Option<i64> {
    let handle = element.get_cached_native_window_handle().ok()?;
    let raw: isize = handle.into();
    Some(raw as i64)
}

pub(super) fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

pub(super) const fn pattern_property(pattern: UiaPattern) -> UIProperty {
    match pattern {
        UiaPattern::Invoke => UIProperty::IsInvokePatternAvailable,
        UiaPattern::Toggle => UIProperty::IsTogglePatternAvailable,
        UiaPattern::Value => UIProperty::IsValuePatternAvailable,
        UiaPattern::Selection => UIProperty::IsSelectionPatternAvailable,
        UiaPattern::ExpandCollapse => UIProperty::IsExpandCollapsePatternAvailable,
        UiaPattern::Scroll => UIProperty::IsScrollPatternAvailable,
        UiaPattern::Text => UIProperty::IsTextPatternAvailable,
        UiaPattern::Window => UIProperty::IsWindowPatternAvailable,
        UiaPattern::Transform => UIProperty::IsTransformPatternAvailable,
        UiaPattern::RangeValue => UIProperty::IsRangeValuePatternAvailable,
    }
}

impl From<ElementSearchScope> for TreeScope {
    fn from(scope: ElementSearchScope) -> Self {
        match scope {
            ElementSearchScope::Children => Self::Children,
            ElementSearchScope::Descendants => Self::Descendants,
            ElementSearchScope::Subtree => Self::Subtree,
        }
    }
}
#[derive(Clone, Copy)]
pub(super) enum TreeView {
    Control,
    Raw,
}

pub(super) fn cached_runtime_id(element: &UIElement) -> A11yResult<Vec<i32>> {
    let value = element
        .get_cached_property_value(UIProperty::RuntimeId)
        .map_err(map_uia_error)?;
    match value.get_value().map_err(map_uia_error)? {
        Value::ArrayI4(items) => Ok(items),
        Value::SAFEARRAY(array) => array.try_into().map_err(map_uia_error),
        other => Err(A11yError::internal(format!(
            "cached RuntimeId had unexpected type {other}"
        ))),
    }
}
#[allow(clippy::missing_const_for_fn)]
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_uia_error(err: uiautomation::Error) -> A11yError {
    A11yError::internal(err.to_string())
}

#[allow(clippy::missing_const_for_fn)]
fn _hwnd_from_i64(hwnd: i64) -> HWND {
    HWND(hwnd as *mut c_void)
}
