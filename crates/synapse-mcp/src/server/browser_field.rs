//! `browser_set_value` (#1000/#994/#1005/#717): background-safe field REPLACE in
//! the user's *normal, authenticated* Chrome via the safe extension bridge — no
//! debugger attach, no OS foreground, no UIA dependency.
//!
//! Root cause this closes: when a Chromium web form is perceived UIA-only (the
//! user's real Chrome has no `--remote-debugging-port`, by design), the legacy
//! `act_set_field_text` Chromium path was the *foreground* UIA-keys tier — so an
//! agent had to steal the operator's foreground to type into a tab. The bridge's
//! `chrome.scripting` path runs in the renderer regardless of paint/foreground
//! state (proven live: it reads/acts on occluded, inactive tabs UIA cannot see),
//! so this tool replaces a field's value entirely in-page and verifies it with
//! TWO independent reads: the in-page post-set readback, and a separate
//! `chrome.tabs` active-element readback. Fail-loud on every divergence; never an
//! optimistic success and never a foreground fallback.

use super::{ErrorData, Json, Parameters, SessionTarget, SynapseService, tool, tool_router};
use crate::m1::mcp_error;
use crate::m2::postcondition::text_signature;
use rmcp::schemars::JsonSchema;
use rmcp::{RoleServer, model::ErrorCode, service::RequestContext};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synapse_core::error_codes;

const TOOL: &str = "browser_set_value";
const CHROME_TAB_PREFIX: &str = "chrome-tab:";
const SOURCE_OF_TRUTH: &str =
    "chrome_bridge_in_page_value + separate chrome.tabs active-element readback";

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserSetValueParams {
    /// Full replacement text. Empty string clears the field.
    pub text: String,
    /// Strict CSS selector for the target field. Exactly one editable+visible
    /// match is required; 0 or >1 fails loud. Mutually exclusive with
    /// `active_element`.
    #[serde(default)]
    pub selector: Option<String>,
    /// Target the tab's current `document.activeElement` instead of a selector.
    /// Mutually exclusive with `selector`.
    #[serde(default)]
    pub active_element: bool,
    /// Chrome bridge tab target id (`chrome-tab:<id>`). Defaults to this
    /// session's active CDP target. Must be owned by this session; the human
    /// foreground tab is never an implicit fallback.
    #[serde(default)]
    pub cdp_target_id: Option<String>,
    /// Browser HWND owning the target. Defaults to the session target's window.
    #[serde(default)]
    pub window_hwnd: Option<i64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrowserSetValueResponse {
    pub ok: bool,
    /// Always false — this tool never drives or depends on the OS foreground.
    pub required_foreground: bool,
    pub transport: String,
    pub window_hwnd: i64,
    pub cdp_target_id: String,
    /// `selector` or `active_element`.
    pub resolved_by: String,
    /// Editable+visible nodes the selector matched (1 on success).
    pub match_count: u32,
    pub tag_name: String,
    pub source_of_truth: String,
    pub requested_len: u32,
    pub before_len: u32,
    pub after_len: u32,
    pub requested_sha256: String,
    pub before_sha256: String,
    pub after_sha256: String,
    /// Whether the value actually changed (before != after).
    pub changed: bool,
    /// Length of the SEPARATE chrome.tabs active-element readback.
    pub independent_readback_len: u32,
    pub independent_readback_sha256: String,
    pub status: String,
    pub elapsed_ms: u32,
}

#[tool_router(router = browser_field_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Background-safe REPLACE of a web form field's text in the user's normal authenticated Chrome via the safe extension bridge (#1000/#717). No debugger attach, no OS foreground, no UIA: runs entirely in-page through chrome.scripting, so it works on inactive/occluded tabs and never steals the operator's foreground. Target by strict CSS `selector` (exactly one editable+visible match; 0 or >1 fails loud) or `active_element=true`. Replaces the value with the native prototype setter (React/Vue/Angular-safe) and verifies with TWO independent reads (in-page post-set + a separate chrome.tabs active-element readback); any divergence is ACTION_POSTCONDITION_FAILED, never an optimistic success. Defaults to this session's active CDP tab target (bind one with set_target/cdp_open_tab); the human foreground tab is never a fallback. Use this instead of foregrounding Chrome to type into a dashboard/form."
    )]
    pub async fn browser_set_value(
        &self,
        params: Parameters<BrowserSetValueParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<BrowserSetValueResponse>, ErrorData> {
        let params = params.0;
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=browser_set_value"
        );
        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::TOOL_PARAMS_INVALID,
                    format!("{TOOL} requires an MCP session id (run the daemon in HTTP mode)"),
                )
            })?;

        let locator = validate_locator(&params)?;
        let (window_hwnd, cdp_target_id) = self.resolve_bridge_target(&session_id, &params)?;

        if synapse_a11y::endpoint_for_window(window_hwnd).is_some() {
            return Err(mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!(
                    "{TOOL} targets the normal Chrome extension bridge, but window {window_hwnd} exposes a raw CDP debug endpoint; use the raw-CDP browser_* tools for a Synapse automation profile"
                ),
            ));
        }
        if !cdp_target_id.starts_with(CHROME_TAB_PREFIX) {
            return Err(mcp_error(
                error_codes::ACTION_TARGET_INVALID,
                format!(
                    "{TOOL} requires a normal Chrome bridge tab target ({CHROME_TAB_PREFIX}<id>); got {cdp_target_id:?}"
                ),
            ));
        }

        let request_details = json!({
            "session_id": &session_id,
            "window_hwnd": window_hwnd,
            "cdp_target_id": &cdp_target_id,
            "resolved_by": locator.label(),
            "requested_len": char_len(&params.text),
            "requested_sha256": text_signature(&params.text),
            "required_foreground": false,
        });
        self.audit_action_started_with_details_for_session(TOOL, &request_details, &session_id)?;
        let result = self
            .browser_set_value_run(
                &session_id,
                window_hwnd,
                &cdp_target_id,
                &locator,
                &params.text,
            )
            .await;
        self.audit_action_result_for_session(TOOL, &result, &session_id)?;
        result.map(Json)
    }

    async fn browser_set_value_run(
        &self,
        session_id: &str,
        window_hwnd: i64,
        cdp_target_id: &str,
        locator: &Locator,
        text: &str,
    ) -> Result<BrowserSetValueResponse, ErrorData> {
        let started = std::time::Instant::now();
        let (selector_arg, active_arg) = match locator {
            Locator::Selector(selector) => (Some(selector.as_str()), false),
            Locator::ActiveElement => (None, true),
        };

        let result = crate::chrome_debugger_bridge::set_field_value(
            window_hwnd,
            cdp_target_id,
            selector_arg,
            active_arg,
            text,
        )
        .await
        .map_err(|error| {
            mcp_error(
                error.code(),
                format!(
                    "{TOOL} bridge setFieldValue failed for target {cdp_target_id:?}: {}",
                    error.detail()
                ),
            )
        })?;

        let before_value = result.before_value.clone().unwrap_or_default();
        let after_value = result.after_value.clone().ok_or_else(|| {
            mcp_error(
                error_codes::ACTION_POSTCONDITION_FAILED,
                format!("{TOOL} bridge returned no after_value for target {cdp_target_id:?}"),
            )
        })?;

        // SoT #1: the in-page post-set value must equal the requested text.
        if !value_matches(&after_value, text) {
            return Err(postcondition_error(
                "in_page_post_set",
                cdp_target_id,
                text,
                &after_value,
            ));
        }

        // SoT #2: a SEPARATE chrome.tabs active-element readback (the field is
        // focused by setFieldValue) must independently equal the requested text.
        let info = crate::chrome_debugger_bridge::target_info(window_hwnd, cdp_target_id)
            .await
            .map_err(|error| {
                mcp_error(
                    error.code(),
                    format!(
                        "{TOOL} separate active-element readback failed for target {cdp_target_id:?}: {}",
                        error.detail()
                    ),
                )
            })?;
        let independent = info
            .active_element
            .as_ref()
            .filter(|active| active.has_active_element == Some(true))
            .and_then(|active| active.value.clone())
            .unwrap_or_default();
        if !value_matches(&independent, text) {
            return Err(postcondition_error(
                "separate_active_element_readback",
                cdp_target_id,
                text,
                &independent,
            ));
        }

        let requested_len = char_len(text);
        let before_len = char_len(&before_value);
        let after_len = char_len(&after_value);
        let changed = before_value != after_value;

        tracing::info!(
            code = "BROWSER_SET_VALUE_READBACK",
            session_id = %session_id,
            hwnd = window_hwnd,
            cdp_target_id = %cdp_target_id,
            resolved_by = %result.resolved_by,
            match_count = result.match_count,
            tag_name = %result.tag_name,
            before_len,
            after_len,
            requested_len,
            changed,
            "readback=browser_set_value method=chrome.scripting.executeScript dual_verified=true"
        );

        Ok(BrowserSetValueResponse {
            ok: true,
            required_foreground: false,
            transport: "chrome_tabs_extension".to_owned(),
            window_hwnd,
            cdp_target_id: cdp_target_id.to_owned(),
            resolved_by: result.resolved_by,
            match_count: result.match_count,
            tag_name: result.tag_name,
            source_of_truth: SOURCE_OF_TRUTH.to_owned(),
            requested_len,
            before_len,
            after_len,
            requested_sha256: text_signature(text),
            before_sha256: text_signature(&before_value),
            after_sha256: text_signature(&after_value),
            changed,
            independent_readback_len: char_len(&independent),
            independent_readback_sha256: text_signature(&independent),
            status: "verified_state".to_owned(),
            elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        })
    }

    fn resolve_bridge_target(
        &self,
        session_id: &str,
        params: &BrowserSetValueParams,
    ) -> Result<(i64, String), ErrorData> {
        let active_target = self.session_target(Some(session_id))?;
        let cdp_target_id = match (params.cdp_target_id.as_ref(), active_target.as_ref()) {
            (Some(target_id), _) => target_id.clone(),
            (None, Some(SessionTarget::Cdp { cdp_target_id, .. })) => cdp_target_id.clone(),
            (None, _) => {
                return Err(mcp_error(
                    error_codes::TARGET_NOT_SET,
                    format!(
                        "{TOOL} requires an active CDP session target or an explicit cdp_target_id owned by this session; refusing the human foreground tab"
                    ),
                ));
            }
        };
        let window_hwnd = params
            .window_hwnd
            .or_else(|| match active_target.as_ref() {
                Some(SessionTarget::Cdp { window_hwnd, .. }) => Some(*window_hwnd),
                Some(SessionTarget::Window { hwnd }) => Some(*hwnd),
                None => None,
            })
            .ok_or_else(|| {
                mcp_error(
                    error_codes::TARGET_NOT_SET,
                    format!("{TOOL} requires window_hwnd when no active session target is set"),
                )
            })?;
        // Ownership: an explicit target must match this session's active CDP
        // target. This keeps the tool from acting on another session's tab.
        if let Some(explicit) = params.cdp_target_id.as_ref() {
            let owned = matches!(
                active_target.as_ref(),
                Some(SessionTarget::Cdp { cdp_target_id, .. })
                    if cdp_target_id.eq_ignore_ascii_case(explicit)
            );
            if !owned {
                return Err(mcp_error(
                    error_codes::ACTION_TARGET_INVALID,
                    format!(
                        "{TOOL} refused target {explicit:?}: it is not this session's active CDP target"
                    ),
                ));
            }
        }
        Ok((window_hwnd, cdp_target_id))
    }
}

#[derive(Debug)]
enum Locator {
    Selector(String),
    ActiveElement,
}

impl Locator {
    fn label(&self) -> &'static str {
        match self {
            Self::Selector(_) => "selector",
            Self::ActiveElement => "active_element",
        }
    }
}

fn validate_locator(params: &BrowserSetValueParams) -> Result<Locator, ErrorData> {
    match (&params.selector, params.active_element) {
        (Some(selector), false) if !selector.trim().is_empty() => {
            Ok(Locator::Selector(selector.clone()))
        }
        (Some(_), false) => Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("{TOOL} selector must be a non-empty CSS selector"),
        )),
        (None, true) => Ok(Locator::ActiveElement),
        (Some(_), true) => Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("{TOOL} requires exactly one of `selector` or `active_element`, not both"),
        )),
        (None, false) => Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("{TOOL} requires a `selector` or `active_element=true`"),
        )),
    }
}

/// Replace verification: newline-normalized exact equality (editable hosts emit
/// `\r\n`/trailing-newline variance that does not change field content).
fn value_matches(observed: &str, requested: &str) -> bool {
    normalize(observed) == normalize(requested)
}

fn normalize(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .strip_suffix('\n')
        .map_or_else(|| normalized.clone(), str::to_owned)
}

fn char_len(value: &str) -> u32 {
    u32::try_from(value.chars().count()).unwrap_or(u32::MAX)
}

fn postcondition_error(
    lens: &str,
    cdp_target_id: &str,
    requested: &str,
    observed: &str,
) -> ErrorData {
    tracing::error!(
        code = error_codes::ACTION_POSTCONDITION_FAILED,
        tool = TOOL,
        lens,
        cdp_target_id,
        requested_len = char_len(requested),
        observed_len = char_len(observed),
        requested_sha256 = %text_signature(requested),
        observed_sha256 = %text_signature(observed),
        "browser_set_value separate readback did not equal the requested replacement text"
    );
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "{TOOL} Source-of-Truth postcondition failed ({lens}): readback does not equal the requested replacement text"
        ),
        Some(json!({
            "code": error_codes::ACTION_POSTCONDITION_FAILED,
            "tool": TOOL,
            "lens": lens,
            "cdp_target_id": cdp_target_id,
            "source_of_truth": SOURCE_OF_TRUTH,
            "verify": {
                "requested_len": char_len(requested),
                "observed_len": char_len(observed),
                "requested_sha256": text_signature(requested),
                "observed_sha256": text_signature(observed),
            },
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(selector: Option<&str>, active: bool) -> BrowserSetValueParams {
        BrowserSetValueParams {
            text: "x".to_owned(),
            selector: selector.map(str::to_owned),
            active_element: active,
            cdp_target_id: None,
            window_hwnd: None,
        }
    }

    #[test]
    fn locator_requires_exactly_one() {
        assert!(matches!(
            validate_locator(&params(Some("#q"), false)),
            Ok(Locator::Selector(_))
        ));
        assert!(matches!(
            validate_locator(&params(None, true)),
            Ok(Locator::ActiveElement)
        ));
        // both
        let err = validate_locator(&params(Some("#q"), true)).expect_err("both must fail");
        assert!(err.message.contains("exactly one"));
        // neither
        let err = validate_locator(&params(None, false)).expect_err("neither must fail");
        assert!(err.message.contains("selector"));
        // empty selector
        let err = validate_locator(&params(Some("  "), false)).expect_err("empty must fail");
        assert!(err.message.contains("non-empty"));
    }

    #[test]
    fn normalize_strips_trailing_newline_and_crlf() {
        assert_eq!(normalize("a\r\nb"), "a\nb");
        assert_eq!(normalize("a\n"), "a");
        assert!(value_matches("composer\n", "composer"));
        assert!(value_matches("", ""));
        assert!(!value_matches("leftover", ""));
    }
}
