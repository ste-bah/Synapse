//! CDP-routed actions on web DOM nodes (#686).
//!
//! When an action targets a web node (an element id carrying the
//! [`crate::CDP_RUNTIME_PREFIX`] sentinel), the action layer routes it here
//! instead of UIA/`SendInput`. We attach CDP, locate the page that owns the
//! node, scroll it into view, resolve its live box model, and dispatch via
//! `Input.dispatchMouseEvent` / `Input.insertText` in **viewport CSS
//! coordinates** — which sidesteps the DPI / scroll / window-content-origin
//! mapping that screen-coordinate clicking would need, and works regardless of
//! the node's initial scroll position.
//!
//! Everything here is `cfg(windows)` because it depends on `chromiumoxide`.

#![cfg(windows)]

use std::time::Duration;

use chromiumoxide::Browser;
use chromiumoxide::cdp::browser_protocol::dom::{
    BackendNodeId, GetBoxModelParams, ResolveNodeParams, ScrollIntoViewIfNeededParams,
};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, GetLayoutMetricsParams, GetNavigationHistoryParams, NavigateParams,
    NavigateToHistoryEntryParams, ReloadParams, Viewport,
};
use chromiumoxide::cdp::browser_protocol::target::TargetId;
use chromiumoxide::cdp::js_protocol::runtime::{CallArgument, CallFunctionOnParams};
use chromiumoxide::page::ScreenshotParams;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;

use crate::{A11yError, A11yResult, cdp_dom::rect_from_quad};

const CDP_INPUT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Where a CDP action landed, in viewport CSS coordinates (diagnostics).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CdpActionPoint {
    pub x: f64,
    pub y: f64,
}

/// One point in a CDP mouse stroke, in viewport CSS coordinates.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CdpMouseStrokePoint {
    pub x: f64,
    pub y: f64,
    pub elapsed_ms: f64,
}

/// Dispatch summary for a CDP mouse stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct CdpMouseStrokeResult {
    pub target_id: String,
    pub point_count: usize,
    pub start: CdpActionPoint,
    pub end: CdpActionPoint,
    pub duration_ms: f64,
}

/// One CDP wheel event in viewport CSS pixels.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CdpWheelDelta {
    pub delta_x: f64,
    pub delta_y: f64,
}

/// One key descriptor for `Input.dispatchKeyEvent`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CdpKeyStroke {
    pub key: String,
    pub code: String,
    pub windows_virtual_key_code: i64,
    pub native_virtual_key_code: i64,
    pub key_identifier: Option<String>,
    pub text: Option<String>,
    pub unmodified_text: Option<String>,
    pub modifier_bit: i64,
    pub location: Option<i64>,
}

/// Scroll source-of-truth read from the target node's DOM context.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CdpScrollState {
    pub is_connected: bool,
    pub window_scroll_x: f64,
    pub window_scroll_y: f64,
    pub target_scroll_left: f64,
    pub target_scroll_top: f64,
    pub target_scroll_width: f64,
    pub target_scroll_height: f64,
    pub target_client_width: f64,
    pub target_client_height: f64,
    pub target_tag: String,
    pub target_id: String,
    pub node_rect_left: f64,
    pub node_rect_top: f64,
    pub node_rect_width: f64,
    pub node_rect_height: f64,
}

/// Active-element Source-of-Truth read from a CDP page target.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CdpActiveElementState {
    pub target_id: String,
    pub has_active_element: bool,
    pub is_editable: bool,
    pub tag_name: String,
    pub id: String,
    pub name: String,
    pub value: String,
    pub selection_start: Option<u32>,
    pub selection_end: Option<u32>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CdpPageNavigationAction {
    Navigate,
    Reload,
    Back,
    Forward,
}

impl CdpPageNavigationAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Navigate => "navigate",
            Self::Reload => "reload",
            Self::Back => "back",
            Self::Forward => "forward",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CdpPageState {
    pub url: String,
    pub title: String,
    pub ready_state: String,
    pub history_current_index: i64,
    pub history_entry_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CdpPageNavigationResult {
    pub target_id: String,
    pub action: String,
    pub requested_url: Option<String>,
    pub before: CdpPageState,
    pub after: CdpPageState,
    pub navigation_error_text: Option<String>,
    pub is_download: Option<bool>,
}

#[derive(Clone, Debug)]
enum CdpPageReadbackExpectation {
    Stable,
    UrlChanged { previous_url: String },
    HistoryEntry { current_index: i64, url: String },
}

impl CdpPageReadbackExpectation {
    fn matches(&self, state: &CdpPageState) -> bool {
        match self {
            Self::Stable => true,
            Self::UrlChanged { previous_url } => state.url != *previous_url,
            Self::HistoryEntry { current_index, url } => {
                state.history_current_index == *current_index && state.url == *url
            }
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::Stable => "stable loaded page".to_owned(),
            Self::UrlChanged { previous_url } => {
                format!("url to change from {previous_url:?}")
            }
            Self::HistoryEntry { current_index, url } => {
                format!("historyIndex={current_index} and url={url:?}")
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct CdpRuntimeScrollDispatch {
    is_connected: bool,
    default_prevented: bool,
    target_scroll_left_before: f64,
    target_scroll_top_before: f64,
    target_scroll_left_after: f64,
    target_scroll_top_after: f64,
    target_tag: String,
    target_id: String,
}

/// Which pointer button a CDP click uses.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CdpMouseButton {
    Left,
    Right,
    Middle,
}

impl CdpMouseButton {
    const fn to_cdp(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

/// Clicks a web node `click_count` times with `button`, after scrolling it into
/// view. Returns the viewport point clicked.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/node cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if box-model resolution or dispatch fails.
pub async fn cdp_click_node(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    button: CdpMouseButton,
    click_count: i64,
) -> A11yResult<CdpActionPoint> {
    with_node_center(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page, center| async move {
            page.execute(mouse_event(
                DispatchMouseEventType::MouseMoved,
                center,
                button.to_cdp(),
                0,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            page.execute(mouse_event(
                DispatchMouseEventType::MousePressed,
                center,
                button.to_cdp(),
                click_count.max(1),
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            page.execute(mouse_event(
                DispatchMouseEventType::MouseReleased,
                center,
                button.to_cdp(),
                click_count.max(1),
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            Ok(center)
        },
    )
    .await
}

/// Focuses a web input node and inserts `text` (as if typed).
///
/// # Errors
///
/// As [`cdp_click_node`].
pub async fn cdp_type_node(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    text: &str,
) -> A11yResult<()> {
    use chromiumoxide::cdp::browser_protocol::dom::FocusParams;

    let text = text.to_owned();
    with_node_center(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page, center| async move {
            // Click to place the caret, then focus and insert text.
            page.execute(mouse_event(
                DispatchMouseEventType::MousePressed,
                center,
                MouseButton::Left,
                1,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            page.execute(mouse_event(
                DispatchMouseEventType::MouseReleased,
                center,
                MouseButton::Left,
                1,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            // The click above already places focus/caret in the field. DOM.focus is
            // a best-effort reinforcement — some nodes (e.g. the AX node maps to a
            // non-focusable wrapper) report "not focusable", which must not abort the
            // insert when the click already focused the input.
            let focus = FocusParams::builder()
                .backend_node_id(BackendNodeId::new(backend_node_id))
                .build();
            let _ = page.execute(focus).await;
            page.execute(InsertTextParams::new(text))
                .await
                .map_err(|err| dispatch_err(&err))?;
            Ok(center)
        },
    )
    .await
    .map(|_point| ())
}

/// Readback for [`cdp_set_node_text`]: which selection strategy applied before
/// the replace, and whether an empty replacement was delivered as a Delete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CdpSetNodeTextReadback {
    /// `selected_value` (input/textarea `select()`) or
    /// `selected_contenteditable` (DOM range over the editable host).
    pub selection_mode: String,
    /// True when `text` was empty and the selection was removed with a
    /// synthesized Delete key instead of `Input.insertText`.
    pub cleared_with_delete: bool,
}

/// JS run on the resolved node to select its full content before the replace.
/// Mirrors the Playwright `fill` strategy: value controls use `select()`, a
/// contenteditable host gets a DOM range over its contents. Returns a wire
/// string so a non-editable target fails loud instead of appending.
const CDP_SELECT_ALL_FUNCTION: &str = r#"function() {
    if (this === null || this === undefined || !this.isConnected) { return 'detached'; }
    if (typeof this.select === 'function' && ('value' in this) && !this.disabled && !this.readOnly) {
        this.focus();
        this.select();
        return 'selected_value';
    }
    if (this.isContentEditable) {
        this.focus();
        const range = this.ownerDocument.createRange();
        range.selectNodeContents(this);
        const selection = this.ownerDocument.defaultView.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
        return 'selected_contenteditable';
    }
    if (('value' in this) && (this.disabled || this.readOnly)) { return 'not_editable_disabled_or_readonly'; }
    return 'not_editable';
}"#;

/// Replaces a web editable node's full text content (#882): real click to
/// place focus/caret, best-effort `DOM.focus`, select-all on the exact
/// resolved node, then `Input.insertText` — which replaces the active
/// selection, the same strategy Playwright `fill()` uses. Empty `text`
/// removes the selection with a synthesized Delete key.
///
/// Fail-loud: a target that is neither a value control nor contenteditable
/// returns `A11Y_CDP_AXTREE_FAILED` naming the select-all readback — there is
/// no append fallback. Callers must verify with a separate
/// [`cdp_node_value`] readback.
///
/// # Errors
///
/// As [`cdp_click_node`], plus `A11Y_CDP_AXTREE_FAILED` for non-editable
/// targets.
pub async fn cdp_set_node_text(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    text: &str,
) -> A11yResult<CdpSetNodeTextReadback> {
    use chromiumoxide::cdp::browser_protocol::dom::FocusParams;

    let text = text.to_owned();
    with_node_center(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page, center| async move {
            page.execute(mouse_event(
                DispatchMouseEventType::MousePressed,
                center,
                MouseButton::Left,
                1,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            page.execute(mouse_event(
                DispatchMouseEventType::MouseReleased,
                center,
                MouseButton::Left,
                1,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            // The click already places focus; DOM.focus is best-effort
            // reinforcement (some AX nodes map to non-focusable wrappers).
            let focus = FocusParams::builder()
                .backend_node_id(BackendNodeId::new(backend_node_id))
                .build();
            let _ = page.execute(focus).await;

            let resolve = ResolveNodeParams::builder()
                .backend_node_id(BackendNodeId::new(backend_node_id))
                .object_group("synapse_set_field_text")
                .build();
            let resolved =
                page.execute(resolve)
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("resolveNode for backendNodeId {backend_node_id}: {err}"),
                    })?;
            let object_id =
                resolved
                    .object
                    .object_id
                    .clone()
                    .ok_or_else(|| A11yError::CdpAxtreeFailed {
                        detail: format!(
                            "resolveNode for backendNodeId {backend_node_id} returned no objectId"
                        ),
                    })?;
            let call = CallFunctionOnParams::builder()
                .function_declaration(CDP_SELECT_ALL_FUNCTION)
                .object_id(object_id)
                .return_by_value(true)
                .silent(true)
                .build()
                .map_err(|err| A11yError::CdpAxtreeFailed {
                    detail: format!("build Runtime.callFunctionOn select-all params: {err}"),
                })?;
            let selection_mode: String = call_function_on_value(&page, call, "select-all").await?;
            if !matches!(
                selection_mode.as_str(),
                "selected_value" | "selected_contenteditable"
            ) {
                return Err(A11yError::CdpAxtreeFailed {
                    detail: format!(
                        "cdp_set_node_text refused backendNodeId {backend_node_id}: target is not an editable web node (select-all readback: {selection_mode})"
                    ),
                });
            }

            let cleared_with_delete = text.is_empty();
            if cleared_with_delete {
                let delete = CdpKeyStroke {
                    key: "Delete".to_owned(),
                    code: "Delete".to_owned(),
                    windows_virtual_key_code: 46,
                    native_virtual_key_code: 46,
                    key_identifier: None,
                    text: None,
                    unmodified_text: None,
                    modifier_bit: 0,
                    location: None,
                };
                page.execute(cdp_key_event(DispatchKeyEventType::KeyDown, &delete, 0)?)
                    .await
                    .map_err(|err| dispatch_err(&err))?;
                page.execute(cdp_key_event(DispatchKeyEventType::KeyUp, &delete, 0)?)
                    .await
                    .map_err(|err| dispatch_err(&err))?;
            } else {
                page.execute(InsertTextParams::new(text))
                    .await
                    .map_err(|err| dispatch_err(&err))?;
            }
            Ok(CdpSetNodeTextReadback {
                selection_mode,
                cleared_with_delete,
            })
        },
    )
    .await
}

/// Dispatches a key sequence to a specific CDP page target without activating
/// the browser window.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/target cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if `Input.dispatchKeyEvent` fails.
pub async fn cdp_press_key_sequence(
    endpoint: &str,
    target_id: &str,
    keys: Vec<CdpKeyStroke>,
    hold_ms: u32,
) -> A11yResult<()> {
    if keys.is_empty() {
        return Err(A11yError::CdpAxtreeFailed {
            detail: "cdp_press_key_sequence requires at least one key".to_owned(),
        });
    }
    with_target_page(endpoint, target_id, |page| async move {
        let mut modifiers = 0_i64;
        for key in &keys {
            let key_down_type = if key.text.is_some() {
                DispatchKeyEventType::KeyDown
            } else {
                DispatchKeyEventType::RawKeyDown
            };
            let event_modifiers = modifiers | key.modifier_bit;
            page.execute(cdp_key_event(key_down_type, key, event_modifiers)?)
                .await
                .map_err(|err| dispatch_err(&err))?;
            modifiers = event_modifiers;
        }
        if hold_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(u64::from(hold_ms))).await;
        }
        for key in keys.iter().rev() {
            if key.modifier_bit != 0 {
                modifiers &= !key.modifier_bit;
            }
            page.execute(cdp_key_event(DispatchKeyEventType::KeyUp, key, modifiers)?)
                .await
                .map_err(|err| dispatch_err(&err))?;
        }
        Ok(())
    })
    .await
}

/// Dispatches a viewport-CSS mouse move/drag path to one CDP page target
/// without moving the OS cursor or activating the browser window.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/target cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if the stroke is invalid or CDP dispatch fails.
pub async fn cdp_mouse_stroke_target(
    endpoint: &str,
    target_id: &str,
    points: Vec<CdpMouseStrokePoint>,
    button: Option<CdpMouseButton>,
) -> A11yResult<CdpMouseStrokeResult> {
    validate_cdp_mouse_stroke_points(&points)?;
    let start = cdp_stroke_action_point(points[0]);
    let end = points
        .last()
        .map_or(start, |point| cdp_stroke_action_point(*point));
    let duration_ms = points.last().map_or(0.0, |point| point.elapsed_ms.max(0.0));
    let button = button
        .map(CdpMouseButton::to_cdp)
        .unwrap_or(MouseButton::None);
    with_target_page(endpoint, target_id, |page| async move {
        let dispatched_target_id = page.target_id().inner().clone();
        dispatch_cdp_mouse_stroke(&page, &points, button).await?;
        Ok(CdpMouseStrokeResult {
            target_id: dispatched_target_id,
            point_count: points.len(),
            start,
            end,
            duration_ms,
        })
    })
    .await
}

/// Reads the target page's active DOM element, value, and selection without
/// activating the browser window.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/target cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if the DOM readback fails.
pub async fn cdp_active_element_state(
    endpoint: &str,
    target_id: &str,
) -> A11yResult<CdpActiveElementState> {
    let target_id = target_id.to_owned();
    let target_id_for_lookup = target_id.clone();
    with_target_page(endpoint, &target_id_for_lookup, |page| async move {
        let expression = format!(
            r#"(() => {{
                const el = document.activeElement;
                if (!el) {{
                    return {{
                        target_id: {target_id_json},
                        has_active_element: false,
                        is_editable: false,
                        tag_name: "",
                        id: "",
                        name: "",
                        value: "",
                        selection_start: null,
                        selection_end: null
                    }};
                }}
                const tagName = String(el.tagName || "");
                const tag = tagName.toUpperCase();
                const inputType = String(el.getAttribute("type") || "text").toLowerCase();
                const textInputTypes = new Set([
                    "text", "search", "url", "tel", "email", "password", "number",
                    "date", "datetime-local", "month", "time", "week", "color"
                ]);
                const ariaDisabled = String(el.getAttribute("aria-disabled") || "").toLowerCase() === "true";
                const isDisabled = Boolean(el.disabled) || ariaDisabled;
                const isReadOnly = Boolean(el.readOnly);
                const isEditable =
                    (tag === "TEXTAREA" && !isDisabled && !isReadOnly) ||
                    (tag === "INPUT" && textInputTypes.has(inputType) && !isDisabled && !isReadOnly) ||
                    (Boolean(el.isContentEditable) && !isDisabled) ||
                    (String(el.getAttribute("role") || "").toLowerCase() === "textbox" && !isDisabled);
                const value = ("value" in el)
                    ? String(el.value ?? "")
                    : String(el.textContent ?? "");
                const selectionStart = (typeof el.selectionStart === "number")
                    ? el.selectionStart
                    : null;
                const selectionEnd = (typeof el.selectionEnd === "number")
                    ? el.selectionEnd
                    : null;
                return {{
                    target_id: {target_id_json},
                    has_active_element: true,
                    is_editable: isEditable,
                    tag_name: tagName,
                    id: String(el.id || ""),
                    name: String(el.getAttribute("name") || ""),
                    value,
                    selection_start: selectionStart,
                    selection_end: selectionEnd
                }};
            }})()"#,
            target_id_json =
                serde_json::to_string(&target_id).unwrap_or_else(|_| "\"\"".to_owned())
        );
        page.evaluate_expression(expression)
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Runtime.evaluate active-element readback: {err}"),
            })?
            .into_value::<CdpActiveElementState>()
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("Runtime.evaluate active-element decode: {err}"),
            })
    })
    .await
}

/// Navigates/reloads/history-navigates a specific CDP page target and returns a
/// separate post-command DOM/history readback for the same target.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/target cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if the Page command fails, reports a navigation
/// error, history has no requested entry, or the readback never reaches a
/// stable loaded state within `wait_timeout_ms`.
pub async fn cdp_navigate_page_target(
    endpoint: &str,
    target_id: &str,
    action: CdpPageNavigationAction,
    url: Option<&str>,
    wait_timeout_ms: u64,
    ignore_cache: bool,
) -> A11yResult<CdpPageNavigationResult> {
    let requested_url = url.map(ToOwned::to_owned);
    with_target_page(endpoint, target_id, |page| async move {
        let target_id = page.target_id().inner().clone();
        let before = read_page_state(&page).await?;
        let mut navigation_error_text = None;
        let mut is_download = None;
        let mut readback_expectation = CdpPageReadbackExpectation::Stable;
        match action {
            CdpPageNavigationAction::Navigate => {
                let Some(url) = requested_url.as_deref() else {
                    return Err(A11yError::CdpAxtreeFailed {
                        detail: "Page.navigate requires a URL".to_owned(),
                    });
                };
                let navigated = page
                    .execute(NavigateParams::new(url.to_owned()))
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("Page.navigate({url:?}): {err}"),
                    })?;
                navigation_error_text = navigated.result.error_text.clone();
                is_download = navigated.result.is_download;
                if navigation_error_text.is_none() && url != before.url {
                    readback_expectation = CdpPageReadbackExpectation::UrlChanged {
                        previous_url: before.url.clone(),
                    };
                }
            }
            CdpPageNavigationAction::Reload => {
                let reload = ReloadParams::builder().ignore_cache(ignore_cache).build();
                page.execute(reload)
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("Page.reload(ignoreCache={ignore_cache}): {err}"),
                    })?;
            }
            CdpPageNavigationAction::Back | CdpPageNavigationAction::Forward => {
                let history = page
                    .execute(GetNavigationHistoryParams::default())
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("Page.getNavigationHistory before history action: {err}"),
                    })?
                    .result;
                let delta = if action == CdpPageNavigationAction::Back {
                    -1
                } else {
                    1
                };
                let target_index = history.current_index + delta;
                let Some(entry) = history
                    .entries
                    .get(usize::try_from(target_index).unwrap_or(usize::MAX))
                else {
                    return Err(A11yError::CdpAxtreeFailed {
                        detail: format!(
                            "Page.{} refused: currentIndex={} entries={}",
                            action.as_str(),
                            history.current_index,
                            history.entries.len()
                        ),
                    });
                };
                page.execute(NavigateToHistoryEntryParams::new(entry.id))
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("Page.navigateToHistoryEntry(entryId={}): {err}", entry.id),
                    })?;
                readback_expectation = CdpPageReadbackExpectation::HistoryEntry {
                    current_index: target_index,
                    url: entry.url.clone(),
                };
            }
        }
        let after = wait_for_page_readback(&page, wait_timeout_ms, &readback_expectation).await?;
        if let Some(error_text) = navigation_error_text.as_deref() {
            let url = requested_url.as_deref().unwrap_or("");
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "Page.navigate({url:?}) failed: {error_text}; observed after readback url={:?} title={:?} readyState={:?} historyIndex={} historyEntries={} isDownload={:?}",
                    after.url,
                    after.title,
                    after.ready_state,
                    after.history_current_index,
                    after.history_entry_count,
                    is_download
                ),
            });
        }
        Ok(CdpPageNavigationResult {
            target_id,
            action: action.as_str().to_owned(),
            requested_url,
            before,
            after,
            navigation_error_text,
            is_download,
        })
    })
    .await
}

/// Dispatches wheel events over a web node after scrolling it into view.
///
/// # Errors
///
/// As [`cdp_click_node`].
pub async fn cdp_scroll_node(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    deltas: Vec<CdpWheelDelta>,
    interval_ms: u32,
) -> A11yResult<CdpActionPoint> {
    with_node_page(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page| async move {
            let rect = node_content_rect(&page, backend_node_id).await?;
            let center = CdpActionPoint {
                x: f64::from(rect.x) + f64::from(rect.w) / 2.0,
                y: f64::from(rect.y) + f64::from(rect.h) / 2.0,
            };
            let last_index = deltas.len().saturating_sub(1);
            for (index, delta) in deltas.into_iter().enumerate() {
                let dispatch = dispatch_dom_scroll(&page, backend_node_id, delta).await?;
                tracing::debug!(
                    code = "A11Y_CDP_DOM_SCROLL_DISPATCHED",
                    backend_node_id,
                    delta_x = delta.delta_x,
                    delta_y = delta.delta_y,
                    target_tag = %dispatch.target_tag,
                    target_id = %dispatch.target_id,
                    default_prevented = dispatch.default_prevented,
                    before_left = dispatch.target_scroll_left_before,
                    before_top = dispatch.target_scroll_top_before,
                    after_left = dispatch.target_scroll_left_after,
                    after_top = dispatch.target_scroll_top_after,
                    "CDP Runtime.callFunctionOn dispatched background DOM scroll"
                );
                if interval_ms > 0 && index < last_index {
                    tokio::time::sleep(std::time::Duration::from_millis(u64::from(interval_ms)))
                        .await;
                }
            }
            Ok(center)
        },
    )
    .await
}

async fn dispatch_dom_scroll(
    page: &chromiumoxide::Page,
    backend_node_id: i64,
    delta: CdpWheelDelta,
) -> A11yResult<CdpRuntimeScrollDispatch> {
    let resolve = ResolveNodeParams::builder()
        .backend_node_id(BackendNodeId::new(backend_node_id))
        .object_group("synapse_scroll_dispatch")
        .build();
    let resolved = page
        .execute(resolve)
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("resolveNode for backendNodeId {backend_node_id}: {err}"),
        })?;
    let object_id =
        resolved
            .object
            .object_id
            .clone()
            .ok_or_else(|| A11yError::CdpAxtreeFailed {
                detail: format!(
                    "resolveNode for backendNodeId {backend_node_id} returned no objectId"
                ),
            })?;
    let call = CallFunctionOnParams::builder()
        .function_declaration(
            r#"function(deltaX, deltaY) {
                const node = this;
                const doc = (node && node.ownerDocument) || document;
                const win = doc.defaultView || window;
                const root = doc.scrollingElement || doc.documentElement || doc.body;
                function isElement(value) {
                    return Boolean(value && value.nodeType === 1);
                }
                function isScrollable(element) {
                    if (!isElement(element)) { return false; }
                    const style = win.getComputedStyle(element);
                    const overflowY = style.overflowY || "";
                    const overflowX = style.overflowX || "";
                    const canY = /(auto|scroll|overlay)/.test(overflowY)
                        && element.scrollHeight > element.clientHeight;
                    const canX = /(auto|scroll|overlay)/.test(overflowX)
                        && element.scrollWidth > element.clientWidth;
                    return canY || canX;
                }
                let container = isElement(node) ? node : node.parentElement;
                while (container && container !== root && !isScrollable(container)) {
                    container = container.parentElement;
                }
                const target = (container && isScrollable(container)) ? container : root;
                const beforeLeft = Number(target.scrollLeft || win.scrollX || 0);
                const beforeTop = Number(target.scrollTop || win.scrollY || 0);
                const eventTarget = isElement(node) ? node : target;
                const event = new win.WheelEvent("wheel", {
                    deltaX,
                    deltaY,
                    bubbles: true,
                    cancelable: true,
                    view: win
                });
                const defaultAllowed = eventTarget.dispatchEvent(event);
                if (defaultAllowed) {
                    if (target === root) {
                        win.scrollBy(deltaX, deltaY);
                    } else if (typeof target.scrollBy === "function") {
                        target.scrollBy({ left: deltaX, top: deltaY, behavior: "auto" });
                    } else {
                        target.scrollLeft += deltaX;
                        target.scrollTop += deltaY;
                    }
                }
                return {
                    is_connected: Boolean(node && node.isConnected),
                    default_prevented: !defaultAllowed,
                    target_scroll_left_before: beforeLeft,
                    target_scroll_top_before: beforeTop,
                    target_scroll_left_after: Number(target.scrollLeft || win.scrollX || 0),
                    target_scroll_top_after: Number(target.scrollTop || win.scrollY || 0),
                    target_tag: String(target.tagName || "DOCUMENT"),
                    target_id: String(target.id || "")
                };
            }"#,
        )
        .object_id(object_id)
        .argument(CallArgument::builder().value(json!(delta.delta_x)).build())
        .argument(CallArgument::builder().value(json!(delta.delta_y)).build())
        .return_by_value(true)
        .silent(true)
        .build()
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("build Runtime.callFunctionOn scroll-dispatch params: {err}"),
        })?;
    let dispatch: CdpRuntimeScrollDispatch =
        call_function_on_value(page, call, "scroll-dispatch").await?;
    if !dispatch.is_connected {
        return Err(A11yError::CdpAxtreeFailed {
            detail: format!("backendNodeId {backend_node_id} resolved to a detached DOM node"),
        });
    }
    Ok(dispatch)
}

/// Reads a web node's semantic value/text via CDP after resolving the exact
/// backend node into a JavaScript object.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if the node cannot be resolved or evaluated.
pub async fn cdp_node_value(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<String> {
    with_node_page(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page| async move {
            let resolve = ResolveNodeParams::builder()
                .backend_node_id(BackendNodeId::new(backend_node_id))
                .object_group("synapse_verify_delta")
                .build();
            let resolved =
                page.execute(resolve)
                    .await
                    .map_err(|err| A11yError::CdpAxtreeFailed {
                        detail: format!("resolveNode for backendNodeId {backend_node_id}: {err}"),
                    })?;
            let object_id =
                resolved
                    .object
                    .object_id
                    .clone()
                    .ok_or_else(|| A11yError::CdpAxtreeFailed {
                        detail: format!(
                            "resolveNode for backendNodeId {backend_node_id} returned no objectId"
                        ),
                    })?;
            let call = CallFunctionOnParams::builder()
                .function_declaration(
                    "function() {
                    if (this === null || this === undefined) { return ''; }
                    if ('value' in this) { return String(this.value ?? ''); }
                    if ('checked' in this) { return String(Boolean(this.checked)); }
                    if (this.isContentEditable && this.innerText !== null && this.innerText !== undefined) {
                        return String(this.innerText).replace(/\\n$/, '');
                    }
                    if (this.textContent !== null && this.textContent !== undefined) {
                        return String(this.textContent);
                    }
                    return '';
                }",
                )
                .object_id(object_id)
                .return_by_value(true)
                .silent(true)
                .build()
                .map_err(|err| A11yError::CdpAxtreeFailed {
                    detail: format!("build Runtime.callFunctionOn params: {err}"),
                })?;
            call_function_on_value(&page, call, "value").await
        },
    )
    .await
}

/// Reads the DOM scroll state attached to a web node's nearest scroll container.
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if the node cannot be resolved or is detached.
pub async fn cdp_node_scroll_state(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<CdpScrollState> {
    with_node_page(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page| async move {
            let state = read_node_scroll_state(&page, backend_node_id).await?;
            if !state.is_connected {
                return Err(A11yError::CdpAxtreeFailed {
                    detail: format!(
                        "backendNodeId {backend_node_id} resolved to a detached DOM node"
                    ),
                });
            }
            Ok(state)
        },
    )
    .await
}

/// Resolves the viewport-CSS centre of a web node (for `act_stroke` target
/// aiming), scrolling it into view first.
///
/// # Errors
///
/// As [`cdp_click_node`].
pub async fn cdp_node_viewport_center(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<CdpActionPoint> {
    with_node_center(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |_page, center| async move { Ok(center) },
    )
    .await
}

/// Moves the in-page CDP pointer over a web node after scrolling it into view.
///
/// # Errors
///
/// As [`cdp_click_node`].
pub async fn cdp_aim_node(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<CdpActionPoint> {
    with_node_center(
        endpoint,
        page_title_hint,
        target_id_hint,
        backend_node_id,
        |page, center| async move {
            page.execute(mouse_event(
                DispatchMouseEventType::MouseMoved,
                center,
                MouseButton::None,
                0,
            ))
            .await
            .map_err(|err| dispatch_err(&err))?;
            Ok(center)
        },
    )
    .await
}

fn validate_cdp_mouse_stroke_points(points: &[CdpMouseStrokePoint]) -> A11yResult<()> {
    if points.is_empty() {
        return Err(A11yError::CdpAxtreeFailed {
            detail: "cdp_mouse_stroke_target requires at least one point".to_owned(),
        });
    }
    let mut previous_elapsed_ms = 0.0;
    for (index, point) in points.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() || !point.elapsed_ms.is_finite() {
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "cdp mouse stroke point {index} must contain finite x/y/elapsed_ms values"
                ),
            });
        }
        if point.elapsed_ms < 0.0 {
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "cdp mouse stroke point {index} elapsed_ms must be >= 0, got {}",
                    point.elapsed_ms
                ),
            });
        }
        if index > 0 && point.elapsed_ms < previous_elapsed_ms {
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "cdp mouse stroke point {index} elapsed_ms {} is before prior sample {}",
                    point.elapsed_ms, previous_elapsed_ms
                ),
            });
        }
        previous_elapsed_ms = point.elapsed_ms;
    }
    Ok(())
}

async fn dispatch_cdp_mouse_stroke(
    page: &chromiumoxide::Page,
    points: &[CdpMouseStrokePoint],
    button: MouseButton,
) -> A11yResult<()> {
    let first = points[0];
    let first_point = cdp_stroke_action_point(first);
    if button == MouseButton::None {
        let mut previous_elapsed_ms = first.elapsed_ms;
        dispatch_mouse_event(
            page,
            mouse_event(
                DispatchMouseEventType::MouseMoved,
                first_point,
                MouseButton::None,
                0,
            ),
            "move",
            0,
        )
        .await?;
        for (index, point) in points.iter().enumerate().skip(1) {
            sleep_until_sample(previous_elapsed_ms, point.elapsed_ms).await;
            previous_elapsed_ms = point.elapsed_ms;
            dispatch_mouse_event(
                page,
                mouse_event(
                    DispatchMouseEventType::MouseMoved,
                    cdp_stroke_action_point(*point),
                    MouseButton::None,
                    0,
                ),
                "move",
                index,
            )
            .await?;
        }
        return Ok(());
    }

    dispatch_mouse_event(
        page,
        mouse_event(
            DispatchMouseEventType::MouseMoved,
            first_point,
            MouseButton::None,
            0,
        ),
        "pre_press_move",
        0,
    )
    .await?;
    dispatch_mouse_event(
        page,
        mouse_event(
            DispatchMouseEventType::MousePressed,
            first_point,
            button.clone(),
            1,
        ),
        "press",
        0,
    )
    .await?;

    let held_buttons = mouse_button_bit(&button);
    let mut previous_elapsed_ms = first.elapsed_ms;
    for (index, point) in points.iter().enumerate().skip(1) {
        sleep_until_sample(previous_elapsed_ms, point.elapsed_ms).await;
        previous_elapsed_ms = point.elapsed_ms;
        dispatch_mouse_event(
            page,
            mouse_event_with_buttons(
                DispatchMouseEventType::MouseMoved,
                cdp_stroke_action_point(*point),
                button.clone(),
                0,
                Some(held_buttons),
            ),
            "drag_move",
            index,
        )
        .await?;
    }
    dispatch_mouse_event(
        page,
        mouse_event(
            DispatchMouseEventType::MouseReleased,
            points
                .last()
                .map_or(first_point, |point| cdp_stroke_action_point(*point)),
            button,
            1,
        ),
        "release",
        points.len().saturating_sub(1),
    )
    .await?;
    Ok(())
}

async fn dispatch_mouse_event(
    page: &chromiumoxide::Page,
    params: DispatchMouseEventParams,
    stage: &'static str,
    sample_index: usize,
) -> A11yResult<()> {
    match tokio::time::timeout(CDP_INPUT_COMMAND_TIMEOUT, page.execute(params)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(dispatch_err(&err)),
        Err(_) => Err(A11yError::CdpAxtreeFailed {
            detail: format!(
                "CDP input dispatch timed out after {} ms at stage {stage} sample_index={sample_index}",
                CDP_INPUT_COMMAND_TIMEOUT.as_millis()
            ),
        }),
    }
}

async fn sleep_until_sample(previous_elapsed_ms: f64, next_elapsed_ms: f64) {
    let delta_ms = (next_elapsed_ms - previous_elapsed_ms).max(0.0);
    if delta_ms > 0.0 {
        tokio::time::sleep(std::time::Duration::from_secs_f64(delta_ms / 1000.0)).await;
    }
}

fn cdp_stroke_action_point(point: CdpMouseStrokePoint) -> CdpActionPoint {
    CdpActionPoint {
        x: point.x,
        y: point.y,
    }
}

/// A decoded, top-down BGRA8 bitmap captured from a web node via CDP (#703).
///
/// `bgra` is 4 bytes per pixel with no row padding, sized `width * height * 4`,
/// ready for the `WinRT` OCR `read_text_from_bgra_bitmap` path.
#[derive(Clone, Debug)]
pub struct CdpNodeBitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Captures just a web node's rendered pixels and returns them as a BGRA8 bitmap
/// for OCR (#703).
///
/// Mirrors how clicks resolve a node — attach, find the owning page, scroll the
/// node into view, resolve its live box model — then converts the viewport-CSS
/// box to document coordinates (using `Page.getLayoutMetrics` scroll offset) and
/// captures exactly that element via `Page.captureScreenshot { clip,
/// captureBeyondViewport:true }`. This is DPI-/scroll-/occlusion-robust and
/// needs no CSS→screen mapping (which the click path also deliberately avoids).
///
/// # Errors
///
/// `A11Y_CDP_ATTACH_FAILED` if the endpoint/node cannot be reached;
/// `A11Y_CDP_AXTREE_FAILED` if box-model resolution, layout metrics, capture, or
/// PNG decode fails.
pub async fn cdp_capture_node_bgra(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<CdpNodeBitmap> {
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let page =
            resolve_owning_page(&browser, page_title_hint, target_id_hint, backend_node_id).await?;
        let rect = node_content_rect(&page, backend_node_id).await?;
        // getBoxModel is viewport-relative (the click path dispatches at its
        // centre as viewport coords and lands correctly); captureScreenshot with
        // captureBeyondViewport=true expects document coords, so add the scroll
        // offset. Chrome's own "Capture node screenshot" uses the same shape.
        let metrics = page
            .execute(GetLayoutMetricsParams::default())
            .await
            .map_err(|err| A11yError::CdpAxtreeFailed {
                detail: format!("getLayoutMetrics: {err}"),
            })?;
        let scroll_x = i32::try_from(metrics.result.css_layout_viewport.page_x).unwrap_or(0);
        let scroll_y = i32::try_from(metrics.result.css_layout_viewport.page_y).unwrap_or(0);
        let clip = Viewport {
            x: f64::from(rect.x) + f64::from(scroll_x),
            y: f64::from(rect.y) + f64::from(scroll_y),
            width: f64::from(rect.w),
            height: f64::from(rect.h),
            scale: 1.0,
        };
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .clip(clip)
            .from_surface(true)
            .capture_beyond_viewport(true)
            .build();
        let png_bytes =
            page.screenshot(params)
                .await
                .map_err(|err| A11yError::CdpAxtreeFailed {
                    detail: format!("captureScreenshot: {err}"),
                })?;
        decode_png_to_bgra(&png_bytes)
    }
    .await;

    handler_task.abort();
    result
}

fn mouse_event(
    kind: DispatchMouseEventType,
    point: CdpActionPoint,
    button: MouseButton,
    click_count: i64,
) -> DispatchMouseEventParams {
    mouse_event_with_buttons(kind, point, button, click_count, None)
}

fn mouse_event_with_buttons(
    kind: DispatchMouseEventType,
    point: CdpActionPoint,
    button: MouseButton,
    click_count: i64,
    buttons_override: Option<i64>,
) -> DispatchMouseEventParams {
    // `buttons` is the bitmask of buttons CURRENTLY held: the button's bit while
    // pressed, 0 once moved or released. Getting this wrong (e.g. leaving the
    // bit set on release) makes Chrome think the button is still down and it
    // never synthesises a `click` event.
    let is_pressed = matches!(kind, DispatchMouseEventType::MousePressed);
    let bit = mouse_button_bit(&button);
    let mut params = DispatchMouseEventParams::new(kind, point.x, point.y);
    params.click_count = Some(click_count);
    params.buttons = Some(buttons_override.unwrap_or(if is_pressed { bit } else { 0 }));
    params.button = Some(button);
    params
}

fn mouse_button_bit(button: &MouseButton) -> i64 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
        _ => 0,
    }
}

fn dispatch_err(err: &chromiumoxide::error::CdpError) -> A11yError {
    A11yError::CdpAxtreeFailed {
        detail: format!("CDP input dispatch failed: {err}"),
    }
}

fn cdp_key_event(
    event_type: DispatchKeyEventType,
    key: &CdpKeyStroke,
    modifiers: i64,
) -> A11yResult<DispatchKeyEventParams> {
    let mut builder = DispatchKeyEventParams::builder()
        .r#type(event_type)
        .modifiers(modifiers)
        .key(key.key.clone())
        .code(key.code.clone())
        .windows_virtual_key_code(key.windows_virtual_key_code)
        .native_virtual_key_code(key.native_virtual_key_code);
    if let Some(value) = &key.key_identifier {
        builder = builder.key_identifier(value.clone());
    }
    if let Some(value) = &key.text {
        builder = builder.text(value.clone());
    }
    if let Some(value) = &key.unmodified_text {
        builder = builder.unmodified_text(value.clone());
    }
    if let Some(value) = key.location {
        builder = builder.location(value);
    }
    builder.build().map_err(|err| A11yError::CdpAxtreeFailed {
        detail: format!("build Input.dispatchKeyEvent params: {err}"),
    })
}

async fn wait_for_page_readback(
    page: &chromiumoxide::Page,
    wait_timeout_ms: u64,
    expectation: &CdpPageReadbackExpectation,
) -> A11yResult<CdpPageState> {
    let started = tokio::time::Instant::now();
    let budget = std::time::Duration::from_millis(wait_timeout_ms);
    let mut last_state: Option<CdpPageState> = None;
    let mut last_error: Option<String> = None;
    loop {
        match read_page_state(page).await {
            Ok(state) => {
                let loaded = state.ready_state == "complete" || state.ready_state == "interactive";
                if loaded && expectation.matches(&state) {
                    return Ok(state);
                }
                last_state = Some(state);
            }
            Err(err) => {
                last_error = Some(err.to_string());
            }
        }
        if started.elapsed() >= budget {
            if let Some(state) = last_state {
                return Err(A11yError::CdpAxtreeFailed {
                    detail: format!(
                        "page readback did not settle within {wait_timeout_ms} ms waiting for {}; last url={:?} title={:?} readyState={:?} historyIndex={} historyEntries={}",
                        expectation.detail(),
                        state.url,
                        state.title,
                        state.ready_state,
                        state.history_current_index,
                        state.history_entry_count
                    ),
                });
            }
            if let Some(error) = last_error {
                return Err(A11yError::CdpAxtreeFailed {
                    detail: format!(
                        "page readback did not settle within {wait_timeout_ms} ms; last readback error: {error}"
                    ),
                });
            }
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "page readback did not settle within {wait_timeout_ms} ms; no page state readback"
                ),
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn read_page_state(page: &chromiumoxide::Page) -> A11yResult<CdpPageState> {
    #[derive(Debug, Deserialize)]
    struct DomState {
        url: String,
        title: String,
        ready_state: String,
    }

    let dom = page
        .evaluate_expression(
            r#"(() => ({
                url: String(location.href || ""),
                title: String(document.title || ""),
                ready_state: String(document.readyState || "")
            }))()"#,
        )
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Runtime.evaluate page readback: {err}"),
        })?
        .into_value::<DomState>()
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Runtime.evaluate page readback decode: {err}"),
        })?;
    let history = page
        .execute(GetNavigationHistoryParams::default())
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Page.getNavigationHistory readback: {err}"),
        })?
        .result;
    Ok(CdpPageState {
        url: dom.url,
        title: dom.title,
        ready_state: dom.ready_state,
        history_current_index: history.current_index,
        history_entry_count: u32::try_from(history.entries.len()).unwrap_or(u32::MAX),
    })
}

/// Polls `browser.pages()` until target discovery surfaces at least one page
/// (fresh connections discover targets asynchronously), up to ~3s.
async fn wait_for_pages(browser: &chromiumoxide::Browser) -> A11yResult<Vec<chromiumoxide::Page>> {
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

/// Attaches, finds the page owning `backend_node_id`, scrolls it into view,
/// resolves its box-model centre, runs `action(page, center)`, and tears down.
async fn with_node_center<A, Fut, T>(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    action: A,
) -> A11yResult<T>
where
    A: FnOnce(chromiumoxide::Page, CdpActionPoint) -> Fut,
    Fut: std::future::Future<Output = A11yResult<T>>,
{
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let page =
            resolve_owning_page(&browser, page_title_hint, target_id_hint, backend_node_id).await?;
        let rect = node_content_rect(&page, backend_node_id).await?;
        let center = CdpActionPoint {
            x: f64::from(rect.x) + f64::from(rect.w) / 2.0,
            y: f64::from(rect.y) + f64::from(rect.h) / 2.0,
        };
        action(page, center).await
    }
    .await;

    handler_task.abort();
    result
}

async fn with_node_page<A, Fut, T>(
    endpoint: &str,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
    action: A,
) -> A11yResult<T>
where
    A: FnOnce(chromiumoxide::Page) -> Fut,
    Fut: std::future::Future<Output = A11yResult<T>>,
{
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let page =
            resolve_owning_page(&browser, page_title_hint, target_id_hint, backend_node_id).await?;
        action(page).await
    }
    .await;

    handler_task.abort();
    result
}

async fn with_target_page<A, Fut, T>(endpoint: &str, target_id: &str, action: A) -> A11yResult<T>
where
    A: FnOnce(chromiumoxide::Page) -> Fut,
    Fut: std::future::Future<Output = A11yResult<T>>,
{
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err(A11yError::CdpAttachFailed {
            detail: "CDP target id must not be empty".to_owned(),
        });
    }
    let (browser, mut handler) =
        Browser::connect(endpoint)
            .await
            .map_err(|err| A11yError::CdpAttachFailed {
                detail: format!("connect {endpoint}: {err}"),
            })?;
    let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = async {
        let page = get_target_page_with_discovery(&browser, target_id).await?;
        action(page).await
    }
    .await;

    handler_task.abort();
    result
}

/// Resolves a CDP target id to its [`chromiumoxide::Page`], tolerating the
/// asynchronous target discovery that follows every fresh `Browser::connect`.
///
/// Root cause of #982: each action opens a fresh CDP connection, and a freshly
/// connected browser populates its target map from
/// `Target.targetCreated`/`attachedToTarget` events processed on the handler
/// task. The FIRST `get_page` after connect therefore routinely races ahead of
/// discovery and returns `NotFound`, surfacing as
/// "selected target … is no longer present for backendNodeId N" for EVERY CDP
/// web element — top-level or iframe — not just nested frames. `observe` never
/// hit this because it waits via [`wait_for_pages`] and retries `get_page`
/// (`wait_for_page_target`); the action path did a single unguarded `get_page`.
///
/// This mirrors observe: prime discovery with [`wait_for_pages`], then retry
/// `get_page` on a bounded schedule. If the target id names an out-of-process
/// iframe child target (its own `targetId`), a plain `get_page` never resolves
/// until flat auto-attach exposes a callable session, so after a few failed
/// attempts we enable flat iframe auto-attach on the discovered pages — exactly
/// what observe does before reading OOPIF DOM.
async fn get_page_with_discovery(
    browser: &chromiumoxide::Browser,
    target_id: &str,
    backend_node_id: i64,
) -> A11yResult<chromiumoxide::Page> {
    get_target_page_with_discovery(browser, target_id)
        .await
        .map_err(|error| match error {
            A11yError::CdpAxtreeFailed { detail } => A11yError::CdpAxtreeFailed {
                detail: format!("{detail} for backendNodeId {backend_node_id}"),
            },
            other => other,
        })
}

async fn get_target_page_with_discovery(
    browser: &chromiumoxide::Browser,
    target_id: &str,
) -> A11yResult<chromiumoxide::Page> {
    // Block until target discovery surfaces at least one page (or the endpoint
    // is unreachable) so the retry loop below is not racing an empty map.
    let pages = wait_for_pages(browser).await?;
    let mut last_error: Option<String> = None;
    let mut auto_attach_enabled = false;
    for attempt in 0..30u32 {
        match browser.get_page(TargetId::new(target_id.to_owned())).await {
            Ok(page) => return Ok(page),
            Err(error) => last_error = Some(error.to_string()),
        }
        // The common in-process case (target id == page target) resolves on an
        // early attempt once discovery lands. If it has not resolved after a few
        // tries the id likely names an OOPIF child target that needs flat
        // auto-attach to expose a session; enable it once, then keep retrying.
        if attempt == 4 && !auto_attach_enabled {
            for page in &pages {
                let _ = crate::cdp_dom::enable_flat_iframe_auto_attach(page).await;
            }
            auto_attach_enabled = true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let discovered = pages
        .iter()
        .map(|page| page.target_id().inner().clone())
        .collect::<Vec<_>>()
        .join(",");
    Err(A11yError::CdpAxtreeFailed {
        detail: format!(
            "selected target {target_id} is no longer present after waiting for CDP target discovery (auto_attach_enabled={auto_attach_enabled}; {} page target(s) discovered: [{discovered}]); last get_page error: {}",
            pages.len(),
            last_error.unwrap_or_else(|| "none".to_owned())
        ),
    })
}

/// Finds the attached page that owns `backend_node_id`, priming each candidate's
/// DOM and confirming ownership by scrolling the node into view.
///
/// Backend node ids are per-DOCUMENT, so the same numeric id can exist in
/// several tabs. Candidate pages whose title matches the foreground window (the
/// tab `observe` read) are tried first. A fresh CDP connection discovers targets
/// asynchronously and has not been pushed each page's DOM, so we poll for pages
/// and prime with `DOM.getDocument` before resolving (required, not optional).
async fn resolve_owning_page(
    browser: &chromiumoxide::Browser,
    page_title_hint: &str,
    target_id_hint: Option<&str>,
    backend_node_id: i64,
) -> A11yResult<chromiumoxide::Page> {
    if let Some(target_id_hint) = target_id_hint.filter(|hint| !hint.trim().is_empty()) {
        let target_id_hint = target_id_hint.trim();
        let page = get_page_with_discovery(browser, target_id_hint, backend_node_id).await?;
        return if page_owns_backend_node(&page, backend_node_id).await {
            Ok(page)
        } else {
            Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "selected target {target_id_hint} does not own backendNodeId {backend_node_id}"
                ),
            })
        };
    }

    let pages = wait_for_pages(browser).await?;
    let mut ordered = Vec::with_capacity(pages.len());
    let mut tail = Vec::new();
    for page in pages {
        let matches_hint = matches!(
            page.get_title().await,
            Ok(Some(title)) if !title.is_empty() && page_title_hint.contains(title.as_str())
        );
        if matches_hint {
            ordered.push(page);
        } else {
            tail.push(page);
        }
    }
    ordered.extend(tail);

    for page in ordered {
        if page_owns_backend_node(&page, backend_node_id).await {
            return Ok(page);
        }
    }
    Err(A11yError::CdpAxtreeFailed {
        detail: format!("no attached page owns backendNodeId {backend_node_id}"),
    })
}

async fn page_owns_backend_node(page: &chromiumoxide::Page, backend_node_id: i64) -> bool {
    use chromiumoxide::cdp::browser_protocol::dom::GetDocumentParams;

    let prime = GetDocumentParams::builder().depth(-1).pierce(true).build();
    let _ = page.execute(prime).await;
    let scroll = ScrollIntoViewIfNeededParams::builder()
        .backend_node_id(BackendNodeId::new(backend_node_id))
        .build();
    page.execute(scroll).await.is_ok()
}

/// Resolves a web node's live content-box rectangle in viewport-CSS pixels.
async fn node_content_rect(
    page: &chromiumoxide::Page,
    backend_node_id: i64,
) -> A11yResult<synapse_core::Rect> {
    let box_params = GetBoxModelParams::builder()
        .backend_node_id(BackendNodeId::new(backend_node_id))
        .build();
    let model = page
        .execute(box_params)
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("getBoxModel: {err}"),
        })?;
    rect_from_quad(model.result.model.content.inner()).ok_or_else(|| A11yError::CdpAxtreeFailed {
        detail: "node has no resolvable box model (not rendered)".to_owned(),
    })
}

async fn read_node_scroll_state(
    page: &chromiumoxide::Page,
    backend_node_id: i64,
) -> A11yResult<CdpScrollState> {
    let resolve = ResolveNodeParams::builder()
        .backend_node_id(BackendNodeId::new(backend_node_id))
        .object_group("synapse_scroll_verify")
        .build();
    let resolved = page
        .execute(resolve)
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("resolveNode for backendNodeId {backend_node_id}: {err}"),
        })?;
    let object_id =
        resolved
            .object
            .object_id
            .clone()
            .ok_or_else(|| A11yError::CdpAxtreeFailed {
                detail: format!(
                    "resolveNode for backendNodeId {backend_node_id} returned no objectId"
                ),
            })?;
    let call = CallFunctionOnParams::builder()
        .function_declaration(
            r#"function() {
                const node = this;
                const doc = (node && node.ownerDocument) || document;
                const win = doc.defaultView || window;
                const root = doc.scrollingElement || doc.documentElement || doc.body;
                function isElement(value) {
                    return Boolean(value && value.nodeType === 1);
                }
                function isScrollable(element) {
                    if (!isElement(element)) { return false; }
                    const style = win.getComputedStyle(element);
                    const overflowY = style.overflowY || "";
                    const overflowX = style.overflowX || "";
                    const canY = /(auto|scroll|overlay)/.test(overflowY)
                        && element.scrollHeight > element.clientHeight;
                    const canX = /(auto|scroll|overlay)/.test(overflowX)
                        && element.scrollWidth > element.clientWidth;
                    return canY || canX;
                }
                let container = isElement(node) ? node : node.parentElement;
                while (container && container !== root && !isScrollable(container)) {
                    container = container.parentElement;
                }
                const target = (container && isScrollable(container)) ? container : root;
                const rect = node && node.getBoundingClientRect
                    ? node.getBoundingClientRect()
                    : { left: 0, top: 0, width: 0, height: 0 };
                return {
                    is_connected: Boolean(node && node.isConnected),
                    window_scroll_x: Number(win.scrollX || 0),
                    window_scroll_y: Number(win.scrollY || 0),
                    target_scroll_left: Number(target.scrollLeft || 0),
                    target_scroll_top: Number(target.scrollTop || 0),
                    target_scroll_width: Number(target.scrollWidth || 0),
                    target_scroll_height: Number(target.scrollHeight || 0),
                    target_client_width: Number(target.clientWidth || 0),
                    target_client_height: Number(target.clientHeight || 0),
                    target_tag: String(target.tagName || "DOCUMENT"),
                    target_id: String(target.id || ""),
                    node_rect_left: Number(rect.left || 0),
                    node_rect_top: Number(rect.top || 0),
                    node_rect_width: Number(rect.width || 0),
                    node_rect_height: Number(rect.height || 0)
                };
            }"#,
        )
        .object_id(object_id)
        .return_by_value(true)
        .silent(true)
        .build()
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("build Runtime.callFunctionOn scroll-state params: {err}"),
        })?;
    call_function_on_value(page, call, "scroll-state").await
}

async fn call_function_on_value<T>(
    page: &chromiumoxide::Page,
    call: CallFunctionOnParams,
    label: &str,
) -> A11yResult<T>
where
    T: DeserializeOwned,
{
    let returns = page
        .execute(call)
        .await
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("Runtime.callFunctionOn {label} readback: {err}"),
        })?;
    if let Some(exception) = &returns.exception_details {
        return Err(A11yError::CdpAxtreeFailed {
            detail: format!("Runtime.callFunctionOn {label} exception: {exception:?}"),
        });
    }
    let remote = returns.result.result.clone();
    let value = remote.value.ok_or_else(|| A11yError::CdpAxtreeFailed {
        detail: format!(
            "Runtime.callFunctionOn {label} returned no by-value payload (type={:?} subtype={:?} description={:?})",
            remote.r#type, remote.subtype, remote.description
        ),
    })?;
    serde_json::from_value::<T>(value).map_err(|err| A11yError::CdpAxtreeFailed {
        detail: format!("Runtime.callFunctionOn {label} decode: {err}"),
    })
}

/// Decodes a Chrome PNG screenshot into a top-down BGRA8 bitmap for OCR (#703).
fn decode_png_to_bgra(png_bytes: &[u8]) -> A11yResult<CdpNodeBitmap> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes))
        .read_info()
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("screenshot PNG header decode failed: {err}"),
        })?;
    let buf_size = reader
        .output_buffer_size()
        .ok_or_else(|| A11yError::CdpAxtreeFailed {
            detail: "screenshot PNG output buffer size overflowed usize".to_owned(),
        })?;
    let mut buf = vec![0u8; buf_size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|err| A11yError::CdpAxtreeFailed {
            detail: format!("screenshot PNG frame decode failed: {err}"),
        })?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(A11yError::CdpAxtreeFailed {
            detail: format!(
                "unexpected screenshot PNG bit depth {:?}; expected 8-bit",
                info.bit_depth
            ),
        });
    }
    let pixels = buf
        .get(..info.buffer_size())
        .ok_or_else(|| A11yError::CdpAxtreeFailed {
            detail: "screenshot PNG buffer shorter than reported frame size".to_owned(),
        })?;
    let bgra = match info.color_type {
        png::ColorType::Rgba => rgba8_to_bgra(pixels),
        png::ColorType::Rgb => rgb8_to_bgra(pixels),
        other => {
            return Err(A11yError::CdpAxtreeFailed {
                detail: format!(
                    "unexpected screenshot PNG color type {other:?}; expected RGB/RGBA"
                ),
            });
        }
    };
    Ok(CdpNodeBitmap {
        width: info.width,
        height: info.height,
        bgra,
    })
}

/// Swaps RGBA8 → BGRA8 (Chrome screenshots are RGBA; `WinRT` OCR wants BGRA).
fn rgba8_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

/// Expands RGB8 → BGRA8 with an opaque alpha channel.
fn rgb8_to_bgra(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.chunks_exact(3) {
        out.extend_from_slice(&[px[2], px[1], px[0], 0xFF]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Locks the FSV-discovered bug: leaving the `buttons` bit set on release
    // makes Chrome think the button is still held and never fires a `click`
    // event. Pressed → button bit; moved/released → 0.
    #[test]
    fn mouse_event_buttons_bitmask_is_set_only_while_pressed() {
        let point = CdpActionPoint { x: 10.0, y: 20.0 };
        let pressed = mouse_event(
            DispatchMouseEventType::MousePressed,
            point,
            MouseButton::Left,
            1,
        );
        let released = mouse_event(
            DispatchMouseEventType::MouseReleased,
            point,
            MouseButton::Left,
            1,
        );
        let moved = mouse_event(
            DispatchMouseEventType::MouseMoved,
            point,
            MouseButton::Left,
            0,
        );
        let hover = mouse_event(
            DispatchMouseEventType::MouseMoved,
            point,
            MouseButton::None,
            0,
        );
        println!(
            "readback=mouse_event buttons pressed:{:?} released:{:?} moved:{:?} hover_button:{:?}",
            pressed.buttons, released.buttons, moved.buttons, hover.button
        );
        assert_eq!(pressed.buttons, Some(1), "left press must hold bit 1");
        assert_eq!(released.buttons, Some(0), "release must clear the bitmask");
        assert_eq!(moved.buttons, Some(0), "move must not hold any button");
        assert_eq!(hover.button, Some(MouseButton::None));
        assert_eq!(pressed.click_count, Some(1));

        let right = mouse_event(
            DispatchMouseEventType::MousePressed,
            point,
            MouseButton::Right,
            1,
        );
        assert_eq!(right.buttons, Some(2), "right press must hold bit 2");
    }

    #[test]
    fn cdp_mouse_button_maps_to_cdp_enum() {
        assert_eq!(CdpMouseButton::Left.to_cdp(), MouseButton::Left);
        assert_eq!(CdpMouseButton::Right.to_cdp(), MouseButton::Right);
        assert_eq!(CdpMouseButton::Middle.to_cdp(), MouseButton::Middle);
    }

    #[test]
    fn mouse_event_drag_move_can_hold_button_bit() {
        let point = CdpActionPoint { x: 10.0, y: 20.0 };
        let drag_move = mouse_event_with_buttons(
            DispatchMouseEventType::MouseMoved,
            point,
            MouseButton::Left,
            0,
            Some(mouse_button_bit(&MouseButton::Left)),
        );
        println!(
            "readback=mouse_event drag_move buttons:{:?} button:{:?}",
            drag_move.buttons, drag_move.button
        );
        assert_eq!(drag_move.buttons, Some(1));
        assert_eq!(drag_move.button, Some(MouseButton::Left));
    }
}
