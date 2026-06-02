use synapse_core::{ElementId, Rect};
use uiautomation::{
    UIAutomation, UIElement,
    patterns::UIExpandCollapsePattern,
    patterns::UIInvokePattern,
    types::{ElementMode, ExpandCollapseState, Handle, Rect as UiaRect, TreeScope},
};

use crate::{A11yError, A11yResult, ElementClickAction, ExpandState};

use super::common::{
    TreeView, cached_hwnd, cached_runtime_id_hex_or_fallback, create_cache_request, map_uia_error,
    with_automation,
};

const RE_RESOLVE_NODE_BUDGET: usize = 20_000;

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
        let pattern: Result<UIInvokePattern, _> = element.get_pattern();
        match pattern {
            Ok(pattern) => {
                pattern.invoke().map_err(|err| {
                    A11yError::internal(format!(
                        "InvokePattern.invoke failed for element {id}: {err}"
                    ))
                })?;
                Ok(ElementClickAction::Invoked)
            }
            Err(_missing_pattern) => Ok(ElementClickAction::CoordinateFallback {
                bbox: element_rect(&element)?,
            }),
        }
    })
}

pub fn focus_element(id: &ElementId) -> A11yResult<()> {
    let id = id.clone();
    with_automation(move |automation| {
        let element = re_resolve_on_worker(automation, &id)?;
        element.set_focus().map_err(map_uia_error)
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
    Ok(match state {
        ExpandCollapseState::Collapsed => ExpandState::Collapsed,
        ExpandCollapseState::Expanded => ExpandState::Expanded,
        ExpandCollapseState::PartiallyExpanded => ExpandState::PartiallyExpanded,
        ExpandCollapseState::LeafNode => ExpandState::LeafNode,
    })
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
    if runtime_id_matches(&root, runtime_id_hex_expected, root_hwnd)? {
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
        if runtime_id_matches(&element, runtime_id_hex_expected, root_hwnd)? {
            return Ok(Some(element));
        }
    }
    Ok(None)
}

fn runtime_id_matches(
    element: &UIElement,
    runtime_id_hex_expected: &str,
    root_hwnd: i64,
) -> A11yResult<bool> {
    let hwnd = cached_hwnd(element)
        .filter(|value| *value != 0)
        .unwrap_or(root_hwnd);
    match cached_runtime_id_hex_or_fallback(element, hwnd) {
        Ok(readback) => Ok(readback.hex.eq_ignore_ascii_case(runtime_id_hex_expected)),
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
            Ok(false)
        }
    }
}
