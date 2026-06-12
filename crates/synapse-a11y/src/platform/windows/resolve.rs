use std::ffi::c_void;

use synapse_core::{ElementId, Rect};
use uiautomation::{
    UIAutomation, UIElement,
    patterns::{
        UIExpandCollapsePattern, UIInvokePattern, UILegacyIAccessiblePattern, UIScrollItemPattern,
        UIScrollPattern, UISelectionItemPattern, UITogglePattern, UIValuePattern,
    },
    types::{
        ElementMode, ExpandCollapseState, Handle, Rect as UiaRect, ScrollAmount, TreeScope,
        UIProperty,
    },
};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        ES_MULTILINE, GWL_STYLE, GetWindowLongW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETTEXT,
        WM_GETTEXTLENGTH, WM_SETTEXT,
    },
};

use crate::{
    A11yError, A11yResult, ElementClickAction, ElementMetadataReadback, ElementScrollReadback,
    ElementScrollStateReadback, ElementValueReadback, ElementValueSetReadback, ExpandState,
};

use super::common::{
    TreeView, cached_bool, cached_hwnd, cached_patterns, cached_role,
    cached_runtime_id_hex_or_fallback, cached_value, create_cache_request, map_uia_error,
    with_automation,
};

const RE_RESOLVE_NODE_BUDGET: usize = 20_000;
const ES_READONLY_STYLE: i32 = 0x0800;
const PASSWORD_LENGTH_READ_TIMEOUT_MS: u32 = 500;
const NATIVE_TEXT_MESSAGE_TIMEOUT_MS: u32 = 500;
const SUPPORTED_CLICK_PATTERNS: [&str; 5] = [
    "InvokePattern",
    "TogglePattern",
    "SelectionItemPattern",
    "ExpandCollapsePattern",
    "LegacyIAccessiblePattern.DoDefaultAction",
];
const SUPPORTED_SCROLL_PATTERNS: [&str; 2] = ["ScrollPattern", "ScrollItemPattern"];
const MAX_UIA_SCROLL_PATTERN_CALLS: u32 = 1024;

#[derive(Clone, Copy, Debug)]
enum ClickPatternKind {
    Invoke,
    Toggle,
    SelectionItem,
    ExpandCollapse,
    LegacyDefaultAction,
}

pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
    let _ = id;
    Err(A11yError::internal(
        "direct UIElement re-resolution is disabled; use data-returning worker APIs so UIA stays on the dedicated MTA worker",
    ))
}

pub(super) fn re_resolve_on_worker(
    automation: &uiautomation::UIAutomation,
    id: &ElementId,
) -> A11yResult<UIElement> {
    let parts = id.parts().map_err(|err| A11yError::InvalidElementId {
        detail: err.to_string(),
    })?;
    let hwnd = isize::try_from(parts.hwnd).map_err(|err| A11yError::InvalidElementId {
        detail: err.to_string(),
    })?;
    if let Some(found) = find_by_runtime_id_hex(
        automation,
        hwnd,
        &parts.runtime_id_hex,
        parts.hwnd,
        TreeView::Control,
    )? {
        return Ok(found);
    }

    find_by_runtime_id_hex(
        automation,
        hwnd,
        &parts.runtime_id_hex,
        parts.hwnd,
        TreeView::Raw,
    )?
    .ok_or_else(|| A11yError::ElementStale {
        detail: format!(
            "element id {id} was not found under hwnd 0x{:x} in control or raw view",
            parts.hwnd
        ),
    })
}

pub fn element_bounding_rect(id: &ElementId) -> A11yResult<Rect> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        element_rect(&element)
    })
}

pub fn click_element_action(id: &ElementId) -> A11yResult<ElementClickAction> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        let mut unsupported_reasons = Vec::new();
        for pattern in click_pattern_order(&element) {
            let action = match pattern {
                ClickPatternKind::Invoke => {
                    try_invoke_pattern(&id, &element, &mut unsupported_reasons)?
                }
                ClickPatternKind::Toggle => {
                    try_toggle_pattern(&id, &element, &mut unsupported_reasons)?
                }
                ClickPatternKind::SelectionItem => {
                    try_selection_item_pattern(&id, &element, &mut unsupported_reasons)?
                }
                ClickPatternKind::ExpandCollapse => {
                    try_expand_collapse_pattern(&id, &element, &mut unsupported_reasons)?
                }
                ClickPatternKind::LegacyDefaultAction => {
                    try_legacy_default_action_pattern(&id, &element, &mut unsupported_reasons)?
                }
            };
            if let Some(action) = action {
                return Ok(action);
            }
        }

        Err(unsupported_click_pattern(
            &id,
            &element,
            &unsupported_reasons,
        ))
    })
}

fn click_pattern_order(element: &UIElement) -> [ClickPatternKind; 5] {
    use ClickPatternKind::{ExpandCollapse, Invoke, LegacyDefaultAction, SelectionItem, Toggle};

    let role = cached_role(element).to_ascii_lowercase();
    if role.contains("check") || role.contains("toggle") {
        return [
            Toggle,
            Invoke,
            SelectionItem,
            ExpandCollapse,
            LegacyDefaultAction,
        ];
    }
    if role.contains("list item") || role.contains("radio") || role.contains("tab item") {
        return [
            SelectionItem,
            Invoke,
            Toggle,
            ExpandCollapse,
            LegacyDefaultAction,
        ];
    }
    if role.contains("combo")
        || role.contains("menu item")
        || role.contains("tree item")
        || role.contains("split button")
    {
        return [
            ExpandCollapse,
            Invoke,
            Toggle,
            SelectionItem,
            LegacyDefaultAction,
        ];
    }
    [
        Invoke,
        Toggle,
        SelectionItem,
        ExpandCollapse,
        LegacyDefaultAction,
    ]
}

fn try_invoke_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let invoke_pattern: Result<UIInvokePattern, _> = element.get_pattern();
    match invoke_pattern {
        Ok(invoke_pattern) => {
            invoke_pattern
                .invoke()
                .map_err(|err| pattern_operation_error(id, "InvokePattern", "invoke", &err))?;
            Ok(Some(ElementClickAction::Invoked))
        }
        Err(err) => {
            unsupported_reasons.push(format!("InvokePattern not exposed: {err}"));
            Ok(None)
        }
    }
}

fn try_toggle_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let toggle_pattern: Result<UITogglePattern, _> = element.get_pattern();
    match toggle_pattern {
        Ok(toggle_pattern) => toggle_pattern_action(id, &toggle_pattern).map(Some),
        Err(err) => {
            unsupported_reasons.push(format!("TogglePattern not exposed: {err}"));
            Ok(None)
        }
    }
}

fn toggle_pattern_action(
    id: &ElementId,
    toggle_pattern: &UITogglePattern,
) -> A11yResult<ElementClickAction> {
    let before_state = toggle_state_label(toggle_pattern.get_toggle_state().map_err(|err| {
        pattern_operation_error(id, "TogglePattern", "ToggleState before Toggle", &err)
    })?);
    toggle_pattern
        .toggle()
        .map_err(|err| pattern_operation_error(id, "TogglePattern", "toggle", &err))?;
    let after_state = toggle_state_label(toggle_pattern.get_toggle_state().map_err(|err| {
        pattern_operation_error(id, "TogglePattern", "ToggleState after Toggle", &err)
    })?);
    if before_state == after_state {
        return Err(A11yError::internal(format!(
            "TogglePattern.toggle returned for element {id}, but ToggleState stayed {after_state}"
        )));
    }
    Ok(ElementClickAction::Toggled {
        before_state,
        after_state,
    })
}

fn try_selection_item_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let selection_item_pattern: Result<UISelectionItemPattern, _> = element.get_pattern();
    match selection_item_pattern {
        Ok(selection_item_pattern) => {
            selection_item_pattern_action(id, &selection_item_pattern).map(Some)
        }
        Err(err) => {
            unsupported_reasons.push(format!("SelectionItemPattern not exposed: {err}"));
            Ok(None)
        }
    }
}

fn selection_item_pattern_action(
    id: &ElementId,
    selection_item_pattern: &UISelectionItemPattern,
) -> A11yResult<ElementClickAction> {
    let was_selected = selection_item_pattern.is_selected().map_err(|err| {
        pattern_operation_error(id, "SelectionItemPattern", "IsSelected before Select", &err)
    })?;
    selection_item_pattern
        .select()
        .map_err(|err| pattern_operation_error(id, "SelectionItemPattern", "select", &err))?;
    let is_selected = selection_item_pattern.is_selected().map_err(|err| {
        pattern_operation_error(id, "SelectionItemPattern", "IsSelected after Select", &err)
    })?;
    if !is_selected {
        return Err(A11yError::internal(format!(
            "SelectionItemPattern.select returned for element {id}, but IsSelected stayed false"
        )));
    }
    Ok(ElementClickAction::Selected {
        was_selected,
        is_selected,
    })
}

fn try_expand_collapse_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let expand_collapse_pattern: Result<UIExpandCollapsePattern, _> = element.get_pattern();
    match expand_collapse_pattern {
        Ok(expand_collapse_pattern) => {
            expand_collapse_pattern_action(id, &expand_collapse_pattern, unsupported_reasons)
        }
        Err(err) => {
            unsupported_reasons.push(format!("ExpandCollapsePattern not exposed: {err}"));
            Ok(None)
        }
    }
}

fn expand_collapse_pattern_action(
    id: &ElementId,
    expand_collapse_pattern: &UIExpandCollapsePattern,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let before_state =
        expand_state_from_uia(expand_collapse_pattern.get_state().map_err(|err| {
            pattern_operation_error(
                id,
                "ExpandCollapsePattern",
                "ExpandCollapseState before action",
                &err,
            )
        })?);
    match before_state {
        ExpandState::Collapsed => expand_pattern_action(id, expand_collapse_pattern, before_state),
        ExpandState::Expanded | ExpandState::PartiallyExpanded => {
            collapse_pattern_action(id, expand_collapse_pattern, before_state)
        }
        ExpandState::LeafNode => {
            unsupported_reasons
                .push("ExpandCollapsePattern exposed but current state is LeafNode".to_owned());
            Ok(None)
        }
    }
}

fn expand_pattern_action(
    id: &ElementId,
    expand_collapse_pattern: &UIExpandCollapsePattern,
    before_state: ExpandState,
) -> A11yResult<Option<ElementClickAction>> {
    expand_collapse_pattern
        .expand()
        .map_err(|err| pattern_operation_error(id, "ExpandCollapsePattern", "expand", &err))?;
    let after_state =
        expand_state_from_uia(expand_collapse_pattern.get_state().map_err(|err| {
            pattern_operation_error(
                id,
                "ExpandCollapsePattern",
                "ExpandCollapseState after expand",
                &err,
            )
        })?);
    if after_state == before_state {
        return Err(A11yError::internal(format!(
            "ExpandCollapsePattern.expand returned for element {id}, but state stayed {after_state:?}"
        )));
    }
    Ok(Some(ElementClickAction::Expanded {
        before_state,
        after_state,
    }))
}

fn collapse_pattern_action(
    id: &ElementId,
    expand_collapse_pattern: &UIExpandCollapsePattern,
    before_state: ExpandState,
) -> A11yResult<Option<ElementClickAction>> {
    expand_collapse_pattern
        .collapse()
        .map_err(|err| pattern_operation_error(id, "ExpandCollapsePattern", "collapse", &err))?;
    let after_state =
        expand_state_from_uia(expand_collapse_pattern.get_state().map_err(|err| {
            pattern_operation_error(
                id,
                "ExpandCollapsePattern",
                "ExpandCollapseState after collapse",
                &err,
            )
        })?);
    if after_state == before_state {
        return Err(A11yError::internal(format!(
            "ExpandCollapsePattern.collapse returned for element {id}, but state stayed {after_state:?}"
        )));
    }
    Ok(Some(ElementClickAction::Collapsed {
        before_state,
        after_state,
    }))
}

fn try_legacy_default_action_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let legacy_pattern: Result<UILegacyIAccessiblePattern, _> = element.get_pattern();
    match legacy_pattern {
        Ok(legacy_pattern) => legacy_default_action(id, &legacy_pattern, unsupported_reasons),
        Err(err) => {
            unsupported_reasons.push(format!("LegacyIAccessiblePattern not exposed: {err}"));
            Ok(None)
        }
    }
}

fn legacy_default_action(
    id: &ElementId,
    legacy_pattern: &UILegacyIAccessiblePattern,
    unsupported_reasons: &mut Vec<String>,
) -> A11yResult<Option<ElementClickAction>> {
    let default_action = match legacy_pattern.get_default_action() {
        Ok(value) if !value.trim().is_empty() => value,
        Ok(_) => {
            unsupported_reasons
                .push("LegacyIAccessiblePattern exposed but DefaultAction is empty".to_owned());
            return Ok(None);
        }
        Err(err) => {
            if stale_provider_error(&err.to_string()) {
                return Err(pattern_operation_error(
                    id,
                    "LegacyIAccessiblePattern",
                    "DefaultAction read",
                    &err,
                ));
            }
            unsupported_reasons.push(format!(
                "LegacyIAccessiblePattern DefaultAction read failed: {err}"
            ));
            return Ok(None);
        }
    };
    legacy_pattern.do_default_action().map_err(|err| {
        pattern_operation_error(id, "LegacyIAccessiblePattern", "DoDefaultAction", &err)
    })?;
    Ok(Some(ElementClickAction::LegacyDefaultAction {
        default_action: Some(default_action),
    }))
}

pub fn focus_element(id: &ElementId) -> A11yResult<()> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        element.set_focus().map_err(map_uia_error)
    })
}

pub fn set_element_value(id: &ElementId, value: &str) -> A11yResult<ElementValueSetReadback> {
    let id = id.clone();
    let value = value.to_owned();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        if !cached_bool(&element, UIProperty::IsEnabled) {
            return Err(A11yError::ElementNotEnabled {
                detail: format!("element {id} IsEnabled=false before ValuePattern.SetValue"),
            });
        }
        let is_password = cached_bool(&element, UIProperty::IsPassword);
        if let Some(hwnd) = native_text_hwnd(&element)? {
            if native_is_readonly(hwnd) {
                return Err(A11yError::ElementValueReadOnly {
                    detail: format!("native edit HWND is read-only for element {id}"),
                });
            }
            return set_native_text_value(&id, hwnd, &value, is_password);
        }
        let pattern: UIValuePattern = match element.get_pattern() {
            Ok(pattern) => pattern,
            Err(error) => {
                return set_element_value_via_native_text(
                    &id,
                    &element,
                    &value,
                    is_password,
                    &error.to_string(),
                );
            }
        };
        if pattern.is_readonly().map_err(map_uia_error)? {
            return Err(A11yError::ElementValueReadOnly {
                detail: format!("ValuePattern is read-only for element {id}"),
            });
        }

        let before_password_len = if is_password {
            Some(password_text_len(&id, &element)?)
        } else {
            None
        };
        let before_value = if is_password {
            String::new()
        } else {
            pattern.get_value().map_err(map_uia_error)?
        };
        pattern.set_value(&value).map_err(map_uia_error)?;
        let after_password_len = if is_password {
            Some(password_text_len(&id, &element)?)
        } else {
            None
        };
        let after_value = if is_password {
            String::new()
        } else {
            pattern.get_value().map_err(map_uia_error)?
        };
        Ok(ElementValueSetReadback {
            method: "uia_value_pattern".to_owned(),
            before_value,
            after_value,
            expected_after_value: None,
            is_password,
            before_password_len,
            after_password_len,
        })
    })
}

fn set_element_value_via_native_text(
    id: &ElementId,
    element: &UIElement,
    value: &str,
    is_password: bool,
    value_pattern_error: &str,
) -> A11yResult<ElementValueSetReadback> {
    if let Some(hwnd) = native_text_hwnd(element)? {
        if native_is_readonly(hwnd) {
            return Err(A11yError::ElementValueReadOnly {
                detail: format!(
                    "native edit HWND is read-only for element {id}; ValuePattern was not exposed: {value_pattern_error}"
                ),
            });
        }
        return set_native_text_value(id, hwnd, value, is_password);
    }
    Err(A11yError::ElementValueUnsupported {
        detail: format!(
            "ValuePattern not exposed for element {id}: {value_pattern_error}; native text-message fallback unavailable because role={:?} native_hwnd={:?}",
            cached_role(element),
            native_hwnd(element)?.map(|hwnd| format!("0x{:x}", hwnd.0 as isize))
        ),
    })
}

fn set_native_text_value(
    id: &ElementId,
    hwnd: HWND,
    value: &str,
    is_password: bool,
) -> A11yResult<ElementValueSetReadback> {
    let before_password_len = if is_password {
        Some(native_text_len(id, hwnd)?)
    } else {
        None
    };
    let before_value = if is_password {
        String::new()
    } else {
        native_text_value(id, hwnd)?
    };
    let expected_after_value = native_set_text(id, hwnd, value)?;
    let after_password_len = if is_password {
        Some(native_text_len(id, hwnd)?)
    } else {
        None
    };
    let after_value = if is_password {
        String::new()
    } else {
        native_text_value(id, hwnd)?
    };
    Ok(ElementValueSetReadback {
        method: "uia_native_window_text_message".to_owned(),
        before_value,
        after_value,
        expected_after_value: if is_password {
            None
        } else {
            Some(expected_after_value)
        },
        is_password,
        before_password_len,
        after_password_len,
    })
}

pub fn element_value(id: &ElementId) -> A11yResult<ElementValueReadback> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        let is_password = cached_bool(&element, UIProperty::IsPassword);
        if let Some(hwnd) = native_text_hwnd(&element)? {
            let password_len = if is_password {
                Some(native_text_len(&id, hwnd)?)
            } else {
                None
            };
            let value = if is_password {
                String::new()
            } else {
                native_text_value(&id, hwnd)?
            };
            return Ok(ElementValueReadback {
                method: "uia_native_window_text_message".to_owned(),
                value,
                is_readonly: native_is_readonly(hwnd),
                is_password,
                password_len,
            });
        }
        let pattern: UIValuePattern = match element.get_pattern() {
            Ok(pattern) => pattern,
            Err(error) => {
                return element_value_via_native_text(
                    &id,
                    &element,
                    is_password,
                    &error.to_string(),
                );
            }
        };
        let is_readonly = pattern.is_readonly().map_err(map_uia_error)?;
        let password_len = if is_password {
            Some(password_text_len(&id, &element)?)
        } else {
            None
        };
        let value = if is_password {
            String::new()
        } else {
            pattern.get_value().map_err(map_uia_error)?
        };
        Ok(ElementValueReadback {
            method: "uia_value_pattern".to_owned(),
            value,
            is_readonly,
            is_password,
            password_len,
        })
    })
}

fn element_value_via_native_text(
    id: &ElementId,
    element: &UIElement,
    is_password: bool,
    value_pattern_error: &str,
) -> A11yResult<ElementValueReadback> {
    if let Some(hwnd) = native_text_hwnd(element)? {
        let password_len = if is_password {
            Some(native_text_len(id, hwnd)?)
        } else {
            None
        };
        let value = if is_password {
            String::new()
        } else {
            native_text_value(id, hwnd)?
        };
        return Ok(ElementValueReadback {
            method: "uia_native_window_text_message".to_owned(),
            value,
            is_readonly: native_is_readonly(hwnd),
            is_password,
            password_len,
        });
    }
    Err(A11yError::ElementValueUnsupported {
        detail: format!(
            "ValuePattern not exposed for element {id}: {value_pattern_error}; native text-message readback unavailable because role={:?} native_hwnd={:?}",
            cached_role(element),
            native_hwnd(element)?.map(|hwnd| format!("0x{:x}", hwnd.0 as isize))
        ),
    })
}

fn password_text_len(id: &ElementId, element: &UIElement) -> A11yResult<usize> {
    let hwnd = native_hwnd(element)?.ok_or_else(|| {
        A11yError::internal(format!(
            "password element {id} has NativeWindowHandle=0; cannot verify password length without reading hidden value text"
        ))
    })?;
    native_text_len(id, hwnd)
}

fn native_text_message_supported(element: &UIElement) -> bool {
    let role = cached_role(element);
    let class_name = element.get_cached_classname().unwrap_or_default();
    native_text_class_supported(&class_name) && role.to_ascii_lowercase().contains("edit")
}

fn native_text_class_supported(class_name: &str) -> bool {
    let class_name = class_name.to_ascii_lowercase();
    class_name == "edit"
        || class_name.starts_with("richedit")
        || class_name.contains(".edit.")
        || class_name.starts_with("windowsforms10.edit.")
}

fn native_text_hwnd(element: &UIElement) -> A11yResult<Option<HWND>> {
    if !native_text_message_supported(element) {
        return Ok(None);
    }
    native_hwnd(element)
}

fn native_hwnd(element: &UIElement) -> A11yResult<Option<HWND>> {
    let handle = element.get_native_window_handle().map_err(|err| {
        A11yError::internal(format!(
            "NativeWindowHandle read failed for native text element: {err}"
        ))
    })?;
    let raw: isize = handle.into();
    if raw == 0 {
        return Ok(None);
    }
    Ok(Some(HWND(raw as *mut c_void)))
}

fn native_text_len(id: &ElementId, hwnd: HWND) -> A11yResult<usize> {
    let mut result = 0_usize;
    send_native_text_message(
        id,
        hwnd,
        WM_GETTEXTLENGTH,
        WPARAM(0),
        LPARAM(0),
        PASSWORD_LENGTH_READ_TIMEOUT_MS,
        "WM_GETTEXTLENGTH",
        &mut result,
    )?;
    Ok(result)
}

fn native_text_value(id: &ElementId, hwnd: HWND) -> A11yResult<String> {
    let len = native_text_len(id, hwnd)?;
    let mut buffer = vec![0_u16; len.saturating_add(1)];
    let mut copied = 0_usize;
    send_native_text_message(
        id,
        hwnd,
        WM_GETTEXT,
        WPARAM(buffer.len()),
        LPARAM(buffer.as_mut_ptr().cast::<c_void>() as isize),
        NATIVE_TEXT_MESSAGE_TIMEOUT_MS,
        "WM_GETTEXT",
        &mut copied,
    )?;
    let copied = copied.min(buffer.len().saturating_sub(1));
    String::from_utf16(&buffer[..copied]).map_err(|err| {
        A11yError::internal(format!(
            "WM_GETTEXT returned invalid UTF-16 for native text element {id}: {err}"
        ))
    })
}

fn native_set_text(id: &ElementId, hwnd: HWND, value: &str) -> A11yResult<String> {
    let expected_value = if native_is_multiline(hwnd) {
        normalize_multiline_edit_newlines(value)
    } else {
        value.to_owned()
    };
    native_set_window_text(id, hwnd, &expected_value)?;
    Ok(expected_value)
}

fn native_is_multiline(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
    style & ES_MULTILINE != 0
}

fn native_is_readonly(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) };
    style & ES_READONLY_STYLE != 0
}

fn normalize_multiline_edit_newlines(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                normalized.push('\r');
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                    normalized.push('\n');
                } else {
                    normalized.push('\n');
                }
            }
            '\n' => {
                normalized.push('\r');
                normalized.push('\n');
            }
            _ => normalized.push(ch),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::native_text_class_supported;

    #[test]
    fn native_text_class_supported_accepts_classic_edit_classes() {
        assert!(native_text_class_supported("Edit"));
        assert!(native_text_class_supported("RICHEDIT50W"));
        assert!(native_text_class_supported(
            "WindowsForms10.EDIT.app.0.1d38a05_r8_ad1"
        ));
    }

    #[test]
    fn native_text_class_supported_rejects_non_edit_window_classes() {
        assert!(!native_text_class_supported("Button"));
        assert!(!native_text_class_supported("Static"));
        assert!(!native_text_class_supported(
            "SynapseIssue784NativeHostWindow"
        ));
    }
}

fn native_set_window_text(id: &ElementId, hwnd: HWND, value: &str) -> A11yResult<()> {
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    wide.push(0);
    let mut result = 0_usize;
    send_native_text_message(
        id,
        hwnd,
        WM_SETTEXT,
        WPARAM(0),
        LPARAM(wide.as_ptr().cast::<c_void>() as isize),
        NATIVE_TEXT_MESSAGE_TIMEOUT_MS,
        "WM_SETTEXT",
        &mut result,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "thin SendMessageTimeoutW wrapper mirrors the Win32 call shape plus error context"
)]
fn send_native_text_message(
    id: &ElementId,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    timeout_ms: u32,
    operation: &'static str,
    result: &mut usize,
) -> A11yResult<()> {
    let send_result = unsafe {
        SendMessageTimeoutW(
            hwnd,
            message,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG,
            timeout_ms,
            Some(result as *mut usize),
        )
    };
    if send_result.0 == 0 {
        return Err(A11yError::internal(format!(
            "SendMessageTimeoutW({operation}) failed or timed out after {timeout_ms}ms for native text element {id} hwnd=0x{:x}",
            hwnd.0 as isize
        )));
    }
    Ok(())
}

pub fn element_metadata(id: &ElementId) -> A11yResult<ElementMetadataReadback> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        let automation_id = element
            .get_cached_automation_id()
            .ok()
            .filter(|value| !value.is_empty());
        Ok(ElementMetadataReadback {
            name: element.get_cached_name().unwrap_or_default(),
            role: cached_role(&element),
            automation_id,
            bbox: element_rect(&element)?,
            enabled: cached_bool(&element, UIProperty::IsEnabled),
            keyboard_focusable: cached_bool(&element, UIProperty::IsKeyboardFocusable),
            patterns: cached_patterns(&element),
            value: cached_value(&element),
        })
    })
}

pub fn scroll_element_into_view(id: &ElementId) -> A11yResult<()> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        if !cached_bool(&element, UIProperty::IsEnabled) {
            return Err(A11yError::ElementNotEnabled {
                detail: format!("element {id} IsEnabled=false before UIA scroll-into-view"),
            });
        }
        let pattern: UIScrollItemPattern =
            element
                .get_pattern()
                .map_err(|err| A11yError::ElementPatternUnsupported {
                    detail: format!(
                        "ScrollItemPattern not exposed for {id}; scroll_element_into_view has no other tier: {err}"
                    ),
                })?;
        pattern.scroll_into_view().map_err(|err| {
            pattern_operation_error(&id, "ScrollItemPattern", "scroll_into_view", &err)
        })
    })
}

pub fn scroll_element(id: &ElementId, dy: i32, dx: i32) -> A11yResult<ElementScrollReadback> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        if !cached_bool(&element, UIProperty::IsEnabled) {
            return Err(A11yError::ElementNotEnabled {
                detail: format!("element {id} IsEnabled=false before UIA scroll dispatch"),
            });
        }

        let before = scroll_state_from_element(&element)?;
        let mut unsupported_reasons = Vec::new();
        let scroll_pattern: Result<UIScrollPattern, _> = element.get_pattern();
        match scroll_pattern {
            Ok(pattern) => {
                let steps = uia_scroll_steps(dy, dx)?;
                for step in &steps {
                    pattern
                        .scroll(step.horizontal, step.vertical)
                        .map_err(|err| {
                            pattern_operation_error(&id, "ScrollPattern", "scroll", &err)
                        })?;
                }
                let after = scroll_state_from_element(&element)?;
                return Ok(ElementScrollReadback {
                    method: "uia_scroll_pattern".to_owned(),
                    before,
                    after,
                    requested_dy: dy,
                    requested_dx: dx,
                    scroll_call_count: u32::try_from(steps.len()).unwrap_or(u32::MAX),
                });
            }
            Err(err) => unsupported_reasons.push(format!("ScrollPattern not exposed: {err}")),
        }

        let scroll_item_pattern: Result<UIScrollItemPattern, _> = element.get_pattern();
        match scroll_item_pattern {
            Ok(pattern) => {
                pattern.scroll_into_view().map_err(|err| {
                    pattern_operation_error(&id, "ScrollItemPattern", "scroll_into_view", &err)
                })?;
                let after = scroll_state_from_element(&element)?;
                Ok(ElementScrollReadback {
                    method: "uia_scroll_item_pattern".to_owned(),
                    before,
                    after,
                    requested_dy: dy,
                    requested_dx: dx,
                    scroll_call_count: 1,
                })
            }
            Err(err) => {
                unsupported_reasons.push(format!("ScrollItemPattern not exposed: {err}"));
                Err(unsupported_scroll_pattern(
                    &id,
                    &element,
                    &unsupported_reasons,
                ))
            }
        }
    })
}

#[derive(Clone, Copy)]
struct UiaScrollStep {
    horizontal: ScrollAmount,
    vertical: ScrollAmount,
}

fn uia_scroll_steps(dy: i32, dx: i32) -> A11yResult<Vec<UiaScrollStep>> {
    let step_count = dy.unsigned_abs().max(dx.unsigned_abs());
    if step_count > MAX_UIA_SCROLL_PATTERN_CALLS {
        return Err(A11yError::ElementPatternUnsupported {
            detail: format!(
                "requested UIA ScrollPattern call count {step_count} exceeds max {MAX_UIA_SCROLL_PATTERN_CALLS}"
            ),
        });
    }
    let capacity = usize::try_from(step_count)
        .map_err(|err| A11yError::internal(format!("UIA scroll step count overflow: {err}")))?;
    let mut vertical_ticks_remaining = dy;
    let mut horizontal_ticks_remaining = dx;
    let mut steps = Vec::with_capacity(capacity);
    for _ in 0..step_count {
        let vertical_tick = take_scroll_tick(&mut vertical_ticks_remaining);
        let horizontal_tick = take_scroll_tick(&mut horizontal_ticks_remaining);
        steps.push(UiaScrollStep {
            horizontal: horizontal_scroll_amount(horizontal_tick),
            vertical: vertical_scroll_amount(vertical_tick),
        });
    }
    Ok(steps)
}

fn take_scroll_tick(value: &mut i32) -> i32 {
    match (*value).cmp(&0) {
        std::cmp::Ordering::Less => {
            *value += 1;
            -1
        }
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => {
            *value -= 1;
            1
        }
    }
}

const fn vertical_scroll_amount(tick: i32) -> ScrollAmount {
    if tick > 0 {
        ScrollAmount::SmallDecrement
    } else if tick < 0 {
        ScrollAmount::SmallIncrement
    } else {
        ScrollAmount::NoAmount
    }
}

const fn horizontal_scroll_amount(tick: i32) -> ScrollAmount {
    if tick > 0 {
        ScrollAmount::SmallDecrement
    } else if tick < 0 {
        ScrollAmount::SmallIncrement
    } else {
        ScrollAmount::NoAmount
    }
}

pub fn element_scroll_state(id: &ElementId) -> A11yResult<ElementScrollStateReadback> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        scroll_state_from_element(&element)
    })
}

fn scroll_state_from_element(element: &UIElement) -> A11yResult<ElementScrollStateReadback> {
    let bbox = element_rect(element)?;
    let scroll_pattern: Result<UIScrollPattern, _> = element.get_pattern();
    let Ok(pattern) = scroll_pattern else {
        return Ok(ElementScrollStateReadback {
            bbox,
            horizontal_scroll_percent: None,
            vertical_scroll_percent: None,
            horizontal_view_size: None,
            vertical_view_size: None,
            horizontally_scrollable: None,
            vertically_scrollable: None,
        });
    };

    Ok(ElementScrollStateReadback {
        bbox,
        horizontal_scroll_percent: finite_scroll_value(
            pattern
                .get_horizontal_scroll_percent()
                .map_err(map_uia_error)?,
        ),
        vertical_scroll_percent: finite_scroll_value(
            pattern
                .get_vertical_scroll_percent()
                .map_err(map_uia_error)?,
        ),
        horizontal_view_size: finite_scroll_value(
            pattern.get_horizontal_view_size().map_err(map_uia_error)?,
        ),
        vertical_view_size: finite_scroll_value(
            pattern.get_vertical_view_size().map_err(map_uia_error)?,
        ),
        horizontally_scrollable: Some(
            pattern
                .is_horizontally_scrollable()
                .map_err(map_uia_error)?,
        ),
        vertically_scrollable: Some(pattern.is_vertically_scrollable().map_err(map_uia_error)?),
    })
}

fn finite_scroll_value(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn unsupported_scroll_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &[String],
) -> A11yError {
    let name = element.get_cached_name().unwrap_or_default();
    let role = cached_role(element);
    let automation_id = element.get_cached_automation_id().unwrap_or_default();
    let patterns = cached_patterns(element);
    tracing::warn!(
        code = "A11Y_SCROLL_CONTROL_PATTERN_UNSUPPORTED",
        element_id = %id,
        element_name = %name,
        element_role = %role,
        automation_id = %automation_id,
        patterns = ?patterns,
        attempted_patterns = ?SUPPORTED_SCROLL_PATTERNS,
        unsupported_reasons = ?unsupported_reasons,
        "UIA element does not expose a supported scroll control pattern; no coordinate fallback synthesized"
    );
    A11yError::ElementPatternUnsupported {
        detail: format!(
            "element {id} does not expose a supported UIA scroll control pattern; name={name:?} role={role:?} automation_id={automation_id:?}; patterns={patterns:?}; attempted_patterns={SUPPORTED_SCROLL_PATTERNS:?}; unsupported_reasons={unsupported_reasons:?}"
        ),
    }
}

pub fn expand_state_of(element: &UIElement) -> A11yResult<ExpandState> {
    let _ = element;
    Err(A11yError::internal(
        "direct UIElement ExpandCollapse read is disabled; use expand_state_of_id so UIA stays on the dedicated MTA worker",
    ))
}

pub fn expand_state_of_id(id: &ElementId) -> A11yResult<ExpandState> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        expand_state_from_element(&element)
    })
}

fn expand_state_from_element(element: &UIElement) -> A11yResult<ExpandState> {
    let pattern: UIExpandCollapsePattern =
        element.get_pattern().map_err(|err| A11yError::Internal {
            detail: format!("ExpandCollapsePattern not exposed: {err}"),
        })?;
    let state = pattern.get_state().map_err(map_uia_error)?;
    Ok(expand_state_from_uia(state))
}

const fn expand_state_from_uia(state: ExpandCollapseState) -> ExpandState {
    match state {
        ExpandCollapseState::Collapsed => ExpandState::Collapsed,
        ExpandCollapseState::Expanded => ExpandState::Expanded,
        ExpandCollapseState::PartiallyExpanded => ExpandState::PartiallyExpanded,
        ExpandCollapseState::LeafNode => ExpandState::LeafNode,
    }
}

fn toggle_state_label(state: uiautomation::types::ToggleState) -> String {
    format!("{state:?}")
}

fn pattern_operation_error(
    id: &ElementId,
    pattern: &'static str,
    operation: &'static str,
    err: &uiautomation::Error,
) -> A11yError {
    let provider_detail = err.to_string();
    if stale_provider_error(&provider_detail) {
        tracing::warn!(
            code = "A11Y_PATTERN_OPERATION_STALE_AFTER_RE_RESOLVE",
            element_id = %id,
            pattern,
            operation,
            provider_error = %provider_detail,
            "UIA provider reported a stale/disposed target during control-pattern dispatch"
        );
        return A11yError::ElementStale {
            detail: format!(
                "{pattern}.{operation} failed for element {id} after re-resolve; provider reported stale/disposed target: {provider_detail}"
            ),
        };
    }
    A11yError::internal(format!(
        "{pattern}.{operation} failed for element {id}: {provider_detail}"
    ))
}

fn stale_provider_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("event was unable to invoke any of the subscribers")
        || detail.contains("element not available")
        || detail.contains("element is no longer available")
        || detail.contains("uia_e_elementnotavailable")
}

fn unsupported_click_pattern(
    id: &ElementId,
    element: &UIElement,
    unsupported_reasons: &[String],
) -> A11yError {
    let name = element.get_cached_name().unwrap_or_default();
    let role = cached_role(element);
    let automation_id = element.get_cached_automation_id().unwrap_or_default();
    tracing::warn!(
        code = "A11Y_CLICK_CONTROL_PATTERN_UNSUPPORTED",
        element_id = %id,
        element_name = %name,
        element_role = %role,
        automation_id = %automation_id,
        attempted_patterns = ?SUPPORTED_CLICK_PATTERNS,
        unsupported_reasons = ?unsupported_reasons,
        "UIA element does not expose a supported click control pattern; no coordinate fallback synthesized"
    );
    A11yError::ElementPatternUnsupported {
        detail: format!(
            "element {id} does not expose a supported click control pattern; name={name:?} role={role:?} automation_id={automation_id:?}; attempted_patterns={SUPPORTED_CLICK_PATTERNS:?}; unsupported_reasons={unsupported_reasons:?}"
        ),
    }
}

fn element_rect(element: &UIElement) -> A11yResult<Rect> {
    element
        .get_bounding_rectangle()
        .map(rect_from_uia)
        .map_err(map_uia_error)
}

fn rect_from_uia(rect: UiaRect) -> Rect {
    Rect {
        x: rect.get_left(),
        y: rect.get_top(),
        w: rect.get_right().saturating_sub(rect.get_left()),
        h: rect.get_bottom().saturating_sub(rect.get_top()),
    }
}

fn find_by_runtime_id_hex(
    automation: &UIAutomation,
    hwnd: isize,
    runtime_id_hex_expected: &str,
    root_hwnd: i64,
    tree_view: TreeView,
) -> A11yResult<Option<UIElement>> {
    let cache = create_cache_request(automation, 0, ElementMode::Full, tree_view)?;
    let root = automation
        .element_from_handle_build_cache(Handle::from(hwnd), &cache)
        .map_err(map_uia_error)?;
    if runtime_id_matches(&root, runtime_id_hex_expected, root_hwnd) {
        return Ok(Some(root));
    }

    let true_condition = automation.create_true_condition().map_err(map_uia_error)?;
    let elements = root
        .find_all_build_cache(TreeScope::Descendants, &true_condition, &cache)
        .map_err(map_uia_error)?;
    for (visited, element) in elements.into_iter().enumerate() {
        if visited >= RE_RESOLVE_NODE_BUDGET {
            tracing::warn!(
                code = "A11Y_RE_RESOLVE_NODE_BUDGET_EXCEEDED",
                root_hwnd,
                budget = RE_RESOLVE_NODE_BUDGET,
                runtime_id_hex_expected,
                "UIA element re-resolve stopped before scanning the full subtree"
            );
            return Ok(None);
        }
        if runtime_id_matches(&element, runtime_id_hex_expected, root_hwnd) {
            return Ok(Some(element));
        }
    }
    Ok(None)
}

fn runtime_id_matches(element: &UIElement, runtime_id_hex_expected: &str, root_hwnd: i64) -> bool {
    let hwnd = cached_hwnd(element)
        .filter(|value| *value != 0)
        .unwrap_or(root_hwnd);
    match cached_runtime_id_hex_or_fallback(element, hwnd) {
        Ok(readback) => readback.hex.eq_ignore_ascii_case(runtime_id_hex_expected),
        Err(error) => {
            tracing::warn!(
                code = "A11Y_RE_RESOLVE_RUNTIME_ID_FAILED",
                error = %error,
                element_name = %element.get_cached_name().unwrap_or_default(),
                element_class = %element.get_cached_classname().unwrap_or_default(),
                control_type = ?element.get_cached_control_type().ok(),
                automation_id = %element.get_cached_automation_id().unwrap_or_default(),
                process_id = element.get_cached_process_id().unwrap_or(-1),
                "cached RuntimeId read failed during re-resolve; current element skipped"
            );
            false
        }
    }
}
