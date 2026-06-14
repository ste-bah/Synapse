//! CDP DOM/accessibility tree → queryable `AccessibleNode` mapping (#685).
//!
//! When CDP is attached to a Chromium-family foreground, this module pulls the
//! page's `Accessibility.getFullAXTree` (+ `DOM.getBoxModel` for bounds) and maps
//! each node into the same [`AccessibleNode`] model the UIA path uses, so an
//! agent can `find(role="button", name_substring="Apply")` on a web page exactly
//! as it does on native UI.
//!
//! The pure mapping ([`build_accessible_nodes`]) is unit-tested with real CDP
//! response shapes; the async fetch ([`fetch_dom_snapshot`]) is the I/O wrapper
//! verified manually against a live Chrome (see `examples/cdp_axtree_probe.rs`).
//!
//! ## Element id scheme
//!
//! Web nodes get an [`ElementId`] of
//! `<hwnd_hex>:cdcd<targetId-hex><backendNodeId-hex>`. The `cdcd` sentinel lets
//! the action layer (#686) recognise a CDP-resolved node and route it back
//! through CDP (`DOM.scrollIntoViewIfNeeded` + box model + `Input.dispatch*`)
//! instead of UIA re-resolution. The target id and backendNodeId round-trip out
//! of the id with no side registry. Legacy backend-only ids still parse, but
//! new observations carry the target id so duplicate tab titles cannot steer
//! follow-up actions/read_text to the wrong document.
//!
//! ## bbox semantics
//!
//! Web-node `bbox` is the element's CSS-pixel rectangle in page-layout
//! coordinates (from `DOM.getBoxModel`), NOT screen pixels. Actions do not rely
//! on it — they re-resolve the live box model at click time after scrolling the
//! node into view — but it lets an agent reason about on-page layout.

use synapse_core::{AccessibleNode, ElementId, Rect, element_id};

/// Sentinel prefix in the runtime-id portion of a web node's [`ElementId`].
/// Hex-only so the id still parses; `cdcd` is vanishingly unlikely to collide
/// with a real UIA runtime id.
pub const CDP_RUNTIME_PREFIX: &str = "cdcd";
const CDP_BACKEND_NODE_HEX_LEN: usize = 12;

/// One node distilled from a CDP `AXNode` (+ its resolved box model). This is the
/// crate-internal, browser-free representation the pure mapper consumes so it can
/// be unit-tested without a live Chrome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CdpDomNode {
    /// `DOM.BackendNodeId` of the associated DOM node (stable within a document).
    pub backend_node_id: i64,
    /// Backend id of this node's nearest mapped ancestor, if any.
    pub parent_backend_node_id: Option<i64>,
    /// Computed ARIA/AX role (e.g. `button`, `link`, `textbox`, `heading`).
    pub role: String,
    /// Accessible name.
    pub name: String,
    /// Accessible value (form fields, sliders), if any.
    pub value: Option<String>,
    /// CDP Page.FrameId owning this node, when known.
    pub frame_id: Option<String>,
    /// Element rectangle in CSS px / page-layout coords, if a box model resolved.
    pub bbox: Option<Rect>,
    /// Number of mapped child nodes.
    pub child_count: u32,
    /// Whether the node is enabled (defaults true unless AX reports disabled).
    pub enabled: bool,
    /// Whether the node is focused.
    pub focused: bool,
}

/// Builds an [`ElementId`] for a web node from the browser window `hwnd` and the
/// DOM `backend_node_id`.
#[must_use]
pub fn cdp_element_id(hwnd: i64, backend_node_id: i64) -> ElementId {
    // Backend ids are non-negative and small; mask to a stable unsigned hex.
    let unsigned = u64::from_ne_bytes(backend_node_id.to_ne_bytes());
    element_id(
        hwnd,
        &format!("{CDP_RUNTIME_PREFIX}{unsigned:0CDP_BACKEND_NODE_HEX_LEN$x}"),
    )
}

/// Builds a CDP web-node id that carries both the selected target id and the
/// backend node id. If Chromium ever returns a non-hex target id, fall back to
/// the legacy backend-only shape rather than emitting an invalid [`ElementId`].
#[must_use]
pub fn cdp_element_id_for_target(hwnd: i64, target_id: &str, backend_node_id: i64) -> ElementId {
    let target_id = target_id.trim();
    if target_id.is_empty() || !target_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return cdp_element_id(hwnd, backend_node_id);
    }
    let unsigned = u64::from_ne_bytes(backend_node_id.to_ne_bytes());
    element_id(
        hwnd,
        &format!(
            "{CDP_RUNTIME_PREFIX}{}{unsigned:0CDP_BACKEND_NODE_HEX_LEN$x}",
            target_id.to_ascii_lowercase()
        ),
    )
}

/// If `id` is a CDP web-node id (`…:cdcd<hex>`), returns its `backend_node_id`.
/// Returns `None` for ordinary UIA element ids so the action layer can tell them
/// apart.
#[must_use]
pub fn cdp_backend_from_element_id(id: &ElementId) -> Option<i64> {
    let parts = id.parts().ok()?;
    let hex = parts.runtime_id_hex.strip_prefix(CDP_RUNTIME_PREFIX)?;
    let backend_hex = hex
        .get(hex.len().saturating_sub(CDP_BACKEND_NODE_HEX_LEN)..)
        .filter(|value| value.len() == CDP_BACKEND_NODE_HEX_LEN)?;
    let unsigned = u64::from_str_radix(backend_hex, 16).ok()?;
    Some(i64::from_ne_bytes(unsigned.to_ne_bytes()))
}

/// If `id` is a target-aware CDP web-node id, returns the CDP TargetID encoded
/// in it. Legacy backend-only ids return `None`.
#[must_use]
pub fn cdp_target_from_element_id(id: &ElementId) -> Option<String> {
    let parts = id.parts().ok()?;
    let hex = parts.runtime_id_hex.strip_prefix(CDP_RUNTIME_PREFIX)?;
    let target_hex = hex.get(..hex.len().checked_sub(CDP_BACKEND_NODE_HEX_LEN)?)?;
    if target_hex.is_empty() {
        return None;
    }
    Some(target_hex.to_ascii_uppercase())
}

/// Pure mapping: CDP nodes → `AccessibleNode`s.
///
/// The mapper assigns stable ids, computes depth from the parent-backend chain,
/// and carries bounds through. Nodes are returned in input order, capped at
/// `max_nodes`. The mapping never invents data — a node with no box model gets a
/// zero `bbox` (callers may filter on it).
#[must_use]
pub fn build_accessible_nodes(
    hwnd: i64,
    nodes: &[CdpDomNode],
    max_nodes: usize,
) -> Vec<AccessibleNode> {
    build_accessible_nodes_for_target(hwnd, None, nodes, max_nodes)
}

/// Pure mapping variant that embeds the selected page target in every web node
/// id so later CDP actions/read_text resolve the same tab the observation read.
#[must_use]
pub fn build_accessible_nodes_for_target(
    hwnd: i64,
    target_id: Option<&str>,
    nodes: &[CdpDomNode],
    max_nodes: usize,
) -> Vec<AccessibleNode> {
    use std::collections::HashMap;

    // Depth = chain length from a root (a node whose parent is absent or not in
    // the set). Memoised so a deep tree stays linear.
    let by_backend: HashMap<i64, &CdpDomNode> = nodes
        .iter()
        .map(|node| (node.backend_node_id, node))
        .collect();
    let mut depth_cache: HashMap<i64, u32> = HashMap::new();

    nodes
        .iter()
        .take(max_nodes)
        .map(|node| {
            let depth = depth_of(node.backend_node_id, &by_backend, &mut depth_cache, 256);
            let element_id = target_id.map_or_else(
                || cdp_element_id(hwnd, node.backend_node_id),
                |target_id| cdp_element_id_for_target(hwnd, target_id, node.backend_node_id),
            );
            AccessibleNode {
                element_id,
                parent: node
                    .parent_backend_node_id
                    .filter(|parent| by_backend.contains_key(parent))
                    .map(|parent| {
                        target_id.map_or_else(
                            || cdp_element_id(hwnd, parent),
                            |target_id| cdp_element_id_for_target(hwnd, target_id, parent),
                        )
                    }),
                name: node.name.clone(),
                role: node.role.clone(),
                automation_id: Some(cdp_automation_id(target_id, node)),
                value: node.value.clone(),
                bbox: node.bbox.unwrap_or(Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                }),
                enabled: node.enabled,
                focused: node.focused,
                patterns: Vec::new(),
                children_count: node.child_count,
                depth,
            }
        })
        .collect()
}

fn cdp_automation_id(target_id: Option<&str>, node: &CdpDomNode) -> String {
    let mut parts = Vec::with_capacity(3);
    if let Some(target_id) = target_id.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("targetId={}", target_id.trim()));
    }
    if let Some(frame_id) = node
        .frame_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("frameId={}", frame_id.trim()));
    }
    parts.push(format!("backendNodeId={}", node.backend_node_id));
    format!("cdp:{}", parts.join(";"))
}

/// Depth of `backend` = chain length to a root, memoised in `cache`. `guard`
/// bounds recursion against a malformed (cyclic) parent chain.
fn depth_of(
    backend: i64,
    by_backend: &std::collections::HashMap<i64, &CdpDomNode>,
    cache: &mut std::collections::HashMap<i64, u32>,
    guard: u32,
) -> u32 {
    if let Some(found) = cache.get(&backend) {
        return *found;
    }
    if guard == 0 {
        return 0;
    }
    let depth = by_backend
        .get(&backend)
        .and_then(|node| node.parent_backend_node_id)
        .filter(|parent| by_backend.contains_key(parent))
        .map_or(0, |parent| {
            depth_of(parent, by_backend, cache, guard - 1) + 1
        });
    cache.insert(backend, depth);
    depth
}

/// Axis-aligned bounding rect of a CDP box-model content quad
/// (`[x1,y1,x2,y2,x3,y3,x4,y4]`). Returns `None` for a malformed quad.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "page coordinates are rounded then cast into the i32 Rect space"
)]
pub fn rect_from_quad(quad: &[f64]) -> Option<Rect> {
    if quad.len() < 8 {
        return None;
    }
    let xs = [quad[0], quad[2], quad[4], quad[6]];
    let ys = [quad[1], quad[3], quad[5], quad[7]];
    let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
        return None;
    }
    Some(Rect {
        x: min_x.round() as i32,
        y: min_y.round() as i32,
        w: (max_x - min_x).round().max(0.0) as i32,
        h: (max_y - min_y).round().max(0.0) as i32,
    })
}

/// A fully-resolved CDP DOM snapshot ready to fold into observation elements.
#[derive(Clone, Debug)]
pub struct CdpDomSnapshot {
    /// Mapped, queryable web nodes.
    pub nodes: Vec<AccessibleNode>,
    /// Total non-ignored AX nodes the page exposed (before `max_nodes` capping).
    pub total_ax_nodes: u32,
    /// Page URL the snapshot came from (diagnostics).
    pub page_url: String,
    /// CDP TargetID selected for this snapshot.
    pub target_id: String,
    /// CDP flat-session id attached to the selected target.
    pub session_id: String,
    /// Number of live page targets considered when selecting the tab.
    pub target_candidate_count: u32,
    /// Machine-readable reason this target was selected.
    pub target_selection_reason: String,
    /// Number of frame documents enumerated from the selected page target.
    pub frame_tree_frame_count: u32,
    /// Number of related iframe targets reached through flat-session attachment.
    pub attached_frame_target_count: u32,
    /// Related iframe targets discovered but not surfaced, with explicit reasons.
    pub blocked_frame_targets: Vec<String>,
    /// Non-fatal per-frame snapshot errors. Root-page attach/tree failures still fail loud.
    pub frame_snapshot_errors: Vec<String>,
}

/// Attaches CDP at `endpoint` and maps the selected tab into web nodes.
///
/// The selected tab is an existing live target from `Target.getTargets`. Synapse
/// prefers an exact foreground URL hint, then a unique foreground-title match,
/// and only falls back to the first discovered page when there is no stronger
/// signal. The snapshot pulls `Accessibility.getFullAXTree` and per-node box
/// models, then maps everything into queryable [`AccessibleNode`]s owned by
/// `hwnd`.
///
/// Fail-loud: any attach/tree failure returns an `A11yError` with a specific
/// code (`A11Y_CDP_ATTACH_FAILED` / `A11Y_CDP_AXTREE_FAILED`) — never a silent
/// empty tree.
///
/// # Errors
///
/// Returns [`crate::A11yError::CdpAttachFailed`] when the client cannot connect
/// or no page target exists, and [`crate::A11yError::CdpAxtreeFailed`] when the
/// accessibility tree cannot be retrieved.
#[cfg(windows)]
#[allow(
    clippy::future_not_send,
    clippy::too_many_lines,
    reason = "single CDP attach/read transaction keeps browser handler lifetime explicit"
)]
pub async fn fetch_dom_snapshot(
    endpoint: &str,
    hwnd: i64,
    foreground_title: &str,
    foreground_url_hint: Option<&str>,
    target_id_hint: Option<&str>,
    max_nodes: usize,
) -> crate::A11yResult<CdpDomSnapshot> {
    use chromiumoxide::Browser;
    use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
    use futures_util::StreamExt as _;

    use crate::A11yError;

    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let pages = wait_for_pages(&browser).await?;
        let target_infos = browser
            .execute(GetTargetsParams::default())
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("Target.getTargets: {err}"),
            })?
            .result
            .target_infos;

        let selection = select_existing_page(
            pages,
            &target_infos,
            foreground_title,
            foreground_url_hint,
            target_id_hint,
        )
        .await?;
        let page = selection.page;
        let page_url = page
            .url()
            .await
            .ok()
            .flatten()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| selection.target_url.clone());

        let mut frame_snapshot_errors = Vec::new();
        if let Err(detail) = enable_flat_iframe_auto_attach(&page).await {
            frame_snapshot_errors.push(detail);
        } else {
            // Give the handler one tick to receive Target.attachedToTarget events
            // for already-present child frame targets before we read getTargets.
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        let target_infos = match browser.execute(GetTargetsParams::default()).await {
            Ok(response) => response.result.target_infos,
            Err(error) => {
                frame_snapshot_errors.push(format!(
                    "Target.getTargets after iframe auto-attach failed: {error}; using initial target table"
                ));
                target_infos
            }
        };

        let main_read =
            fetch_target_dom_read(&page, &selection.target_id, max_nodes).await?;
        frame_snapshot_errors.extend(main_read.frame_snapshot_errors.clone());

        let mut nodes = build_accessible_nodes_for_target(
            hwnd,
            Some(&selection.target_id),
            &main_read.dom_nodes,
            max_nodes,
        );
        let mut total_ax_nodes = main_read.total_ax_nodes;
        let mut frame_tree_frame_count = main_read.frame_tree_frame_count;
        let mut attached_frame_target_count = 0_u32;
        let mut blocked_frame_targets = Vec::new();
        let related_iframe_targets = related_iframe_targets(&target_infos, &main_read.frame_ids);
        for target in related_iframe_targets {
            if nodes.len() >= max_nodes {
                blocked_frame_targets.push(format!(
                    "iframe target {} ({}) skipped because max_nodes={} was already reached",
                    target.target_id.inner(),
                    target.url,
                    max_nodes
                ));
                continue;
            }
            let iframe_page = match wait_for_page_target(&browser, target.target_id.clone()).await {
                Ok(page) => page,
                Err(error) => {
                    blocked_frame_targets.push(format!(
                        "iframe target {} parentFrameId={} url={} was discovered in Target.getTargets but did not expose a callable flat session through chromiumoxide: {error}",
                        target.target_id.inner(),
                        target
                            .parent_frame_id
                            .as_ref()
                            .map_or("", |frame_id| frame_id.inner().as_str()),
                        target.url
                    ));
                    continue;
                }
            };
            let remaining = max_nodes.saturating_sub(nodes.len());
            match fetch_target_dom_read(&iframe_page, target.target_id.inner(), remaining).await {
                Ok(read) => {
                    attached_frame_target_count = attached_frame_target_count.saturating_add(1);
                    total_ax_nodes = total_ax_nodes.saturating_add(read.total_ax_nodes);
                    frame_tree_frame_count =
                        frame_tree_frame_count.saturating_add(read.frame_tree_frame_count);
                    frame_snapshot_errors.extend(read.frame_snapshot_errors.clone());
                    nodes.extend(build_accessible_nodes_for_target(
                        hwnd,
                        Some(target.target_id.inner()),
                        &read.dom_nodes,
                        remaining,
                    ));
                }
                Err(error) => {
                    blocked_frame_targets.push(format!(
                        "iframe target {} parentFrameId={} url={} attached but DOM/AX snapshot failed: {error}",
                        target.target_id.inner(),
                        target
                            .parent_frame_id
                            .as_ref()
                            .map_or("", |frame_id| frame_id.inner().as_str()),
                        target.url
                    ));
                }
            }
        }
        Ok(CdpDomSnapshot {
            nodes,
            total_ax_nodes,
            page_url,
            target_id: selection.target_id,
            session_id: selection.session_id,
            target_candidate_count: selection.target_candidate_count,
            target_selection_reason: selection.selection_reason,
            frame_tree_frame_count,
            attached_frame_target_count,
            blocked_frame_targets,
            frame_snapshot_errors,
        })
    }
    .await;

    handler_task.abort();
    result
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct CdpTargetDomRead {
    dom_nodes: Vec<CdpDomNode>,
    total_ax_nodes: u32,
    frame_tree_frame_count: u32,
    frame_ids: Vec<String>,
    frame_snapshot_errors: Vec<String>,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct CdpFrameDescriptor {
    frame_id: Option<chromiumoxide::cdp::browser_protocol::page::FrameId>,
    frame_id_wire: Option<String>,
}

/// Turns on flat (sessionId-routed) auto-attach for child `iframe` targets of
/// `page`. Sharing this with the action layer (`cdp_action`) lets a write/click
/// resolve an out-of-process iframe child target the same way `observe`
/// enumerated it, instead of failing once the element id names a child target.
#[cfg(windows)]
pub(crate) async fn enable_flat_iframe_auto_attach(
    page: &chromiumoxide::Page,
) -> Result<(), String> {
    use chromiumoxide::cdp::browser_protocol::target::{
        FilterEntry, SetAutoAttachParams, TargetFilter,
    };

    let filter = TargetFilter::new(vec![
        FilterEntry::builder()
            .r#type("iframe")
            .exclude(false)
            .build(),
    ]);
    let params = SetAutoAttachParams::builder()
        .auto_attach(true)
        .wait_for_debugger_on_start(false)
        .flatten(true)
        .filter(filter)
        .build()
        .map_err(|error| format!("Target.setAutoAttach iframe params: {error}"))?;
    page.execute(params)
        .await
        .map_err(|error| format!("Target.setAutoAttach iframe flatten=true failed: {error}"))?;
    Ok(())
}

#[cfg(windows)]
async fn fetch_target_dom_read(
    page: &chromiumoxide::Page,
    target_id: &str,
    max_nodes: usize,
) -> crate::A11yResult<CdpTargetDomRead> {
    use std::collections::{HashMap, HashSet};

    use chromiumoxide::cdp::browser_protocol::accessibility::{EnableParams, GetFullAxTreeParams};
    use chromiumoxide::cdp::browser_protocol::dom::{
        BackendNodeId, GetBoxModelParams, GetDocumentParams,
    };

    use crate::A11yError;

    page.execute(EnableParams::default())
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("target {target_id} Accessibility.enable: {err}"),
        })?;

    let mut frame_snapshot_errors = Vec::new();
    let prime = GetDocumentParams::builder().depth(-1).pierce(true).build();
    if let Err(error) = page.execute(prime).await {
        frame_snapshot_errors.push(format!(
            "target {target_id} DOM.getDocument depth=-1 pierce=true failed: {error}"
        ));
    }

    let frames = frame_descriptors_for_page(page, target_id, &mut frame_snapshot_errors).await;
    let mut dom_nodes = Vec::new();
    let mut total_ax_nodes = 0_u32;
    let mut first_frame_error = None;
    let mut seen_backends = HashSet::new();

    for frame in &frames {
        let params = frame
            .frame_id
            .clone()
            .map_or_else(GetFullAxTreeParams::default, |frame_id| {
                GetFullAxTreeParams::builder().frame_id(frame_id).build()
            });
        let tree = match page.execute(params).await {
            Ok(tree) => tree,
            Err(error) => {
                let detail = format!(
                    "target {target_id} frame {} Accessibility.getFullAXTree failed: {error}",
                    frame.frame_id_wire.as_deref().unwrap_or("<root>")
                );
                if first_frame_error.is_none() {
                    first_frame_error = Some(detail.clone());
                }
                frame_snapshot_errors.push(detail);
                continue;
            }
        };

        let ax_nodes = &tree.result.nodes;
        let by_ax_id: HashMap<&str, &_> = ax_nodes
            .iter()
            .map(|node| (node.node_id.inner().as_str(), node))
            .collect();

        for node in ax_nodes {
            if node.ignored {
                continue;
            }
            total_ax_nodes = total_ax_nodes.saturating_add(1);
            let Some(backend) = node.backend_dom_node_id.as_ref().map(|id| *id.inner()) else {
                continue;
            };
            if !seen_backends.insert(backend) {
                continue;
            }
            let role = ax_value_string(node.role.as_ref());
            if role.is_empty() {
                continue;
            }
            let name = ax_value_string(node.name.as_ref());
            let value = {
                let value = ax_value_string(node.value.as_ref());
                (!value.is_empty()).then_some(value)
            };
            let parent_backend = nearest_backend_ancestor(node, &by_ax_id);
            let child_count = node
                .child_ids
                .as_ref()
                .map_or(0, |ids| u32::try_from(ids.len()).unwrap_or(u32::MAX));

            let bbox = if dom_nodes.len() < max_nodes {
                let params = GetBoxModelParams::builder()
                    .backend_node_id(BackendNodeId::new(backend))
                    .build();
                page.execute(params)
                    .await
                    .ok()
                    .and_then(|response| rect_from_quad(response.result.model.content.inner()))
            } else {
                None
            };
            let frame_id = frame.frame_id_wire.clone();

            dom_nodes.push(CdpDomNode {
                backend_node_id: backend,
                parent_backend_node_id: parent_backend,
                role,
                name,
                value,
                frame_id,
                bbox,
                child_count,
                enabled: true,
                focused: false,
            });
        }
    }

    if dom_nodes.is_empty()
        && let Some(detail) = first_frame_error
    {
        return Err(A11yError::CdpAxtreeFailed { detail });
    }

    Ok(CdpTargetDomRead {
        dom_nodes,
        total_ax_nodes,
        frame_tree_frame_count: u32::try_from(frames.len()).unwrap_or(u32::MAX),
        frame_ids: frames
            .iter()
            .filter_map(|frame| frame.frame_id_wire.clone())
            .collect(),
        frame_snapshot_errors,
    })
}

#[cfg(windows)]
async fn frame_descriptors_for_page(
    page: &chromiumoxide::Page,
    target_id: &str,
    frame_snapshot_errors: &mut Vec<String>,
) -> Vec<CdpFrameDescriptor> {
    use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;

    match page.execute(GetFrameTreeParams::default()).await {
        Ok(response) => {
            let mut frames = Vec::new();
            collect_frame_descriptors(&response.result.frame_tree, &mut frames);
            if frames.is_empty() {
                frame_snapshot_errors.push(format!(
                    "target {target_id} Page.getFrameTree returned zero frames; falling back to root AX tree"
                ));
                vec![CdpFrameDescriptor {
                    frame_id: None,
                    frame_id_wire: None,
                }]
            } else {
                frames
            }
        }
        Err(error) => {
            frame_snapshot_errors.push(format!(
                "target {target_id} Page.getFrameTree failed: {error}; falling back to root AX tree"
            ));
            vec![CdpFrameDescriptor {
                frame_id: None,
                frame_id_wire: None,
            }]
        }
    }
}

#[cfg(windows)]
fn collect_frame_descriptors(
    frame_tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
    out: &mut Vec<CdpFrameDescriptor>,
) {
    out.push(CdpFrameDescriptor {
        frame_id: Some(frame_tree.frame.id.clone()),
        frame_id_wire: Some(frame_tree.frame.id.inner().clone()),
    });
    if let Some(children) = frame_tree.child_frames.as_ref() {
        for child in children {
            collect_frame_descriptors(child, out);
        }
    }
}

#[cfg(windows)]
fn related_iframe_targets<'a>(
    target_infos: &'a [chromiumoxide::cdp::browser_protocol::target::TargetInfo],
    selected_frame_ids: &[String],
) -> Vec<&'a chromiumoxide::cdp::browser_protocol::target::TargetInfo> {
    target_infos
        .iter()
        .filter(|target| target.r#type == "iframe")
        .filter(|target| {
            target
                .parent_frame_id
                .as_ref()
                .is_some_and(|frame_id| selected_frame_ids.iter().any(|id| id == frame_id.inner()))
        })
        .collect()
}

#[cfg(windows)]
async fn wait_for_page_target(
    browser: &chromiumoxide::Browser,
    target_id: chromiumoxide::cdp::browser_protocol::target::TargetId,
) -> Result<chromiumoxide::Page, String> {
    let mut last_error = None;
    for _ in 0..10 {
        match browser.get_page(target_id.clone()).await {
            Ok(page) => return Ok(page),
            Err(error) => {
                last_error = Some(error.to_string());
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "target was not present".to_owned()))
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct ExistingPageCandidate {
    page: chromiumoxide::Page,
    target_id: String,
    session_id: String,
    title: String,
    url: String,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct ExistingPageSelection {
    page: chromiumoxide::Page,
    target_id: String,
    session_id: String,
    target_url: String,
    target_candidate_count: u32,
    selection_reason: String,
}

#[cfg(windows)]
async fn select_existing_page(
    pages: Vec<chromiumoxide::Page>,
    target_infos: &[chromiumoxide::cdp::browser_protocol::target::TargetInfo],
    foreground_title: &str,
    foreground_url_hint: Option<&str>,
    target_id_hint: Option<&str>,
) -> crate::A11yResult<ExistingPageSelection> {
    use std::collections::HashMap;

    use crate::A11yError;

    let page_targets: HashMap<&str, _> = target_infos
        .iter()
        .filter(|info| info.r#type == "page")
        .map(|info| (info.target_id.as_ref(), info))
        .collect();
    let mut candidates = Vec::new();
    for page in pages {
        let target_id = page.target_id().inner().clone();
        let info = page_targets.get(target_id.as_str()).copied();
        let title = if let Some(info) = info
            && !info.title.is_empty()
        {
            info.title.clone()
        } else {
            page.get_title().await.ok().flatten().unwrap_or_default()
        };
        let url = if let Some(info) = info
            && !info.url.is_empty()
        {
            info.url.clone()
        } else {
            page.url().await.ok().flatten().unwrap_or_default()
        };
        let session_id = page.session_id().inner().clone();
        candidates.push(ExistingPageCandidate {
            page,
            target_id,
            session_id,
            title,
            url,
        });
    }

    if candidates.is_empty() {
        return Err(A11yError::CdpAttachFailed {
            detail: "Target.getTargets/page discovery found no existing page targets".to_owned(),
        });
    }

    let target_candidate_count = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
    if let Some(target_id_hint) = target_id_hint.filter(|hint| !hint.trim().is_empty()) {
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.target_id == target_id_hint)
        {
            return Ok(selection(
                candidate,
                target_candidate_count,
                "target_id_hint",
            ));
        }
        return Err(A11yError::CdpAxtreeFailed {
            detail: format!(
                "explicit CDP target_id hint {target_id_hint:?} was not discovered among page targets: {}",
                candidates
                    .iter()
                    .map(|candidate| candidate.target_id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        });
    }

    if let Some(url_hint) = foreground_url_hint.filter(|hint| !hint.trim().is_empty()) {
        let url_matches: Vec<_> = candidates
            .iter()
            .filter(|candidate| url_matches_hint(&candidate.url, url_hint))
            .collect();
        match url_matches.as_slice() {
            [candidate] => {
                return Ok(selection(candidate, target_candidate_count, "url_hint"));
            }
            [] => {}
            matches => {
                return Err(A11yError::CdpAxtreeFailed {
                    detail: format!(
                        "ambiguous CDP target selection for foreground URL hint {url_hint:?}: {} matching page targets",
                        matches.len()
                    ),
                });
            }
        }
    }

    let title_matches: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            !candidate.title.is_empty() && foreground_title.contains(candidate.title.as_str())
        })
        .collect();
    match title_matches.as_slice() {
        [candidate] => Ok(selection(
            candidate,
            target_candidate_count,
            "foreground_title",
        )),
        [] => Ok(selection(
            &candidates[0],
            target_candidate_count,
            "fallback_first_page",
        )),
        matches => Err(A11yError::CdpAxtreeFailed {
            detail: format!(
                "ambiguous CDP target selection for foreground title {foreground_title:?}: {} matching page targets",
                matches.len()
            ),
        }),
    }
}

#[cfg(windows)]
fn selection(
    candidate: &ExistingPageCandidate,
    target_candidate_count: u32,
    selection_reason: &str,
) -> ExistingPageSelection {
    ExistingPageSelection {
        page: candidate.page.clone(),
        target_id: candidate.target_id.clone(),
        session_id: candidate.session_id.clone(),
        target_url: candidate.url.clone(),
        target_candidate_count,
        selection_reason: selection_reason.to_owned(),
    }
}

#[cfg(windows)]
fn url_matches_hint(candidate_url: &str, hint: &str) -> bool {
    let candidate = candidate_url.trim();
    let hint = hint.trim();
    if candidate.eq_ignore_ascii_case(hint) {
        return true;
    }
    trim_trailing_url_slash(candidate).eq_ignore_ascii_case(trim_trailing_url_slash(hint))
}

#[cfg(windows)]
fn trim_trailing_url_slash(value: &str) -> &str {
    value.strip_suffix('/').unwrap_or(value)
}

#[cfg(windows)]
async fn wait_for_pages(
    browser: &chromiumoxide::Browser,
) -> crate::A11yResult<Vec<chromiumoxide::Page>> {
    use crate::A11yError;

    for _ in 0..30 {
        match browser.pages().await {
            Ok(pages) if !pages.is_empty() => return Ok(pages),
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            Err(err) => {
                return Err(A11yError::CdpAttachFailed {
                    detail: format!("list pages: {err}"),
                });
            }
        }
    }
    Err(A11yError::CdpAttachFailed {
        detail: "no page targets became available within 3s".to_owned(),
    })
}

/// Extracts the string value of a CDP `AxValue` (role/name/value), empty if none.
#[cfg(windows)]
fn ax_value_string(
    value: Option<&chromiumoxide::cdp::browser_protocol::accessibility::AxValue>,
) -> String {
    value
        .and_then(|value| value.value.as_ref())
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Walks an AX node's `parentId` chain to the first ancestor that has a backend
/// DOM node id, so the mapped tree stays connected even across generic AX nodes.
#[cfg(windows)]
fn nearest_backend_ancestor(
    node: &chromiumoxide::cdp::browser_protocol::accessibility::AxNode,
    by_ax_id: &std::collections::HashMap<
        &str,
        &chromiumoxide::cdp::browser_protocol::accessibility::AxNode,
    >,
) -> Option<i64> {
    let mut current = node.parent_id.as_ref()?.inner().as_str();
    for _ in 0..256 {
        let parent = by_ax_id.get(current)?;
        if let Some(backend) = parent.backend_dom_node_id.as_ref() {
            return Some(*backend.inner());
        }
        current = parent.parent_id.as_ref()?.inner().as_str();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_element_id_round_trips_backend_node_id() {
        let hwnd = 0x000A_BCDE;
        let id = cdp_element_id(hwnd, 42);
        println!("readback=cdp_element_id before=backend:42 after=id:{id}");
        assert!(id.to_string().contains(":cdcd"));
        assert_eq!(cdp_backend_from_element_id(&id), Some(42));
        assert_eq!(cdp_target_from_element_id(&id), None);
    }

    #[test]
    fn target_aware_cdp_element_id_round_trips_target_and_backend() {
        let hwnd = 0x000A_BCDE;
        let target_id = "1940BBDE7A3BE4F3E5CDA16A112E8CAC";
        let id = cdp_element_id_for_target(hwnd, target_id, 42);
        println!("readback=cdp_element_id target={target_id} backend=42 after=id:{id}");
        assert_eq!(cdp_target_from_element_id(&id).as_deref(), Some(target_id));
        assert_eq!(cdp_backend_from_element_id(&id), Some(42));
    }

    #[test]
    fn uia_element_ids_are_not_mistaken_for_cdp() {
        let uia = element_id(0x1234, "0000002a00000001");
        println!(
            "readback=cdp_detect edge=uia id:{uia} backend:{:?}",
            cdp_backend_from_element_id(&uia)
        );
        assert_eq!(cdp_backend_from_element_id(&uia), None);
    }

    #[test]
    fn rect_from_quad_computes_axis_aligned_bounds() {
        // The real Apply-button quad observed against Chrome 149 in FSV.
        let quad = [
            16.0, 69.4375, 49.359, 69.4375, 49.359, 84.4375, 16.0, 84.4375,
        ];
        let rect = rect_from_quad(&quad).expect("valid quad yields a rect");
        println!("readback=rect_from_quad before=quad:{quad:?} after=rect:{rect:?}");
        assert_eq!(rect.x, 16);
        assert_eq!(rect.y, 69);
        assert_eq!(rect.w, 33);
        assert_eq!(rect.h, 15);
    }

    #[test]
    fn rect_from_quad_rejects_short_quad() {
        assert_eq!(rect_from_quad(&[1.0, 2.0, 3.0]), None);
    }

    #[test]
    fn build_accessible_nodes_assigns_depth_and_ids() {
        let nodes = vec![
            CdpDomNode {
                backend_node_id: 1,
                parent_backend_node_id: None,
                role: "RootWebArea".to_owned(),
                name: "Apply to YC".to_owned(),
                value: None,
                frame_id: None,
                bbox: Some(Rect {
                    x: 0,
                    y: 0,
                    w: 1600,
                    h: 900,
                }),
                child_count: 1,
                enabled: true,
                focused: false,
            },
            CdpDomNode {
                backend_node_id: 6,
                parent_backend_node_id: Some(1),
                role: "button".to_owned(),
                name: "Apply".to_owned(),
                value: None,
                frame_id: None,
                bbox: Some(Rect {
                    x: 16,
                    y: 69,
                    w: 33,
                    h: 15,
                }),
                child_count: 0,
                enabled: true,
                focused: true,
            },
        ];
        let mapped = build_accessible_nodes(0x2200, &nodes, 60);
        println!(
            "readback=build_nodes after=count:{} roles:{:?}",
            mapped.len(),
            mapped
                .iter()
                .map(|node| (node.role.clone(), node.depth))
                .collect::<Vec<_>>()
        );
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].depth, 0);
        assert_eq!(mapped[1].depth, 1);
        assert_eq!(mapped[1].role, "button");
        assert_eq!(mapped[1].name, "Apply");
        // Child's parent id is the root's cdp id, enabling tree navigation.
        assert_eq!(mapped[1].parent.as_ref(), Some(&cdp_element_id(0x2200, 1)));
        // Backend id recovers from the mapped element id (action routing #686).
        assert_eq!(cdp_backend_from_element_id(&mapped[1].element_id), Some(6));
    }

    #[test]
    fn build_accessible_nodes_for_target_embeds_target_in_child_and_parent_ids() {
        let target_id = "7F8969F3FC3DCB527D0658F63027FF3E";
        let nodes = vec![
            CdpDomNode {
                backend_node_id: 1,
                parent_backend_node_id: None,
                role: "RootWebArea".to_owned(),
                name: "Tab".to_owned(),
                value: None,
                frame_id: None,
                bbox: None,
                child_count: 1,
                enabled: true,
                focused: false,
            },
            CdpDomNode {
                backend_node_id: 6,
                parent_backend_node_id: Some(1),
                role: "button".to_owned(),
                name: "Apply".to_owned(),
                value: None,
                frame_id: None,
                bbox: None,
                child_count: 0,
                enabled: true,
                focused: false,
            },
        ];
        let mapped = build_accessible_nodes_for_target(0x2200, Some(target_id), &nodes, 60);
        println!(
            "readback=build_nodes_target after=child:{} parent:{:?}",
            mapped[1].element_id, mapped[1].parent
        );
        assert_eq!(
            cdp_target_from_element_id(&mapped[1].element_id).as_deref(),
            Some(target_id)
        );
        assert_eq!(
            mapped[1]
                .parent
                .as_ref()
                .and_then(cdp_target_from_element_id)
                .as_deref(),
            Some(target_id)
        );
        assert_eq!(cdp_backend_from_element_id(&mapped[1].element_id), Some(6));
    }

    #[test]
    fn build_accessible_nodes_caps_at_max() {
        let nodes: Vec<CdpDomNode> = (0..10)
            .map(|index| CdpDomNode {
                backend_node_id: index,
                parent_backend_node_id: None,
                role: "link".to_owned(),
                name: format!("link-{index}"),
                value: None,
                frame_id: None,
                bbox: None,
                child_count: 0,
                enabled: true,
                focused: false,
            })
            .collect();
        let mapped = build_accessible_nodes(0x10, &nodes, 4);
        println!(
            "readback=build_nodes_cap before=in:10 max:4 after=out:{}",
            mapped.len()
        );
        assert_eq!(mapped.len(), 4);
    }
}
