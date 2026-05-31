use std::time::{Duration, Instant};
use std::{collections::HashSet, sync::Mutex};

use synapse_core::{AccessibleNode, AccessibleSubtree, ElementId, UiaPattern, element_id};
use uiautomation::{
    UIAutomation, UIElement,
    core::{UICacheRequest, UICondition},
    types::{ElementMode, PropertyConditionFlags, TreeScope, UIProperty},
    variants::Variant,
};

use crate::{A11yError, A11yResult, ElementSearchScope, ids::runtime_id_hex};

use super::common::{
    TreeView, cached_hwnd, cached_patterns, cached_rect, cached_role, cached_runtime_id,
    cached_value, create_cache_request, map_uia_error, non_empty, pattern_property, with_automation,
};

static SNAPSHOT_CACHE: Mutex<Option<SnapshotCache>> = Mutex::new(None);
const RAW_SUPPLEMENT_DEPTH: u32 = 2;
/// Hard ceiling on nodes collected in one snapshot. Cross-process trees (UWP,
/// browsers) can be large; this bounds worst-case work while preserving every
/// node collected up to the cap (the result is flagged `truncated`, never
/// silently collapsed).
const SNAPSHOT_NODE_BUDGET: usize = 4000;
/// Wall-clock budget for one snapshot walk. When exceeded the walk stops
/// descending further but KEEPS everything collected so far and flags
/// `truncated`. This replaces the previous behaviour, which discarded a
/// complete tree and re-ran at depth 1 whenever a walk took >25ms — fatal for
/// inherently slower cross-process UWP/ApplicationFrameHost trees.
const SNAPSHOT_DEADLINE: Duration = Duration::from_millis(400);
const RAW_SUPPLEMENT_NODE_BUDGET: usize = 60;
// Packaged Notepad exposes these top-level menu items through raw
// name+ExpandCollapse search even when RawView child walking omits them.
const RAW_MENU_SUPPLEMENT_NAMES: [&str; 3] = ["File", "Edit", "View"];

struct SnapshotCache {
    requested_depth: u32,
    captured_at: Instant,
    tree: AccessibleSubtree,
}

struct SnapshotWalk<'a> {
    /// True condition so every child is enumerated (raw view equivalent).
    true_condition: &'a UICondition,
    cache: &'a UICacheRequest,
    root_hwnd: i64,
    /// Stop descending once this many nodes are collected.
    node_budget: usize,
    /// Stop descending once this instant is reached.
    deadline: Instant,
}

pub fn snapshot(root: &UIElement, depth: u32) -> A11yResult<AccessibleSubtree> {
    if let Some(tree) = cached_snapshot(depth) {
        return Ok(tree);
    }

    with_automation(|automation| {
        let started = Instant::now();
        let mut tree = snapshot_at_depth(automation, root, depth)?;
        if depth >= RAW_SUPPLEMENT_DEPTH {
            tree.truncated |= supplement_raw_pattern_nodes(automation, root, &mut tree.nodes)?;
            tree.max_depth = tree.max_depth.max(RAW_SUPPLEMENT_DEPTH);
        }
        // Observability: `truncated` is also surfaced to callers (observe
        // diagnostics) so an incomplete tree is never mistaken for a complete
        // one. A truncated tree means the node budget/deadline was hit or a
        // subtree errored (the per-subtree error is logged at warn separately).
        tracing::debug!(
            code = "A11Y_SNAPSHOT_ASSEMBLED",
            requested_depth = depth,
            nodes = tree.nodes.len(),
            max_depth = tree.max_depth,
            truncated = tree.truncated,
            elapsed_ms = started.elapsed().as_millis(),
            "a11y snapshot assembled"
        );
        store_snapshot(depth, &tree);
        Ok(tree)
    })
}

pub fn find_by_name_and_pattern(
    root: &UIElement,
    name: &str,
    pattern: UiaPattern,
    scope: ElementSearchScope,
) -> A11yResult<Option<AccessibleNode>> {
    if name.is_empty() {
        return Ok(None);
    }

    with_automation(|automation| {
        let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
        let name_condition = automation
            .create_property_condition(
                UIProperty::Name,
                Variant::from(name),
                Some(PropertyConditionFlags::IgnoreCase),
            )
            .map_err(map_uia_error)?;
        let pattern_condition = automation
            .create_property_condition(pattern_property(pattern), Variant::from(true), None)
            .map_err(map_uia_error)?;
        let condition = automation
            .create_and_condition(name_condition, pattern_condition)
            .map_err(map_uia_error)?;
        let elements = root
            .find_all_build_cache(scope.into(), &condition, &cache)
            .map_err(map_uia_error)?;
        let root_hwnd = root
            .build_updated_cache(&cache)
            .ok()
            .and_then(|cached_root| cached_hwnd(&cached_root))
            .unwrap_or(0);

        elements
            .into_iter()
            .filter(|element| element.is_cached_enabled().unwrap_or(true))
            .map(|element| node_from_cached_element(&element, None, 0, root_hwnd, 0))
            .next()
            .transpose()
    })
}
fn snapshot_at_depth(
    automation: &UIAutomation,
    root: &UIElement,
    depth: u32,
) -> A11yResult<AccessibleSubtree> {
    let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
    let true_condition = automation.create_true_condition().map_err(map_uia_error)?;
    let cached_root = root.build_updated_cache(&cache).map_err(map_uia_error)?;
    let root_hwnd = cached_hwnd(&cached_root).unwrap_or(0);
    let mut nodes = Vec::new();
    let mut truncated = false;
    let walk = SnapshotWalk {
        true_condition: &true_condition,
        cache: &cache,
        root_hwnd,
        node_budget: SNAPSHOT_NODE_BUDGET,
        deadline: Instant::now() + SNAPSHOT_DEADLINE,
    };
    collect_nodes(&walk, &cached_root, None, 0, depth, &mut nodes, &mut truncated)?;
    let root = nodes
        .first()
        .map(|node| node.element_id.clone())
        .ok_or_else(|| A11yError::ElementStale {
            detail: "snapshot root produced no UIA node".to_owned(),
        })?;
    Ok(AccessibleSubtree {
        root,
        nodes,
        max_depth: depth,
        truncated,
    })
}

fn supplement_raw_pattern_nodes(
    automation: &UIAutomation,
    root: &UIElement,
    nodes: &mut Vec<AccessibleNode>,
) -> A11yResult<bool> {
    let Some(root_id) = nodes.first().map(|node| node.element_id.clone()) else {
        return Ok(false);
    };
    let root_hwnd = root_id
        .parts()
        .map_err(|err| A11yError::InvalidElementId {
            detail: err.to_string(),
        })?
        .hwnd;
    let cache = create_cache_request(automation, 0, ElementMode::Full, TreeView::Raw)?;
    let mut seen: HashSet<ElementId> = nodes.iter().map(|node| node.element_id.clone()).collect();
    let mut truncated = false;
    for name in RAW_MENU_SUPPLEMENT_NAMES {
        let name_condition = automation
            .create_property_condition(
                UIProperty::Name,
                Variant::from(name),
                Some(PropertyConditionFlags::IgnoreCase),
            )
            .map_err(map_uia_error)?;
        let pattern_condition = automation
            .create_property_condition(
                UIProperty::IsExpandCollapsePatternAvailable,
                Variant::from(true),
                None,
            )
            .map_err(map_uia_error)?;
        let condition = automation
            .create_and_condition(name_condition, pattern_condition)
            .map_err(map_uia_error)?;
        let raw_elements = root
            .find_all_build_cache(TreeScope::Subtree, &condition, &cache)
            .map_err(map_uia_error)?;
        for element in raw_elements {
            if nodes.len() >= RAW_SUPPLEMENT_NODE_BUDGET {
                truncated = true;
                break;
            }
            let node = node_from_cached_element(
                &element,
                Some(root_id.clone()),
                RAW_SUPPLEMENT_DEPTH,
                root_hwnd,
                0,
            )?;
            if seen.insert(node.element_id.clone()) {
                nodes.push(node);
            }
        }
        if truncated {
            break;
        }
    }
    Ok(truncated)
}

fn cached_snapshot(depth: u32) -> Option<AccessibleSubtree> {
    let guard = SNAPSHOT_CACHE.lock().ok()?;
    let cache = guard.as_ref()?;
    let is_fresh =
        cache.requested_depth == depth && cache.captured_at.elapsed() <= Duration::from_millis(50);
    let tree = is_fresh.then(|| cache.tree.clone());
    drop(guard);
    tree
}

fn store_snapshot(depth: u32, tree: &AccessibleSubtree) {
    if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
        *guard = Some(SnapshotCache {
            requested_depth: depth,
            captured_at: Instant::now(),
            tree: tree.clone(),
        });
    }
}

pub(super) fn invalidate_snapshot_cache() {
    if let Ok(mut guard) = SNAPSHOT_CACHE.lock() {
        *guard = None;
    }
}

fn collect_nodes(
    walk: &SnapshotWalk<'_>,
    element: &UIElement,
    parent: Option<ElementId>,
    depth: u32,
    max_depth: u32,
    nodes: &mut Vec<AccessibleNode>,
    truncated: &mut bool,
) -> A11yResult<ElementId> {
    let children = if depth >= max_depth {
        Vec::new()
    } else if nodes.len() >= walk.node_budget || Instant::now() >= walk.deadline {
        // Budget/deadline hit: keep everything collected, but do not descend
        // further and flag the result as incomplete (never silently complete).
        *truncated = true;
        Vec::new()
    } else {
        // `find_all_build_cache(Children, true)` returns a `Result` (unlike the
        // tree walker's `get_children_build_cache`, which the crate collapses to
        // `Option`/`None` and hides the error). It also reliably crosses the
        // cross-process `Windows.UI.Core.CoreWindow` boundary for UWP apps.
        match element.find_all_build_cache(TreeScope::Children, walk.true_condition, walk.cache) {
            Ok(children) => children,
            Err(err) => {
                *truncated = true;
                tracing::warn!(
                    code = "A11Y_CHILD_ENUM_FAILED",
                    error = %err,
                    depth,
                    element_name = %element.get_cached_name().unwrap_or_default(),
                    element_class = %element.get_cached_classname().unwrap_or_default(),
                    control_type = ?element.get_cached_control_type().ok(),
                    automation_id = %element.get_cached_automation_id().unwrap_or_default(),
                    process_id = element.get_cached_process_id().unwrap_or(-1),
                    "UIA child enumeration failed; subtree omitted and snapshot flagged truncated"
                );
                Vec::new()
            }
        }
    };
    let node = node_from_cached_element(element, parent, depth, walk.root_hwnd, children.len())?;
    let node_id = node.element_id.clone();
    nodes.push(node);
    for child in children {
        collect_nodes(
            walk,
            &child,
            Some(node_id.clone()),
            depth + 1,
            max_depth,
            nodes,
            truncated,
        )?;
    }
    Ok(node_id)
}

fn node_from_cached_element(
    element: &UIElement,
    parent: Option<ElementId>,
    depth: u32,
    root_hwnd: i64,
    children_count: usize,
) -> A11yResult<AccessibleNode> {
    let runtime_id = cached_runtime_id(element)?;
    let runtime_id_hex = runtime_id_hex(&runtime_id);
    let hwnd = cached_hwnd(element)
        .filter(|value| *value != 0)
        .unwrap_or(root_hwnd);
    Ok(AccessibleNode {
        element_id: element_id(hwnd, &runtime_id_hex),
        parent,
        name: element.get_cached_name().unwrap_or_default(),
        role: cached_role(element),
        automation_id: non_empty(element.get_cached_automation_id().unwrap_or_default()),
        value: cached_value(element),
        bbox: cached_rect(element),
        enabled: element.is_cached_enabled().unwrap_or(false),
        focused: element.has_cached_keyboard_focus().unwrap_or(false),
        patterns: cached_patterns(element),
        children_count: u32::try_from(children_count).unwrap_or(u32::MAX),
        depth,
    })
}
