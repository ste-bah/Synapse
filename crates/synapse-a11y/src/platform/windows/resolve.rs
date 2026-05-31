use synapse_core::ElementId;
use uiautomation::{
    UIElement,
    patterns::UIExpandCollapsePattern,
    types::{ElementMode, ExpandCollapseState, Handle},
};

use crate::{A11yError, A11yResult, ExpandState, ids::runtime_id_hex};

use super::common::{
    TreeView, cached_runtime_id, create_cache_request, map_uia_error, with_automation,
};

pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
    let parts = id.parts().map_err(|err| A11yError::InvalidElementId {
        detail: err.to_string(),
    })?;
    with_automation(|automation| {
        let control_cache =
            create_cache_request(automation, 8, ElementMode::Full, TreeView::Control)?;
        let hwnd = isize::try_from(parts.hwnd).map_err(|err| A11yError::InvalidElementId {
            detail: err.to_string(),
        })?;
        let root = automation
            .element_from_handle_build_cache(Handle::from(hwnd), &control_cache)
            .map_err(map_uia_error)?;
        if let Some(found) = find_by_runtime_id_hex(&root, &parts.runtime_id_hex, 0, 8)? {
            return Ok(found);
        }

        let raw_cache = create_cache_request(automation, 8, ElementMode::Full, TreeView::Raw)?;
        let raw_root = automation
            .element_from_handle_build_cache(Handle::from(hwnd), &raw_cache)
            .map_err(map_uia_error)?;
        find_by_runtime_id_hex(&raw_root, &parts.runtime_id_hex, 0, 8)?.ok_or_else(|| {
            A11yError::ElementStale {
                detail: format!(
                    "element id {id} was not found under hwnd 0x{:x} in control or raw view",
                    parts.hwnd
                ),
            }
        })
    })
}

pub fn expand_state_of(element: &UIElement) -> A11yResult<ExpandState> {
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
fn find_by_runtime_id_hex(
    root: &UIElement,
    runtime_id_hex_expected: &str,
    depth: u32,
    max_depth: u32,
) -> A11yResult<Option<UIElement>> {
    let runtime_id = cached_runtime_id(root)?;
    if runtime_id_hex(&runtime_id).eq_ignore_ascii_case(runtime_id_hex_expected) {
        return Ok(Some(root.clone()));
    }
    if depth >= max_depth {
        return Ok(None);
    }

    let children = match root.get_cached_children() {
        Ok(children) => children,
        Err(err) => {
            // Do not silently treat a navigation failure as "no children":
            // log it with context so a failed re-resolve is diagnosable.
            tracing::warn!(
                code = "A11Y_CACHED_CHILDREN_FAILED",
                error = %err,
                depth,
                element_name = %root.get_cached_name().unwrap_or_default(),
                element_class = %root.get_cached_classname().unwrap_or_default(),
                "cached child navigation failed during re-resolve; subtree skipped"
            );
            Vec::new()
        }
    };
    for child in children {
        if let Some(found) =
            find_by_runtime_id_hex(&child, runtime_id_hex_expected, depth + 1, max_depth)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
