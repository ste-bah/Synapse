use synapse_core::{ElementId, Rect};
use uiautomation::{
    UIAutomation, UIElement,
    patterns::{
        UIExpandCollapsePattern, UIInvokePattern, UILegacyIAccessiblePattern,
        UISelectionItemPattern, UITogglePattern, UIValuePattern,
    },
    types::{ElementMode, ExpandCollapseState, Handle, Rect as UiaRect, TreeScope},
};

use crate::{A11yError, A11yResult, ElementClickAction, ElementValueSetReadback, ExpandState};

use super::common::{
    TreeView, cached_hwnd, cached_role, cached_runtime_id_hex_or_fallback, create_cache_request,
    map_uia_error, with_automation,
};

const RE_RESOLVE_NODE_BUDGET: usize = 20_000;
const SUPPORTED_CLICK_PATTERNS: [&str; 5] = [
    "InvokePattern",
    "TogglePattern",
    "SelectionItemPattern",
    "ExpandCollapsePattern",
    "LegacyIAccessiblePattern.DoDefaultAction",
];

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
        let pattern: UIValuePattern = element.get_pattern().map_err(|err| {
            A11yError::internal(format!("ValuePattern not exposed for element {id}: {err}"))
        })?;
        if pattern.is_readonly().map_err(map_uia_error)? {
            return Err(A11yError::internal(format!(
                "ValuePattern is read-only for element {id}"
            )));
        }
        let before_value = pattern.get_value().map_err(map_uia_error)?;
        pattern.set_value(&value).map_err(map_uia_error)?;
        let after_value = pattern.get_value().map_err(map_uia_error)?;
        Ok(ElementValueSetReadback {
            method: "uia_value_pattern".to_owned(),
            before_value,
            after_value,
        })
    })
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
