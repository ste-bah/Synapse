use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::{ErrorData, RoleServer, model::ErrorCode, model::Tool, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use synapse_action::lease;
use synapse_core::error_codes;
use synapse_storage::cf;

use super::{Json, Parameters, SynapseService, empty_input_schema, mcp_error, tool, tool_router};

const TOOL_PROFILE_PREFIX: &str = "mcp/tool-profile/v1/";
const TOOL_PROFILE_SOURCE_OF_TRUTH: &str = "CF_SESSIONS mcp/tool-profile/v1/<session_id>";
const TOOL_PROFILE_ROW_KIND: &str = "mcp_tool_profile";
const TOOL_PROFILE_SCHEMA_VERSION: u32 = 1;
const MAX_PROFILE_REASON_CHARS: usize = 1024;
pub(crate) const PUBLIC_TOOL_LIMIT: usize = 40;
const PUBLIC_TOOL_REGISTRY_SOURCE_OF_TRUTH: &str =
    "crates/synapse-mcp/src/server/tool_profiles.rs PUBLIC_TOOL_NAMES";
const PUBLIC_TOOL_REGISTRY_OPERATION: &str = "validate_public_tool_registry";
const FACADE_CONTRACT_SOURCE_OF_TRUTH: &str =
    "crates/synapse-mcp/src/server/tool_profiles.rs FACADE_TOOL_CONTRACTS";
const FACADE_CONTRACT_OPERATION: &str = "validate_facade_contract";
const FACADE_CONTRACT_ERROR_CODE: &str = "FACADE_CONTRACT_INVALID";
const FACADE_CONTRACT_STRUCTURED_ERROR: &str = "facade errors must include code, operation, source_of_truth, remediation, and target/source id when applicable";

pub(crate) const PUBLIC_TOOL_NAMES: &[&str] = &[
    "health",
    "profile",
    "session",
    "subscribe",
    "observe",
    "find",
    "read_text",
    "screenshot",
    "target",
    "act",
    "shell",
    "process",
    "browser_tabs",
    "browser_nav",
    "browser_dom",
    "browser_form",
    "browser_wait",
    "browser_capture",
    "browser_storage",
    "browser_debugger",
    "workspace",
    "agent",
    "task",
    "approval",
    "escalation",
    "timeline",
    "episode",
    "routine",
    "assist",
    "reality",
    "verification",
    "storage",
    "model",
    "cost",
    "hygiene",
    "audit",
    "replay",
    "privacy",
    "setup",
    "telemetry",
];

const PUBLIC_TOOL_IMPLEMENTATION_DENYLIST: &[&str] = &[
    "act_click",
    "act_clipboard",
    "act_combo",
    "act_focus_window",
    "act_keymap",
    "act_launch",
    "act_pad",
    "act_press",
    "act_run_shell",
    "act_run_shell_cancel",
    "act_run_shell_start",
    "act_run_shell_status",
    "act_scroll",
    "act_set_field_text",
    "act_set_value",
    "act_spawn_agent",
    "act_stroke",
    "act_type",
    "armed_routine_tick",
    "audit_intelligence_query",
    "browser_screenshot",
    "capture_gif",
    "capture_screenshot",
    "cdp_activate_tab",
    "cdp_bridge_reload",
    "cdp_close_tab",
    "cdp_navigate_tab",
    "cdp_open_tab",
    "cdp_target_info",
    "clear_target",
    "demo_record_start",
    "demo_record_stop",
    "get_target",
    "hidden_desktop_pip_frame",
    "intent_detect_tick",
    "profile_authoring_generate",
    "profile_authoring_inspect",
    "profile_authoring_list",
    "release_all",
    "set_target",
    "storage_gc_once",
    "storage_put_probe_rows",
    "suggestion_tick",
    "target_act",
    "target_claim",
    "target_claim_adopt",
    "target_claim_status",
    "target_release",
    "tool_profile_set",
    "tool_profile_status",
    "window_list",
];

const FACADE_TOOL_CONTRACTS: &[FacadeToolContractSpec] = &[
    facade_contract(
        "health",
        "HealthOperation",
        "daemon health payload + sanitized tools/list surface",
        &[op(
            "status",
            false,
            false,
            "daemon health payload + process/socket SoT",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "read daemon process/socket state, then call health again after fixing the failed subsystem",
        )],
    ),
    facade_contract(
        "profile",
        "ProfileOperation",
        "CF_SESSIONS mcp/tool-profile/v1/<session_id> + sanitized tools/list",
        &[
            op(
                "status",
                false,
                false,
                "CF_SESSIONS policy row + sanitized tools/list",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "read tool_profile_status and compare visible tool names against the profile row",
            ),
            op(
                "set",
                true,
                false,
                "CF_SESSIONS mcp/tool-profile/v1/<session_id>",
                Some("CF_SESSIONS row readback + notifications/tools/list_changed attempt"),
                error_codes::TOOL_PROFILE_POLICY_DENIED,
                "hold the required lease/profile permission, provide reason/confirmation, then retry",
            ),
        ],
    ),
    facade_contract(
        "session",
        "SessionOperation",
        "MCP session registry + CF_SESSIONS session-target rows",
        &[op(
            "list",
            false,
            false,
            "daemon session registry + CF_SESSIONS session-target rows",
            None,
            error_codes::HTTP_SESSION_INVALID,
            "establish an MCP session and retry through the same client transport",
        )],
    ),
    facade_contract(
        "subscribe",
        "SubscribeOperation",
        "SSE subscriber registry + MCP session id",
        &[op(
            "events",
            false,
            false,
            "SSE subscriber registry + emitted event stream",
            None,
            error_codes::HTTP_SESSION_INVALID,
            "open a session-scoped subscription against the live daemon and retry",
        )],
    ),
    facade_contract(
        "observe",
        "ObserveOperation",
        "capture backend readback + perception observation payload",
        &[op(
            "current",
            false,
            true,
            "active session target + capture backend readback",
            None,
            error_codes::CAPTURE_TARGET_INVALID,
            "bind a target or pass an explicit target, then retry observe",
        )],
    ),
    facade_contract(
        "find",
        "FindOperation",
        "perception index over latest observation readback",
        &[op(
            "elements",
            false,
            true,
            "latest observation payload + element index",
            None,
            error_codes::CAPTURE_TARGET_INVALID,
            "capture or bind the intended target before querying elements",
        )],
    ),
    facade_contract(
        "read_text",
        "ReadTextOperation",
        "OCR/accessibility/browser text readback",
        &[op(
            "text",
            false,
            true,
            "target-specific OCR, accessibility, or browser text readback",
            None,
            error_codes::CAPTURE_TARGET_INVALID,
            "bind the text source target and retry with a scoped selector when needed",
        )],
    ),
    facade_contract(
        "screenshot",
        "ScreenshotOperation",
        "capture artifact path + image metadata readback",
        &[
            op(
                "capture",
                false,
                true,
                "still screenshot artifact bytes + target metadata",
                None,
                error_codes::CAPTURE_TARGET_INVALID,
                "bind the target and retry after the capture backend reports healthy",
            ),
            op(
                "gif",
                false,
                true,
                "GIF artifact bytes + target metadata",
                None,
                error_codes::CAPTURE_TARGET_INVALID,
                "bind the target and retry after the capture backend reports healthy",
            ),
        ],
    ),
    facade_contract(
        "target",
        "TargetOperation",
        "MCP session target registry + CF_SESSIONS row",
        &[
            op(
                "get",
                false,
                false,
                "daemon session target registry + CF_SESSIONS row",
                None,
                error_codes::HTTP_SESSION_INVALID,
                "establish an MCP session before reading target state",
            ),
            op(
                "list",
                false,
                false,
                "top-level window snapshot + target claim registry readback",
                None,
                error_codes::ACTION_TARGET_INVALID,
                "refresh the live window list and choose a current target row",
            ),
            op(
                "set",
                true,
                true,
                "daemon session target registry + CF_SESSIONS row",
                Some("CF_SESSIONS session-target row readback + daemon registry readback"),
                error_codes::ACTION_TARGET_INVALID,
                "pass a valid window or CDP target discovered from the live host",
            ),
            op(
                "clear",
                true,
                false,
                "daemon session target registry + CF_SESSIONS row",
                Some("CF_SESSIONS session-target row readback + daemon registry readback"),
                error_codes::HTTP_SESSION_INVALID,
                "establish an MCP session before clearing target state",
            ),
            op(
                "claim",
                true,
                true,
                "target claim registry + CF_ACTION_LOG",
                Some("target claim registry readback + CF_ACTION_LOG command audit row"),
                error_codes::ACTION_TARGET_INVALID,
                "pass a live target from target operation=list, then retry claim",
            ),
            op(
                "status",
                false,
                false,
                "target claim registry readback",
                None,
                error_codes::HTTP_SESSION_INVALID,
                "establish an MCP session before reading target claim status",
            ),
            op(
                "adopt",
                true,
                true,
                "target claim registry + session lifecycle state",
                Some("target claim registry readback + owner session teardown readback"),
                error_codes::TARGET_CLAIM_NOT_FOUND,
                "read target operation=status, then pass a current owner_session_id",
            ),
            op(
                "release",
                true,
                true,
                "target claim registry + CF_ACTION_LOG",
                Some("target claim registry readback + CF_ACTION_LOG command audit row"),
                error_codes::TARGET_CLAIM_NOT_FOUND,
                "pass the claimed target or current session claim before releasing",
            ),
        ],
    ),
    facade_contract(
        "act",
        "ActOperation",
        "target/action audit row + post-action target readback",
        &[
            op(
                "invoke",
                true,
                true,
                "target action preflight + input lease/readback",
                Some("post-action target/UI readback + CF_ACTION_LOG command audit row"),
                error_codes::ACTION_TARGET_INVALID,
                "bind a valid target and acquire any required control lease before retrying",
            ),
            op(
                "foreground",
                true,
                true,
                "foreground input lease + target/action audit row",
                Some("foreground lease/profile restore readback + post-action target/UI readback"),
                error_codes::ACTION_FOREGROUND_LEASE_NOT_HELD,
                "provide a non-empty reason and let the facade acquire/restore the audited foreground lane",
            ),
        ],
    ),
    facade_contract(
        "shell",
        "ShellOperation",
        "durable shell job table + process status readback",
        &[
            op(
                "run",
                true,
                false,
                "shell job registry + process handle",
                Some("shell job row/status readback + output artifact path"),
                error_codes::TOOL_INTERNAL_ERROR,
                "inspect the job row and process state, then retry with corrected command/input",
            ),
            op(
                "status",
                false,
                false,
                "shell job registry + output artifact path",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "read the job id returned by run and verify the artifact path exists",
            ),
        ],
    ),
    facade_contract(
        "process",
        "ProcessOperation",
        "OS process table snapshot",
        &[op(
            "list",
            false,
            false,
            "OS process table snapshot",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "refresh the process table and scope the query by exact pid/path when mutating",
        )],
    ),
    facade_contract(
        "browser_tabs",
        "BrowserTabsOperation",
        "already-open Chrome bridge tabs.query/readback",
        &[
            op(
                "list",
                false,
                false,
                "Chrome bridge tabs.query result + window context",
                None,
                error_codes::ACTION_TARGET_INVALID,
                "use an already-open authenticated Chrome window and retry",
            ),
            op(
                "select",
                true,
                true,
                "Chrome bridge tabs.query result + MCP session target registry",
                Some("session target registry readback after selecting the tab"),
                error_codes::ACTION_TARGET_INVALID,
                "select a cdp_target_id from the current tabs list",
            ),
            op(
                "new",
                true,
                false,
                "Chrome bridge tabs.create result + tabs.query readback",
                Some("new tab id readback from tabs.query"),
                error_codes::TOOL_PARAMS_INVALID,
                "pass a valid URL or empty string for about:blank",
            ),
            op(
                "close",
                true,
                true,
                "Chrome bridge tabs.remove result + tabs.query readback",
                Some("tabs.query absence readback for the closed target"),
                error_codes::ACTION_TARGET_INVALID,
                "close only a tab owned by this MCP session",
            ),
        ],
    ),
    facade_contract(
        "browser_nav",
        "BrowserNavOperation",
        "Chrome bridge navigation result + page URL/readiness readback",
        &[op(
            "navigate",
            true,
            true,
            "Chrome bridge navigation result",
            Some("page URL + readyState readback from the same target"),
            error_codes::TOOL_PARAMS_INVALID,
            "pass a valid target-scoped URL and wait condition",
        )],
    ),
    facade_contract(
        "browser_dom",
        "BrowserDomOperation",
        "target-scoped DOM query/evaluate readback",
        &[op(
            "query",
            false,
            true,
            "target-scoped DOM snapshot",
            None,
            error_codes::ACTION_TARGET_INVALID,
            "bind the intended tab and use a strict selector or element id",
        )],
    ),
    facade_contract(
        "browser_form",
        "BrowserFormOperation",
        "target-scoped DOM form mutation + DOM value readback",
        &[op(
            "set_value",
            true,
            true,
            "target-scoped DOM mutation",
            Some("DOM value/property readback after mutation"),
            error_codes::ACTION_TARGET_INVALID,
            "bind the target and pass a strict selector or element id",
        )],
    ),
    facade_contract(
        "browser_wait",
        "BrowserWaitOperation",
        "target-scoped browser wait condition readback",
        &[op(
            "for_condition",
            false,
            true,
            "target-scoped DOM/URL/load-state readback",
            None,
            error_codes::TOOL_PARAMS_INVALID,
            "use a declared wait condition and bounded timeout",
        )],
    ),
    facade_contract(
        "browser_capture",
        "BrowserCaptureOperation",
        "browser screenshot/content/readback artifacts",
        &[op(
            "screenshot",
            false,
            true,
            "target-scoped screenshot bytes + page metadata",
            None,
            error_codes::ACTION_TARGET_INVALID,
            "bind the tab and retry capture after the bridge reports healthy",
        )],
    ),
    facade_contract(
        "browser_storage",
        "BrowserStorageOperation",
        "browser cookies/local storage/session storage readback",
        &[
            op(
                "read",
                false,
                true,
                "target-scoped browser storage readback",
                None,
                error_codes::ACTION_TARGET_INVALID,
                "bind the tab and request one supported storage namespace",
            ),
            op(
                "write",
                true,
                true,
                "target-scoped browser storage mutation",
                Some("target-scoped browser storage readback after mutation"),
                error_codes::TOOL_PARAMS_INVALID,
                "pass a supported storage namespace/key/value and verify the readback",
            ),
        ],
    ),
    facade_contract(
        "browser_debugger",
        "BrowserDebuggerOperation",
        "explicit browser_debugger profile + raw CDP/chrome.debugger readback",
        &[op(
            "call",
            true,
            true,
            "browser_debugger profile row + raw CDP/chrome.debugger response",
            Some("browser_debugger response/readback for the same target"),
            error_codes::TOOL_PROFILE_POLICY_DENIED,
            "switch to browser_debugger with reason/confirmation before raw debugger calls",
        )],
    ),
    facade_contract(
        "workspace",
        "WorkspaceOperation",
        "CF_KV workspace-blackboard exact rows",
        &[
            op(
                "get",
                false,
                false,
                "CF_KV workspace-blackboard exact row",
                None,
                error_codes::STORAGE_READ_FAILED,
                "read the exact run/key row and handle absent rows explicitly",
            ),
            op(
                "put",
                true,
                false,
                "CF_KV workspace-blackboard exact row",
                Some("CF_KV exact row readback with value hash/version"),
                error_codes::STORAGE_WRITE_FAILED,
                "retry with the observed expected_version or correct the key/value",
            ),
        ],
    ),
    facade_contract(
        "agent",
        "AgentOperation",
        "agent lifecycle registry + CF_AGENT_EVENTS/CF_KV rows",
        &[
            op(
                "list",
                false,
                false,
                "agent lifecycle registry + recent journal rows",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "refresh the live registry and inspect the agent id/session id",
            ),
            op(
                "spawn",
                true,
                false,
                "agent spawn registry + transcript/event rows",
                Some("spawned agent registry row + transcript/event readback"),
                error_codes::TOOL_INTERNAL_ERROR,
                "fix the spawn command/template and retry after reading the event row",
            ),
        ],
    ),
    facade_contract(
        "task",
        "TaskOperation",
        "agent task registry + task event rows",
        &[op(
            "status",
            false,
            false,
            "agent task registry + task event rows",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "read the task id from the registry before retrying task operations",
        )],
    ),
    facade_contract(
        "approval",
        "ApprovalOperation",
        "approval queue rows + decision audit rows",
        &[
            op(
                "list",
                false,
                false,
                "approval queue rows",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "read current approval rows and choose an existing request id",
            ),
            op(
                "decide",
                true,
                false,
                "approval queue row",
                Some("approval decision row readback"),
                error_codes::TOOL_PARAMS_INVALID,
                "decide an existing pending approval id with explicit outcome",
            ),
        ],
    ),
    facade_contract(
        "escalation",
        "EscalationOperation",
        "escalation policy rows + request audit rows",
        &[op(
            "request",
            true,
            false,
            "escalation request row",
            Some("escalation request/audit readback"),
            error_codes::TOOL_PARAMS_INVALID,
            "include the exact operator-only action and current SoT evidence",
        )],
    ),
    facade_contract(
        "timeline",
        "TimelineOperation",
        "timeline event store + query readback",
        &[op(
            "query",
            false,
            false,
            "timeline event store query readback",
            None,
            error_codes::STORAGE_READ_FAILED,
            "narrow the time range/filter and retry after reading storage health",
        )],
    ),
    facade_contract(
        "episode",
        "EpisodeOperation",
        "episode segment/export rows + file artifacts",
        &[op(
            "segment",
            true,
            false,
            "episode segment rows",
            Some("episode row/file artifact readback"),
            error_codes::STORAGE_WRITE_FAILED,
            "retry with a bounded time range and verify the written artifact path",
        )],
    ),
    facade_contract(
        "routine",
        "RoutineOperation",
        "routine registry rows + routine feedback rows",
        &[op(
            "list",
            false,
            false,
            "routine registry rows",
            None,
            error_codes::STORAGE_READ_FAILED,
            "read routine registry storage health and retry with a narrower filter",
        )],
    ),
    facade_contract(
        "assist",
        "AssistOperation",
        "suggestion/routine assist queue + target readback",
        &[op(
            "suggest",
            false,
            true,
            "assist suggestion queue + target readback",
            None,
            error_codes::ACTION_TARGET_INVALID,
            "bind the target and read suggestion queue state before accepting",
        )],
    ),
    facade_contract(
        "reality",
        "RealityOperation",
        "reality baseline/delta/audit rows",
        &[op(
            "audit",
            false,
            false,
            "reality baseline/delta/audit rows",
            None,
            error_codes::STORAGE_READ_FAILED,
            "read the latest baseline and retry the drift audit with a bounded scope",
        )],
    ),
    facade_contract(
        "verification",
        "VerificationOperation",
        "verification claim rows + physical SoT readback references",
        &[op(
            "record",
            true,
            false,
            "verification claim row",
            Some("verification row readback with physical SoT reference"),
            error_codes::STORAGE_WRITE_FAILED,
            "include the physical source-of-truth readback before recording the claim",
        )],
    ),
    facade_contract(
        "storage",
        "StorageOperation",
        "RocksDB CF metadata + exact row readbacks",
        &[op(
            "inspect",
            false,
            false,
            "RocksDB CF metadata + optional exact row readback",
            None,
            error_codes::STORAGE_READ_FAILED,
            "inspect the named CF/key and fix storage health before mutating",
        )],
    ),
    facade_contract(
        "model",
        "ModelOperation",
        "local model registry CF_KV row + probe readback",
        &[
            op(
                "list",
                false,
                false,
                "CF_KV local model registry rows",
                None,
                error_codes::STORAGE_READ_FAILED,
                "read registry storage and probe diagnostics before routing a model",
            ),
            op(
                "update",
                true,
                false,
                "CF_KV local model registry row",
                Some("forced structured tool-call probe row readback"),
                error_codes::TOOL_PARAMS_INVALID,
                "fix endpoint/model/key settings until the real probe passes",
            ),
        ],
    ),
    facade_contract(
        "cost",
        "CostOperation",
        "cost table rows + token event scan/readback",
        &[op(
            "summarize",
            false,
            false,
            "cost table rows + bounded token event scan",
            None,
            error_codes::STORAGE_READ_FAILED,
            "price the missing model or narrow the scan scope before retrying",
        )],
    ),
    facade_contract(
        "hygiene",
        "HygieneOperation",
        "repo/host hygiene report artifacts",
        &[op(
            "report",
            false,
            false,
            "hygiene report artifact + process/worktree readback",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "read the failing hygiene artifact and remediate exact paths/processes",
        )],
    ),
    facade_contract(
        "audit",
        "AuditOperation",
        "daemon log/event rows + audit result rows",
        &[op(
            "query",
            false,
            false,
            "daemon logs + event/audit rows",
            None,
            error_codes::STORAGE_READ_FAILED,
            "query a bounded time range and inspect storage health if rows are missing",
        )],
    ),
    facade_contract(
        "replay",
        "ReplayOperation",
        "demo/replay event rows + replay state artifact",
        &[op(
            "run",
            true,
            false,
            "demo/replay source rows",
            Some("replay state artifact readback"),
            error_codes::STORAGE_READ_FAILED,
            "select an existing recording and verify replay output artifact bytes",
        )],
    ),
    facade_contract(
        "privacy",
        "PrivacyOperation",
        "privacy policy rows + redaction/exclusion readback",
        &[op(
            "redact",
            true,
            false,
            "privacy policy/exclusion rows",
            Some("redaction/exclusion row readback"),
            error_codes::STORAGE_WRITE_FAILED,
            "write a scoped privacy rule and read back the affected row/artifact",
        )],
    ),
    facade_contract(
        "setup",
        "SetupOperation",
        "host setup readback + daemon transport configuration",
        &[op(
            "doctor",
            false,
            false,
            "host setup readback + daemon process/socket state",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "repair the exact missing local prerequisite and read the configured SoT again",
        )],
    ),
    facade_contract(
        "telemetry",
        "TelemetryOperation",
        "telemetry counters/events + public/implementation tool counts",
        &[op(
            "status",
            false,
            false,
            "telemetry counters/events + tool profile snapshot",
            None,
            error_codes::STORAGE_READ_FAILED,
            "read telemetry storage health and compare public/implementation count snapshots",
        )],
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FacadeToolContractSpec {
    tool_name: &'static str,
    operation_enum: &'static str,
    source_of_truth: &'static str,
    operations: &'static [FacadeOperationContractSpec],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FacadeOperationContractSpec {
    operation: &'static str,
    mutates_state: bool,
    target_required: bool,
    source_of_truth: &'static str,
    readback_source_of_truth: Option<&'static str>,
    error_code: &'static str,
    remediation: &'static str,
}

const fn facade_contract(
    tool_name: &'static str,
    operation_enum: &'static str,
    source_of_truth: &'static str,
    operations: &'static [FacadeOperationContractSpec],
) -> FacadeToolContractSpec {
    FacadeToolContractSpec {
        tool_name,
        operation_enum,
        source_of_truth,
        operations,
    }
}

const fn op(
    operation: &'static str,
    mutates_state: bool,
    target_required: bool,
    source_of_truth: &'static str,
    readback_source_of_truth: Option<&'static str>,
    error_code: &'static str,
    remediation: &'static str,
) -> FacadeOperationContractSpec {
    FacadeOperationContractSpec {
        operation,
        mutates_state,
        target_required,
        source_of_truth,
        readback_source_of_truth,
        error_code,
        remediation,
    }
}

const NORMAL_ALLOWED_EXACT: &[&str] = PUBLIC_TOOL_NAMES;
const NORMAL_ALLOWED_PREFIXES: &[&str] = &[];

const BROWSER_CONTROL_ALLOWED_EXACT: &[&str] = &[
    "approval_list",
    "browser_adopt_active_tab",
    "browser_aria_snapshot",
    "browser_assert",
    "browser_batch",
    "browser_clock",
    "browser_cookies",
    "browser_content",
    "browser_downloads",
    "browser_file_upload",
    "browser_fill_form",
    "browser_frames",
    "browser_inspect",
    "browser_locate",
    "browser_page_events",
    "browser_scroll_into_view",
    "browser_screenshot",
    "browser_set_content",
    "browser_set_value",
    "browser_storage",
    "browser_tabs",
    "browser_wait_for",
    "capture_gif",
    "capture_screenshot",
    "cdp_activate_tab",
    "cdp_bridge_reload",
    "cdp_close_tab",
    "cdp_navigate_tab",
    "cdp_open_tab",
    "cdp_target_info",
    "clear_target",
    "control_lease_acquire",
    "control_lease_release",
    "control_lease_status",
    "escalation_list",
    "find",
    "get_target",
    "health",
    "observe",
    "observe_delta",
    "read_text",
    "reality_audit",
    "reality_baseline",
    "session_end",
    "session_list",
    "session_status",
    "set_capture_target",
    "set_perception_mode",
    "set_target",
    "storage_inspect",
    "target_act",
    "target_claim",
    "target_claim_adopt",
    "target_claim_status",
    "target_release",
    "tool_profile_set",
    "tool_profile_status",
    "verification_audit",
    "verification_bind",
    "verification_inbox",
    "verification_poll",
    "verification_sources",
    "window_list",
    "workspace_get",
    "workspace_list",
    "workspace_put",
    "workspace_subscribe",
];

const BROWSER_DEBUGGER_ONLY_EXACT: &[&str] = &[
    "browser_add_init_script",
    "browser_add_script_tag",
    "browser_add_style_tag",
    "browser_console_messages",
    "browser_drag",
    "browser_drop",
    "browser_emulate",
    "browser_evaluate",
    "browser_expose_binding",
    "browser_handle_dialog",
    "browser_network",
    "browser_network_har",
    "browser_network_overrides",
    "browser_pdf",
    "browser_route",
];

const BROWSER_DEBUGGER_ALLOWED_EXACT: &[&str] = &[
    "approval_list",
    "browser_add_init_script",
    "browser_add_script_tag",
    "browser_add_style_tag",
    "browser_adopt_active_tab",
    "browser_aria_snapshot",
    "browser_assert",
    "browser_batch",
    "browser_clock",
    "browser_console_messages",
    "browser_cookies",
    "browser_content",
    "browser_downloads",
    "browser_drag",
    "browser_drop",
    "browser_emulate",
    "browser_evaluate",
    "browser_expose_binding",
    "browser_file_upload",
    "browser_fill_form",
    "browser_frames",
    "browser_handle_dialog",
    "browser_inspect",
    "browser_locate",
    "browser_network",
    "browser_network_har",
    "browser_network_overrides",
    "browser_page_events",
    "browser_pdf",
    "browser_route",
    "browser_scroll_into_view",
    "browser_screenshot",
    "browser_set_content",
    "browser_set_value",
    "browser_storage",
    "browser_tabs",
    "browser_wait_for",
    "capture_gif",
    "capture_screenshot",
    "cdp_activate_tab",
    "cdp_bridge_reload",
    "cdp_close_tab",
    "cdp_navigate_tab",
    "cdp_open_tab",
    "cdp_target_info",
    "clear_target",
    "control_lease_acquire",
    "control_lease_release",
    "control_lease_status",
    "escalation_list",
    "find",
    "get_target",
    "health",
    "observe",
    "observe_delta",
    "read_text",
    "reality_audit",
    "reality_baseline",
    "session_end",
    "session_list",
    "session_status",
    "set_capture_target",
    "set_perception_mode",
    "set_target",
    "storage_inspect",
    "target_act",
    "target_claim",
    "target_claim_adopt",
    "target_claim_status",
    "target_release",
    "tool_profile_set",
    "tool_profile_status",
    "verification_audit",
    "verification_bind",
    "verification_inbox",
    "verification_poll",
    "verification_sources",
    "window_list",
    "workspace_get",
    "workspace_list",
    "workspace_put",
    "workspace_subscribe",
];

const BREAK_GLASS_HAZARDOUS_TOOLS: &[&str] = &[
    "act_click",
    "act_clipboard",
    "act_combo",
    "act_focus_window",
    "act_keymap",
    "act_pad",
    "act_press",
    "act_scroll",
    "act_set_field_text",
    "act_set_value",
    "act_stroke",
    "act_type",
    "hidden_desktop_pip_frame",
    "release_all",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolProfileKind {
    NormalAgent,
    BrowserControl,
    /// Browser-only CDP / chrome.debugger capability lane. This keeps
    /// attach-capable browser tools explicit without exposing raw OS foreground,
    /// shell, or agent-spawn surfaces.
    BrowserDebugger,
    BreakGlass,
    /// Full Synapse tool surface for Synapse-spawned local-model agents
    /// (gemma/DeepSeek/etc., #1031). Identical visibility to `BreakGlass` (every
    /// real tool, including the foreground input primitives) but assigned
    /// automatically to the trusted local-model harness instead of requiring an
    /// explicit operator-held foreground lease. Local models operate the machine
    /// and must never be missing a tool; foreground contention is handled at
    /// action time by the per-action lease/target guards (#717/#999), not by
    /// hiding the tools from discovery.
    FullCapability,
}

impl ToolProfileKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NormalAgent => "normal_agent",
            Self::BrowserControl => "browser_control",
            Self::BrowserDebugger => "browser_debugger",
            Self::BreakGlass => "break_glass",
            Self::FullCapability => "full_capability",
        }
    }

    /// Whether this profile is permitted to reach the real human OS foreground
    /// input tier. Normal/task profiles still preserve foreground-equivalent
    /// capability through `agent_logical_foreground` / `foreground_lane` routes;
    /// only the serialized real-foreground tier needs break-glass/full-capability
    /// proof (#999/#1219).
    pub(crate) const fn allows_foreground_tier(self) -> bool {
        matches!(self, Self::BreakGlass | Self::FullCapability)
    }

    fn label(self) -> &'static str {
        match self {
            Self::NormalAgent => "normal_agent",
            Self::BrowserControl => "dashboard/browser-control task",
            Self::BrowserDebugger => "browser-debugger task",
            Self::BreakGlass => "break-glass/admin",
            Self::FullCapability => "full-capability local-model agent",
        }
    }

    fn is_visible(self, tool_name: &str) -> bool {
        match self {
            Self::BreakGlass | Self::FullCapability => true,
            Self::NormalAgent => {
                NORMAL_ALLOWED_EXACT.contains(&tool_name)
                    || NORMAL_ALLOWED_PREFIXES
                        .iter()
                        .any(|prefix| tool_name.starts_with(prefix))
            }
            Self::BrowserControl => BROWSER_CONTROL_ALLOWED_EXACT.contains(&tool_name),
            Self::BrowserDebugger => BROWSER_DEBUGGER_ALLOWED_EXACT.contains(&tool_name),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PersistedToolProfile {
    schema_version: u32,
    row_kind: String,
    session_id: String,
    profile: ToolProfileKind,
    source: String,
    reason: Option<String>,
    set_by_session_id: Option<String>,
    stored_at_unix_ms: u64,
    allowed_tool_count: usize,
    allowed_tool_sha256: String,
    denied_break_glass_tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileAssignment {
    pub schema_version: u32,
    pub row_kind: String,
    pub session_id: String,
    pub profile: ToolProfileKind,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_by_session_id: Option<String>,
    pub stored_at_unix_ms: u64,
    pub allowed_tool_count: usize,
    pub allowed_tool_sha256: String,
    pub denied_break_glass_tools: Vec<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileRowReadback {
    pub cf_name: &'static str,
    pub key_hex: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
    pub record: ToolProfileAssignment,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileAuditReadback {
    pub cf_name: &'static str,
    pub key_hex: String,
    pub value_len_bytes: u64,
    pub value_sha256: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicToolRegistrySnapshot {
    pub source_of_truth: &'static str,
    pub max_public_tool_count: usize,
    pub public_tool_count: usize,
    pub public_tool_sha256: String,
    pub public_tool_names: Vec<String>,
    pub implementation_tool_count: usize,
    pub registered_tools_present: Vec<String>,
    pub registered_tools_missing: Vec<String>,
    pub duplicate_public_tool_names: Vec<String>,
    pub forbidden_public_tool_names: Vec<String>,
    pub over_limit_by: usize,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacadeOperationContractSnapshot {
    pub operation: &'static str,
    pub mutates_state: bool,
    pub target_required: bool,
    pub source_of_truth: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readback_source_of_truth: Option<&'static str>,
    pub error_code: &'static str,
    pub remediation: &'static str,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacadeToolContractSnapshot {
    pub tool_name: &'static str,
    pub operation_enum: &'static str,
    pub source_of_truth: &'static str,
    pub operations: Vec<FacadeOperationContractSnapshot>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FacadeContractSnapshot {
    pub source_of_truth: &'static str,
    pub structured_error_contract: &'static str,
    pub public_tool_count: usize,
    pub contract_tool_count: usize,
    pub operation_count: usize,
    pub mutating_operation_count: usize,
    pub facade_contract_sha256: String,
    pub contract_tool_names: Vec<String>,
    pub missing_contract_tool_names: Vec<String>,
    pub unknown_contract_tool_names: Vec<String>,
    pub duplicate_contract_tool_names: Vec<String>,
    pub duplicate_operation_names: Vec<String>,
    pub invalid_contract_reasons: Vec<String>,
    pub contracts: Vec<FacadeToolContractSnapshot>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileSnapshot {
    pub source_of_truth: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub profile: ToolProfileKind,
    pub profile_label: &'static str,
    pub source: String,
    pub implementation_tool_count: usize,
    pub visible_tool_count: usize,
    pub visible_tool_sha256: String,
    pub visible_tool_names: Vec<String>,
    pub denied_break_glass_tools: Vec<String>,
    pub foreground_capability: ToolProfileForegroundCapability,
    pub hidden_tool_routes: Vec<HiddenToolCapabilityRoute>,
    pub public_tool_registry: PublicToolRegistrySnapshot,
    pub facade_contract: FacadeContractSnapshot,
    /// #1352: this session's CURRENT readiness for the real OS-foreground route —
    /// whether it already holds the lease + a break_glass profile, and the exact
    /// remaining steps. Lets an agent preflight the foreground route before
    /// attempting a foreground-only action instead of discovering the gate by trial.
    pub foreground_route: ToolProfileForegroundRoute,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_row: Option<ToolProfileRowReadback>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileForegroundRoute {
    /// True when this session currently owns the foreground input lease.
    pub holds_foreground_lease: bool,
    /// True when the current profile already exposes raw OS-foreground primitives.
    pub profile_allows_foreground: bool,
    /// True when a real OS-foreground action can run right now (lease held AND a
    /// foreground-capable profile) — no further escalation needed.
    pub foreground_route_ready: bool,
    /// The exact ordered steps still required to reach a runnable foreground
    /// action; empty when `foreground_route_ready`.
    pub remaining_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileForegroundCapability {
    pub source_of_truth: &'static str,
    pub profile_preserves_capability: bool,
    pub human_os_foreground: &'static str,
    pub agent_logical_foreground: &'static str,
    pub preferred_path: &'static str,
    pub real_os_foreground_path: &'static str,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HiddenToolCapabilityRoute {
    pub hidden_tool: String,
    pub status: &'static str,
    pub preferred_tools: Vec<String>,
    pub agent_logical_foreground_policy: &'static str,
    pub human_os_foreground_policy: &'static str,
    pub break_glass_policy: &'static str,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileStatusResponse {
    pub snapshot: ToolProfileSnapshot,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileSetParams {
    pub profile: ToolProfileKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    #[schemars(default)]
    pub confirm_break_glass: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProfileOperation {
    Status,
    Set,
}

impl ProfileOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Set => "set",
        }
    }
}

const fn default_profile_operation() -> ProfileOperation {
    ProfileOperation::Status
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileParams {
    #[serde(default = "default_profile_operation")]
    #[schemars(default = "default_profile_operation")]
    pub operation: ProfileOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<ToolProfileKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    #[schemars(default)]
    pub confirm_break_glass: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfileResponse {
    pub operation: ProfileOperation,
    pub source_of_truth: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolProfileStatusResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<ToolProfileSetResponse>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileLeaseProof {
    pub required: bool,
    pub held: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    pub caller_is_owner: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolProfileSetResponse {
    pub before: ToolProfileSnapshot,
    pub after: ToolProfileSnapshot,
    pub row_readback: ToolProfileRowReadback,
    pub intent_audit: ToolProfileAuditReadback,
    pub final_audit: ToolProfileAuditReadback,
    pub lease_proof: ToolProfileLeaseProof,
}

#[tool_router(router = tool_profile_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Public profile facade. operation=status reads this MCP session's effective profile, visible public facade tools, durable CF_SESSIONS policy row, and facade contract. operation=set persists a new profile through the same audited readback path as tool_profile_set; explicit advanced profiles require confirm_break_glass=true and a non-empty reason, and break_glass/full_capability also require the foreground input lease."
    )]
    pub async fn profile(
        &self,
        params: Parameters<ProfileParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ProfileResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "profile",
            operation = params.0.operation.as_str(),
            "tool.invocation kind=profile"
        );
        let params = params.0;
        match params.operation {
            ProfileOperation::Status => Ok(Json(ProfileResponse {
                operation: ProfileOperation::Status,
                source_of_truth: TOOL_PROFILE_SOURCE_OF_TRUTH,
                status: Some(self.tool_profile_status_response(&request_context)?),
                set: None,
            })),
            ProfileOperation::Set => {
                let profile = params.profile.ok_or_else(|| {
                    profile_facade_error(
                        ProfileOperation::Set,
                        "profile operation=set requires profile",
                        "pass profile=normal_agent, browser_control, browser_debugger, break_glass, or full_capability",
                    )
                })?;
                let set = self
                    .tool_profile_set_response(
                        ToolProfileSetParams {
                            profile,
                            reason: params.reason,
                            confirm_break_glass: params.confirm_break_glass,
                        },
                        &request_context,
                        "profile",
                        "set",
                    )
                    .await?;
                Ok(Json(ProfileResponse {
                    operation: ProfileOperation::Set,
                    source_of_truth: TOOL_PROFILE_SOURCE_OF_TRUTH,
                    status: None,
                    set: Some(set),
                }))
            }
        }
    }

    #[tool(
        description = "Read this MCP session's effective tool profile, visible tools/list names, durable CF_SESSIONS policy row, and capability-preserving routes for hidden raw foreground/browser-debugger primitives. The readback distinguishes human_os_foreground from agent_logical_foreground and the browser debugger lane: normal_agent/browser_control expose debugger-free already-open Chrome routes, browser_debugger explicitly exposes raw-CDP/chrome.debugger browser tools, and real OS foreground primitives stay reachable only through lease + break_glass.",
        input_schema = empty_input_schema()
    )]
    pub async fn tool_profile_status(
        &self,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ToolProfileStatusResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "tool_profile_status",
            "tool.invocation kind=tool_profile_status"
        );
        self.tool_profile_status_response(&request_context)
            .map(Json)
    }

    #[tool(
        description = "Set this MCP session's durable tool profile. normal_agent and browser_control expose debugger-free already-open Chrome routes and hide raw-CDP/chrome.debugger browser tools from default discovery. browser_debugger exposes browser-only raw-CDP/chrome.debugger tools only when confirm_break_glass=true and reason is non-empty. break_glass exposes the full raw surface only when confirm_break_glass=true, reason is non-empty, and this session currently owns the foreground input lease."
    )]
    pub async fn tool_profile_set(
        &self,
        params: Parameters<ToolProfileSetParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<ToolProfileSetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "tool_profile_set",
            "tool.invocation kind=tool_profile_set"
        );
        self.tool_profile_set_response(
            params.0,
            &request_context,
            "tool_profile_set",
            "profile_set",
        )
        .await
        .map(Json)
    }
}

impl SynapseService {
    fn tool_profile_status_response(
        &self,
        request_context: &RequestContext<RoleServer>,
    ) -> Result<ToolProfileStatusResponse, ErrorData> {
        let session_id = super::context::mcp_session_id_from_request_context(request_context)?;
        Ok(ToolProfileStatusResponse {
            snapshot: self.tool_profile_snapshot(session_id.as_deref())?,
        })
    }

    async fn tool_profile_set_response(
        &self,
        params: ToolProfileSetParams,
        request_context: &RequestContext<RoleServer>,
        audit_tool: &'static str,
        audit_verb: &'static str,
    ) -> Result<ToolProfileSetResponse, ErrorData> {
        let session_id = super::context::mcp_session_id_from_request_context(request_context)?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::HTTP_SESSION_INVALID,
                    format!(
                        "{audit_tool} operation={audit_verb} requires an MCP session id so the policy decision can be persisted"
                    ),
                )
            })?;
        let reason = normalize_reason(params.reason.as_deref())?;
        let before = self.tool_profile_snapshot(Some(&session_id))?;
        let lease_proof = break_glass_lease_proof(&session_id, params.profile);
        let command_payload = json!({
            "requested_profile": params.profile.as_str(),
            "reason": reason,
            "confirm_break_glass": params.confirm_break_glass,
            "operation": audit_verb,
        });
        let command_before = json!({
            "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
            "before_profile": before.profile.as_str(),
            "before_visible_tool_count": before.visible_tool_count,
            "lease_proof": lease_proof,
        });
        let intent_audit = audit_readback(self.command_audit_intent(
            super::command_audit::CommandAuditInput::mcp(
                audit_tool,
                audit_verb,
                Some(session_id.clone()),
                Some(session_id.clone()),
                command_payload.clone(),
                command_before.clone(),
                Value::Null,
                "pending",
            ),
        )?);

        if let Err(error) = validate_profile_set_policy(
            &session_id,
            params.profile,
            reason.as_deref(),
            params.confirm_break_glass,
            &lease_proof,
        ) {
            let final_audit = self.command_audit_final(
                super::command_audit::CommandAuditInput::mcp(
                    audit_tool,
                    audit_verb,
                    Some(session_id.clone()),
                    Some(session_id.clone()),
                    command_payload,
                    command_before,
                    json!({
                        "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
                        "after_profile": before.profile.as_str(),
                        "lease_proof": lease_proof,
                    }),
                    "error",
                )
                .with_error(
                    super::command_audit::command_audit_error_from_error_data(&error),
                ),
            )?;
            let _final_audit = audit_readback(final_audit);
            return Err(error);
        }

        let row_readback = match self.write_tool_profile_assignment(
            &session_id,
            params.profile,
            audit_tool,
            reason.clone(),
            Some(session_id.clone()),
        ) {
            Ok(row) => row,
            Err(error) => {
                let final_audit = self.command_audit_final(
                    super::command_audit::CommandAuditInput::mcp(
                        audit_tool,
                        audit_verb,
                        Some(session_id.clone()),
                        Some(session_id.clone()),
                        command_payload,
                        command_before,
                        json!({
                            "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
                            "after_profile": before.profile.as_str(),
                            "lease_proof": lease_proof,
                        }),
                        "error",
                    )
                    .with_error(
                        super::command_audit::command_audit_error_from_error_data(&error),
                    ),
                )?;
                let _final_audit = audit_readback(final_audit);
                return Err(error);
            }
        };
        let after = self.tool_profile_snapshot(Some(&session_id))?;

        // #1020: the visible tool surface for this MCP session just changed.
        // Push a `notifications/tools/list_changed` so the client refetches
        // `tools/list` within the same session. The CF_SESSIONS row is already
        // the durable source of truth, so notification failure is logged loudly
        // and returned only through the daemon logs.
        if before.visible_tool_sha256 != after.visible_tool_sha256 {
            match request_context.peer.notify_tool_list_changed().await {
                Ok(()) => {
                    tracing::info!(
                        code = "MCP_TOOL_LIST_CHANGED_NOTIFIED",
                        session_id = %session_id,
                        tool = audit_tool,
                        operation = audit_verb,
                        before_profile = before.profile.as_str(),
                        after_profile = after.profile.as_str(),
                        before_visible_tool_count = before.visible_tool_count,
                        after_visible_tool_count = after.visible_tool_count,
                        "profile tool pushed notifications/tools/list_changed after a visible tool-surface change"
                    );
                }
                Err(notify_err) => {
                    tracing::error!(
                        code = "MCP_TOOL_LIST_CHANGED_NOTIFY_FAILED",
                        session_id = %session_id,
                        tool = audit_tool,
                        operation = audit_verb,
                        before_profile = before.profile.as_str(),
                        after_profile = after.profile.as_str(),
                        error = %notify_err,
                        "profile tool persisted the new profile but failed to push notifications/tools/list_changed; the client may need to reconnect to observe the updated tool surface"
                    );
                }
            }
        }

        let final_audit = audit_readback(self.command_audit_final(
            super::command_audit::CommandAuditInput::mcp(
                audit_tool,
                audit_verb,
                Some(session_id.clone()),
                Some(session_id),
                command_payload,
                command_before,
                json!({
                    "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
                    "after_profile": after.profile.as_str(),
                    "after_visible_tool_count": after.visible_tool_count,
                    "row_readback": row_readback,
                    "lease_proof": lease_proof,
                }),
                "ok",
            ),
        )?);

        Ok(ToolProfileSetResponse {
            before,
            after,
            row_readback,
            intent_audit,
            final_audit,
            lease_proof,
        })
    }

    pub(crate) fn tools_for_session_profile(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<Tool>, ErrorData> {
        let snapshot = self.tool_profile_snapshot(session_id)?;
        let mut tools = self.full_sanitized_tools();
        if session_id.is_some() {
            tools.retain(|tool| snapshot.profile.is_visible(tool.name.as_ref()));
        }
        sort_tools_for_profile(&mut tools, snapshot.profile);
        Ok(tools)
    }

    pub(crate) fn tool_profile_snapshot(
        &self,
        session_id: Option<&str>,
    ) -> Result<ToolProfileSnapshot, ErrorData> {
        let full_tool_names = self.full_tool_names();
        let implementation_tool_count = full_tool_names.len();
        let public_tool_registry = public_tool_registry_snapshot_for(&full_tool_names)?;
        let facade_contract =
            facade_contract_snapshot_for(&public_tool_registry.public_tool_names)?;
        let (profile, source, policy_row) = match session_id {
            Some(session_id) => {
                let row = self.ensure_tool_profile_assignment(session_id)?;
                (row.record.profile, row.record.source.clone(), Some(row))
            }
            None => (
                ToolProfileKind::BreakGlass,
                "unscoped_stdio_admin".to_owned(),
                None,
            ),
        };
        let visible_tool_names = if session_id.is_some() {
            visible_tool_names_for_profile(profile, &full_tool_names)
        } else {
            full_tool_names
        };
        let visible_tool_sha256 = sha256_json_hex(&visible_tool_names)?;
        let denied_break_glass_tools = denied_break_glass_tools(&visible_tool_names);
        let hidden_tool_routes = hidden_tool_capability_routes(&visible_tool_names);
        Ok(ToolProfileSnapshot {
            source_of_truth: TOOL_PROFILE_SOURCE_OF_TRUTH,
            session_id: session_id.map(ToOwned::to_owned),
            profile,
            profile_label: profile.label(),
            source,
            implementation_tool_count,
            visible_tool_count: visible_tool_names.len(),
            visible_tool_sha256,
            visible_tool_names,
            denied_break_glass_tools,
            foreground_capability: foreground_capability_policy(profile),
            hidden_tool_routes,
            public_tool_registry,
            facade_contract,
            foreground_route: foreground_route_readiness(session_id, profile),
            policy_row,
        })
    }

    pub(crate) fn public_tool_registry_snapshot(
        &self,
    ) -> Result<PublicToolRegistrySnapshot, ErrorData> {
        public_tool_registry_snapshot_for(&self.full_tool_names())
    }

    pub(crate) fn facade_contract_snapshot() -> Result<FacadeContractSnapshot, ErrorData> {
        facade_contract_snapshot_for(&public_tool_names())
    }

    pub(crate) fn admit_tool_call_for_profile(
        &self,
        tool_name: &str,
        session_id: Option<&str>,
    ) -> Result<(), ErrorData> {
        let Some(session_id) = session_id else {
            return Ok(());
        };
        let full_tool_names = self.full_tool_names();
        if !full_tool_names.iter().any(|name| name == tool_name) {
            return Ok(());
        }
        let row = self.ensure_tool_profile_assignment(session_id)?;
        if row.record.profile.is_visible(tool_name) {
            return Ok(());
        }
        let visible_tool_names =
            visible_tool_names_for_profile(row.record.profile, &full_tool_names);
        let capability_route = hidden_tool_capability_route(tool_name);
        let error = ErrorData::new(
            ErrorCode(-32099),
            format!(
                "tool {tool_name:?} is hidden by MCP tool profile {} for session {session_id}",
                row.record.profile.as_str()
            ),
            Some(json!({
                "code": error_codes::TOOL_PROFILE_POLICY_DENIED,
                "tool": tool_name,
                "session_id": session_id,
                "profile": row.record.profile.as_str(),
                "profile_label": row.record.profile.label(),
                "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
                "policy_row": row,
                "visible_tool_count": visible_tool_names.len(),
                "capability_route": capability_route,
                "resolution": "use the named capability_route preferred tools for default agent work; call profile operation=set profile=browser_debugger with confirm_break_glass=true plus a non-empty reason for browser raw-CDP/chrome.debugger instrumentation; acquire the foreground input lease and call profile operation=set profile=break_glass with confirm_break_glass=true plus a non-empty reason for real human OS foreground work",
            })),
        );
        let command_payload = json!({
            "requested_tool": tool_name,
            "profile": row.record.profile.as_str(),
        });
        let command_before = json!({
            "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
            "policy_row": row,
            "visible_tool_count": visible_tool_names.len(),
        });
        self.command_audit_final(
            super::command_audit::CommandAuditInput::mcp(
                "tool_profile_policy",
                "tool_call_denied",
                Some(session_id.to_owned()),
                Some(session_id.to_owned()),
                command_payload,
                command_before,
                json!({
                    "source_of_truth": "CF_ACTION_LOG command_audit row",
                    "denied_tool": tool_name,
                    "capability_route": hidden_tool_capability_route(tool_name),
                }),
                "error",
            )
            .with_error(super::command_audit::command_audit_error_from_error_data(
                &error,
            )),
        )?;
        Err(error)
    }

    fn ensure_tool_profile_assignment(
        &self,
        session_id: &str,
    ) -> Result<ToolProfileRowReadback, ErrorData> {
        let is_local_agent = self.session_is_local_model_agent(session_id);
        match self.read_tool_profile_assignment(session_id)? {
            Some(row) => {
                // Self-heal: if a session was first seen before its MCP
                // initialize client identity landed in the registry, it may have
                // been written the least-privilege `default_normal_agent` row.
                // Once the registry classifies it as the trusted local-model
                // harness, upgrade it to the full-capability surface so the
                // local model is never left without the input primitives (#1031).
                // Only the *default* normal-agent row self-heals; an explicit
                // operator profile choice is never silently widened.
                if is_local_agent && row.record.source == "default_normal_agent" {
                    return self.write_tool_profile_assignment(
                        session_id,
                        ToolProfileKind::FullCapability,
                        "default_full_capability_local_agent",
                        None,
                        None,
                    );
                }
                if !self.tool_profile_assignment_surface_is_current(&row.record)? {
                    return self.write_tool_profile_assignment(
                        session_id,
                        row.record.profile,
                        row.record.source.clone(),
                        row.record.reason.clone(),
                        row.record.set_by_session_id.clone(),
                    );
                }
                Ok(row)
            }
            None => {
                let (profile, source) = if is_local_agent {
                    (
                        ToolProfileKind::FullCapability,
                        "default_full_capability_local_agent",
                    )
                } else {
                    (ToolProfileKind::NormalAgent, "default_normal_agent")
                };
                self.write_tool_profile_assignment(session_id, profile, source, None, None)
            }
        }
    }

    fn tool_profile_assignment_surface_is_current(
        &self,
        record: &ToolProfileAssignment,
    ) -> Result<bool, ErrorData> {
        let full_tool_names = self.full_tool_names();
        let allowed_tool_names = visible_tool_names_for_profile(record.profile, &full_tool_names);
        Ok(record.allowed_tool_count == allowed_tool_names.len()
            && record.allowed_tool_sha256 == sha256_json_hex(&allowed_tool_names)?
            && record.denied_break_glass_tools == denied_break_glass_tools(&allowed_tool_names))
    }

    /// True when `session_id` belongs to a Synapse-spawned local-model agent
    /// (the `synapse-local-model-agent` MCP client, classified `"local-model"`
    /// by [`super::session_registry::infer_agent_kind`]). Trust basis: Synapse is
    /// loopback-only + bearer-token + single-user, so the MCP client identity is
    /// a sound affordance signal here (it is NOT a cross-tenant security boundary;
    /// `clientInfo` is self-reported per MCP). Unknown / unclassified sessions
    /// return false and get the least-privilege default.
    fn session_is_local_model_agent(&self, session_id: &str) -> bool {
        self.session_registry_ref()
            .lock()
            .ok()
            .and_then(|registry| registry.agent_kind_for(session_id))
            .as_deref()
            == Some("local-model")
    }

    fn read_tool_profile_assignment(
        &self,
        session_id: &str,
    ) -> Result<Option<ToolProfileRowReadback>, ErrorData> {
        let db = self.m3_storage()?;
        let key = tool_profile_key(session_id);
        let rows = db
            .scan_cf_prefix(cf::CF_SESSIONS, &key)
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let Some((read_key, value)) = rows.into_iter().find(|(row_key, _)| row_key == &key) else {
            return Ok(None);
        };
        let persisted =
            synapse_storage::decode_json::<PersistedToolProfile>(&value).map_err(|error| {
                mcp_error(
                    error.code(),
                    format!("decode tool profile row failed for {session_id}: {error}"),
                )
            })?;
        if persisted.schema_version != TOOL_PROFILE_SCHEMA_VERSION
            || persisted.row_kind != TOOL_PROFILE_ROW_KIND
            || persisted.session_id != session_id
        {
            return Err(mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!(
                    "tool profile row mismatch for {session_id}: schema_version={} row_kind={} row_session_id={}",
                    persisted.schema_version, persisted.row_kind, persisted.session_id
                ),
            ));
        }
        let record = ToolProfileAssignment {
            schema_version: persisted.schema_version,
            row_kind: persisted.row_kind,
            session_id: persisted.session_id,
            profile: persisted.profile,
            source: persisted.source,
            reason: persisted.reason,
            set_by_session_id: persisted.set_by_session_id,
            stored_at_unix_ms: persisted.stored_at_unix_ms,
            allowed_tool_count: persisted.allowed_tool_count,
            allowed_tool_sha256: persisted.allowed_tool_sha256,
            denied_break_glass_tools: persisted.denied_break_glass_tools,
        };
        Ok(Some(ToolProfileRowReadback {
            cf_name: cf::CF_SESSIONS,
            key_hex: hex_lower(&read_key),
            value_len_bytes: value.len() as u64,
            value_sha256: sha256_hex(&value),
            record,
        }))
    }

    fn write_tool_profile_assignment(
        &self,
        session_id: &str,
        profile: ToolProfileKind,
        source: impl Into<String>,
        reason: Option<String>,
        set_by_session_id: Option<String>,
    ) -> Result<ToolProfileRowReadback, ErrorData> {
        let full_tool_names = self.full_tool_names();
        let allowed_tool_names = visible_tool_names_for_profile(profile, &full_tool_names);
        let allowed_tool_sha256 = sha256_json_hex(&allowed_tool_names)?;
        let record = ToolProfileAssignment {
            schema_version: TOOL_PROFILE_SCHEMA_VERSION,
            row_kind: TOOL_PROFILE_ROW_KIND.to_owned(),
            session_id: session_id.to_owned(),
            profile,
            source: source.into(),
            reason,
            set_by_session_id,
            stored_at_unix_ms: unix_ms_now(),
            allowed_tool_count: allowed_tool_names.len(),
            allowed_tool_sha256,
            denied_break_glass_tools: denied_break_glass_tools(&allowed_tool_names),
        };
        let encoded = synapse_storage::encode_json(&record).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode tool profile row failed for {session_id}: {error}"),
            )
        })?;
        let db = self.m3_storage()?;
        let key = tool_profile_key(session_id);
        db.put_batch_pressure_bypass(cf::CF_SESSIONS, [(key.clone(), encoded.clone())])
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let readback = self
            .read_tool_profile_assignment(session_id)?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_CORRUPTED,
                    format!("tool profile row missing after write for {session_id}"),
                )
            })?;
        if readback.value_sha256 != sha256_hex(&encoded) {
            return Err(mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!("tool profile row readback hash mismatch for {session_id}"),
            ));
        }
        tracing::info!(
            code = "MCP_TOOL_PROFILE_PERSISTED",
            session_id,
            profile = profile.as_str(),
            visible_tool_count = readback.record.allowed_tool_count,
            key_hex = %readback.key_hex,
            "persisted MCP tool profile to CF_SESSIONS"
        );
        Ok(readback)
    }

    fn full_sanitized_tools(&self) -> Vec<Tool> {
        super::schema_sanitize::sanitize_tools(self.tool_router.list_all())
    }

    fn full_tool_names(&self) -> Vec<String> {
        let mut names = self
            .full_sanitized_tools()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}

fn validate_profile_set_policy(
    session_id: &str,
    profile: ToolProfileKind,
    reason: Option<&str>,
    confirm_break_glass: bool,
    lease_proof: &ToolProfileLeaseProof,
) -> Result<(), ErrorData> {
    if !matches!(
        profile,
        ToolProfileKind::BrowserDebugger
            | ToolProfileKind::BreakGlass
            | ToolProfileKind::FullCapability
    ) {
        return Ok(());
    }
    let profile_label = profile.as_str();
    if !confirm_break_glass {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("explicit profile={profile_label} requires confirm_break_glass=true"),
        ));
    }
    if reason.is_none_or(str::is_empty) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("explicit profile={profile_label} requires a non-empty reason"),
        ));
    }
    if profile == ToolProfileKind::BrowserDebugger {
        return Ok(());
    }
    // The full raw surface (break_glass) and the local-agent full-capability
    // surface, when requested *explicitly* via tool_profile_set, require
    // foreground-lease proof. This stops any agent from self-escalating to raw
    // foreground primitives by hand. The frictionless path to full_capability is
    // the automatic, client-identity-keyed default for the trusted local-model
    // harness (see `ensure_tool_profile_assignment`), never this tool.
    if !lease_proof.caller_is_owner {
        return Err(ErrorData::new(
            ErrorCode(-32099),
            format!(
                "explicit profile={profile_label} requires this MCP session to own the foreground input lease; current owner={:?}",
                lease_proof.owner_session_id
            ),
            Some(json!({
                "code": error_codes::ACTION_FOREGROUND_LEASE_NOT_HELD,
                "session_id": session_id,
                "profile": profile.as_str(),
                "lease_proof": lease_proof,
                "resolution": "call control_lease_acquire first, then retry profile operation=set or tool_profile_set with confirm_break_glass=true and a reason",
            })),
        ));
    }
    Ok(())
}

fn break_glass_lease_proof(session_id: &str, profile: ToolProfileKind) -> ToolProfileLeaseProof {
    let status = lease::status();
    ToolProfileLeaseProof {
        required: matches!(
            profile,
            ToolProfileKind::BreakGlass | ToolProfileKind::FullCapability
        ),
        // FullCapability is auto-assigned to the trusted local-model harness and
        // does not gate on the foreground lease; only an *explicit*
        // tool_profile_set escalation (handled in validate_profile_set_policy)
        // is lease-gated. See [`validate_profile_set_policy`].
        held: status.held,
        owner_session_id: status.owner_session_id.clone(),
        caller_is_owner: status.owner_session_id.as_deref() == Some(session_id),
        expires_in_ms: status.expires_in_ms,
    }
}

fn normalize_reason(raw: Option<&str>) -> Result<Option<String>, ErrorData> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.chars().count() > MAX_PROFILE_REASON_CHARS {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("tool profile reason must be at most {MAX_PROFILE_REASON_CHARS} characters"),
        ));
    }
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

fn profile_facade_error(
    operation: ProfileOperation,
    message: &'static str,
    remediation: &'static str,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.to_owned(),
        Some(json!({
            "code": error_codes::TOOL_PARAMS_INVALID,
            "operation": operation.as_str(),
            "source_of_truth": TOOL_PROFILE_SOURCE_OF_TRUTH,
            "remediation": remediation,
        })),
    )
}

fn visible_tool_names_for_profile(
    profile: ToolProfileKind,
    full_tool_names: &[String],
) -> Vec<String> {
    let mut names = full_tool_names
        .iter()
        .filter(|name| profile.is_visible(name))
        .cloned()
        .collect::<Vec<_>>();
    names.sort_by(|left, right| {
        tool_rank(profile, left)
            .cmp(&tool_rank(profile, right))
            .then(left.cmp(right))
    });
    names
}

fn denied_break_glass_tools(visible_tool_names: &[String]) -> Vec<String> {
    let visible = visible_tool_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    BREAK_GLASS_HAZARDOUS_TOOLS
        .iter()
        .copied()
        .filter(|name| !visible.contains(name))
        .map(str::to_owned)
        .collect()
}

/// #1352: compute THIS session's current standing on the real OS-foreground
/// route — whether it already holds the lease and a foreground-capable profile,
/// and the precise remaining escalation steps. A read-only preflight so an agent
/// (or the operator) can see the gate state before attempting a foreground action.
fn foreground_route_readiness(
    session_id: Option<&str>,
    profile: ToolProfileKind,
) -> ToolProfileForegroundRoute {
    let holds_foreground_lease = match session_id {
        Some(session_id) => {
            synapse_action::lease::status().owner_session_id.as_deref() == Some(session_id)
        }
        // Unscoped stdio admin runs without a session lease concept.
        None => true,
    };
    let profile_allows_foreground = matches!(
        profile,
        ToolProfileKind::BreakGlass | ToolProfileKind::FullCapability
    );
    let mut remaining_steps = Vec::new();
    if !holds_foreground_lease {
        remaining_steps.push(
            "control_lease_acquire (this session must own the foreground input lease)".to_owned(),
        );
    }
    if !profile_allows_foreground {
        remaining_steps.push(
            "profile operation=set profile=break_glass confirm_break_glass=true reason=<why> (requires the lease first)"
                .to_owned(),
        );
    }
    ToolProfileForegroundRoute {
        holds_foreground_lease,
        profile_allows_foreground,
        foreground_route_ready: holds_foreground_lease && profile_allows_foreground,
        remaining_steps,
    }
}

fn foreground_capability_policy(profile: ToolProfileKind) -> ToolProfileForegroundCapability {
    let (preferred_path, real_os_foreground_path) = match profile {
        ToolProfileKind::NormalAgent => (
            "only registered public facade tools are visible in the normal profile; implementation tools require an explicit advanced profile or a facade route",
            "control_lease_acquire + profile operation=set break_glass + raw foreground primitive; denied without lease/reason/confirm",
        ),
        ToolProfileKind::BrowserControl => (
            "debugger-free browser/target_act tools plus lease controls are visible in the task profile; raw CDP/chrome.debugger, shell, and spawn surfaces stay hidden",
            "control_lease_acquire + profile operation=set break_glass + raw foreground primitive; denied without lease/reason/confirm",
        ),
        ToolProfileKind::BrowserDebugger => (
            "browser-only raw CDP/chrome.debugger tools are visible by explicit profile; raw shell/spawn and OS foreground primitives stay hidden",
            "control_lease_acquire + profile operation=set break_glass + raw foreground primitive; denied without lease/reason/confirm",
        ),
        ToolProfileKind::BreakGlass | ToolProfileKind::FullCapability => (
            "full raw surface is visible; prefer target_act/session-lane tools unless real OS foreground input is the intended lane",
            "raw foreground primitives may run only under their own lease/target guards and action-audit policy",
        ),
    };
    ToolProfileForegroundCapability {
        source_of_truth: TOOL_PROFILE_SOURCE_OF_TRUTH,
        profile_preserves_capability: true,
        human_os_foreground: "the physical foreground window used by the human; never an implicit fallback for hidden tools",
        agent_logical_foreground: "the per-session foreground-equivalent target/lane; preferred for valid local work",
        preferred_path,
        real_os_foreground_path,
    }
}

fn hidden_tool_capability_routes(visible_tool_names: &[String]) -> Vec<HiddenToolCapabilityRoute> {
    let visible = visible_tool_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    BREAK_GLASS_HAZARDOUS_TOOLS
        .iter()
        .chain(BROWSER_DEBUGGER_ONLY_EXACT.iter())
        .copied()
        .filter(|name| !visible.contains(name))
        .map(hidden_tool_capability_route)
        .collect()
}

fn hidden_tool_capability_route(tool_name: &str) -> HiddenToolCapabilityRoute {
    let preferred_tools = match tool_name {
        "act_click" => vec![
            "act operation=invoke click",
            "browser_dom operation=query",
            "target operation=set",
        ],
        "act_type" | "act_set_value" | "act_set_field_text" => {
            vec![
                "act operation=invoke set_field",
                "browser_form operation=set_value",
                "browser_dom operation=query",
            ]
        }
        "act_press" | "act_keymap" | "act_combo" => {
            vec!["act operation=invoke press", "browser_dom operation=query"]
        }
        "act_scroll" => vec![
            "act operation=invoke scroll",
            "browser_dom operation=query",
            "browser_capture operation=screenshot",
            "observe operation=current",
            "target operation=set",
        ],
        "act_stroke" | "act_pad" => vec![
            "target operation=set",
            "control_lease_acquire",
            "profile operation=set profile=break_glass",
        ],
        "act_focus_window" => vec![
            "target operation=set",
            "control_lease_acquire",
            "profile operation=set profile=break_glass",
            "act operation=invoke focus_window",
            "session operation=list",
        ],
        "act_launch" => vec![
            "agent operation=spawn",
            "browser_tabs operation=new",
            "browser_nav operation=navigate",
        ],
        "act_clipboard" => vec![
            "workspace operation=put",
            "browser_form operation=set_value",
            "act operation=invoke set_field",
        ],
        "release_all" => vec![
            "target operation=set",
            "control_lease_release",
            "session operation=list",
        ],
        "hidden_desktop_pip_frame" => {
            vec![
                "screenshot operation=capture",
                "observe operation=current",
                "session operation=list",
            ]
        }
        "action_diagnostic_queue_full_setup" | "action_diagnostic_rate_limit_override" => {
            vec![
                "health",
                "storage operation=inspect",
                "session operation=list",
            ]
        }
        "browser_console_messages"
        | "browser_network"
        | "browser_network_har"
        | "browser_network_overrides"
        | "browser_route" => vec![
            "profile operation=set profile=browser_debugger confirm_break_glass=true reason=<why raw CDP is required>",
            "browser_dom operation=query",
            "browser_wait operation=for_condition",
            "browser_storage operation=read",
        ],
        tool if BROWSER_DEBUGGER_ONLY_EXACT.contains(&tool) => vec![
            "profile operation=set profile=browser_debugger confirm_break_glass=true reason=<why chrome.debugger is required>",
            "browser_tabs operation=list",
            "browser_dom operation=query",
            "act operation=invoke",
        ],
        _ => vec![
            "act operation=invoke",
            "profile operation=set profile=break_glass",
        ],
    };
    HiddenToolCapabilityRoute {
        hidden_tool: tool_name.to_owned(),
        status: "routed_or_break_glass",
        preferred_tools: preferred_tools.into_iter().map(str::to_owned).collect(),
        agent_logical_foreground_policy: "use the preferred tools against this session's agent_logical_foreground/foreground_lane",
        human_os_foreground_policy: "never use the human OS foreground as an implicit fallback",
        break_glass_policy: "for browser CDP/chrome.debugger instrumentation, call profile operation=set profile=browser_debugger with confirm_break_glass=true and a non-empty reason; for a real OS foreground primitive, first acquire the input lease, then call profile operation=set profile=break_glass with confirm_break_glass=true and a non-empty reason",
    }
}

#[derive(Debug, Eq, PartialEq)]
struct PublicToolRegistryValidation {
    duplicate_public_tool_names: Vec<String>,
    forbidden_public_tool_names: Vec<String>,
    over_limit_by: usize,
}

impl PublicToolRegistryValidation {
    const fn is_valid(&self) -> bool {
        self.duplicate_public_tool_names.is_empty()
            && self.forbidden_public_tool_names.is_empty()
            && self.over_limit_by == 0
    }
}

#[derive(Debug, Eq, PartialEq)]
struct FacadeContractValidation {
    missing_contract_tool_names: Vec<String>,
    unknown_contract_tool_names: Vec<String>,
    duplicate_contract_tool_names: Vec<String>,
    duplicate_operation_names: Vec<String>,
    invalid_contract_reasons: Vec<String>,
}

impl FacadeContractValidation {
    const fn is_valid(&self) -> bool {
        self.missing_contract_tool_names.is_empty()
            && self.unknown_contract_tool_names.is_empty()
            && self.duplicate_contract_tool_names.is_empty()
            && self.duplicate_operation_names.is_empty()
            && self.invalid_contract_reasons.is_empty()
    }
}

fn public_tool_names() -> Vec<String> {
    PUBLIC_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

fn public_tool_registry_snapshot_for(
    full_tool_names: &[String],
) -> Result<PublicToolRegistrySnapshot, ErrorData> {
    let public_tool_names = public_tool_names();
    let validation = validate_public_tool_registry_names(&public_tool_names)?;
    let full_tool_names = full_tool_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let registered_tools_present = public_tool_names
        .iter()
        .filter(|name| full_tool_names.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let registered_tools_missing = public_tool_names
        .iter()
        .filter(|name| !full_tool_names.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    Ok(PublicToolRegistrySnapshot {
        source_of_truth: PUBLIC_TOOL_REGISTRY_SOURCE_OF_TRUTH,
        max_public_tool_count: PUBLIC_TOOL_LIMIT,
        public_tool_count: public_tool_names.len(),
        public_tool_sha256: sha256_json_hex(&public_tool_names)?,
        public_tool_names,
        implementation_tool_count: full_tool_names.len(),
        registered_tools_present,
        registered_tools_missing,
        duplicate_public_tool_names: validation.duplicate_public_tool_names,
        forbidden_public_tool_names: validation.forbidden_public_tool_names,
        over_limit_by: validation.over_limit_by,
    })
}

fn validate_public_tool_registry_names(
    names: &[String],
) -> Result<PublicToolRegistryValidation, ErrorData> {
    let validation = inspect_public_tool_registry_names(names);
    if validation.is_valid() {
        return Ok(validation);
    }
    Err(public_tool_registry_error(names, &validation))
}

fn inspect_public_tool_registry_names(names: &[String]) -> PublicToolRegistryValidation {
    let mut seen = BTreeSet::new();
    let mut duplicate_public_tool_names = BTreeSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            duplicate_public_tool_names.insert(name.clone());
        }
    }
    let forbidden_public_tool_names = names
        .iter()
        .filter(|name| PUBLIC_TOOL_IMPLEMENTATION_DENYLIST.contains(&name.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    PublicToolRegistryValidation {
        duplicate_public_tool_names: duplicate_public_tool_names.into_iter().collect(),
        forbidden_public_tool_names,
        over_limit_by: names.len().saturating_sub(PUBLIC_TOOL_LIMIT),
    }
}

fn public_tool_registry_error(
    names: &[String],
    validation: &PublicToolRegistryValidation,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "public MCP tool registry is invalid: count={} max={} duplicates={:?} forbidden={:?}",
            names.len(),
            PUBLIC_TOOL_LIMIT,
            validation.duplicate_public_tool_names,
            validation.forbidden_public_tool_names
        ),
        Some(json!({
            "code": error_codes::TOOL_INTERNAL_ERROR,
            "detail_code": "PUBLIC_TOOL_REGISTRY_INVALID",
            "operation": PUBLIC_TOOL_REGISTRY_OPERATION,
            "source_of_truth": PUBLIC_TOOL_REGISTRY_SOURCE_OF_TRUTH,
            "max_public_tool_count": PUBLIC_TOOL_LIMIT,
            "public_tool_count": names.len(),
            "over_limit_by": validation.over_limit_by,
            "duplicate_public_tool_names": validation.duplicate_public_tool_names,
            "forbidden_public_tool_names": validation.forbidden_public_tool_names,
            "remediation": "edit PUBLIC_TOOL_NAMES so it has at most 40 unique facade names and no implementation-only tools",
        })),
    )
}

fn facade_contract_snapshot_for(
    public_tool_names: &[String],
) -> Result<FacadeContractSnapshot, ErrorData> {
    let validation = validate_facade_contracts(public_tool_names, FACADE_TOOL_CONTRACTS)?;
    let contracts = facade_contract_snapshots(FACADE_TOOL_CONTRACTS);
    let contract_tool_names = contracts
        .iter()
        .map(|contract| contract.tool_name.to_owned())
        .collect::<Vec<_>>();
    let operation_count = contracts
        .iter()
        .map(|contract| contract.operations.len())
        .sum::<usize>();
    let mutating_operation_count = contracts
        .iter()
        .flat_map(|contract| &contract.operations)
        .filter(|operation| operation.mutates_state)
        .count();
    let facade_contract_sha256 = sha256_json_hex(&contracts)?;
    Ok(FacadeContractSnapshot {
        source_of_truth: FACADE_CONTRACT_SOURCE_OF_TRUTH,
        structured_error_contract: FACADE_CONTRACT_STRUCTURED_ERROR,
        public_tool_count: public_tool_names.len(),
        contract_tool_count: contracts.len(),
        operation_count,
        mutating_operation_count,
        facade_contract_sha256,
        contract_tool_names,
        missing_contract_tool_names: validation.missing_contract_tool_names,
        unknown_contract_tool_names: validation.unknown_contract_tool_names,
        duplicate_contract_tool_names: validation.duplicate_contract_tool_names,
        duplicate_operation_names: validation.duplicate_operation_names,
        invalid_contract_reasons: validation.invalid_contract_reasons,
        contracts,
    })
}

fn facade_contract_snapshots(
    contracts: &[FacadeToolContractSpec],
) -> Vec<FacadeToolContractSnapshot> {
    contracts
        .iter()
        .map(|contract| FacadeToolContractSnapshot {
            tool_name: contract.tool_name,
            operation_enum: contract.operation_enum,
            source_of_truth: contract.source_of_truth,
            operations: contract
                .operations
                .iter()
                .map(|operation| FacadeOperationContractSnapshot {
                    operation: operation.operation,
                    mutates_state: operation.mutates_state,
                    target_required: operation.target_required,
                    source_of_truth: operation.source_of_truth,
                    readback_source_of_truth: operation.readback_source_of_truth,
                    error_code: operation.error_code,
                    remediation: operation.remediation,
                })
                .collect(),
        })
        .collect()
}

fn validate_facade_contracts(
    public_tool_names: &[String],
    contracts: &[FacadeToolContractSpec],
) -> Result<FacadeContractValidation, ErrorData> {
    let validation = inspect_facade_contracts(public_tool_names, contracts);
    if validation.is_valid() {
        return Ok(validation);
    }
    Err(facade_contract_error(
        public_tool_names,
        contracts,
        &validation,
    ))
}

fn inspect_facade_contracts(
    public_tool_names: &[String],
    contracts: &[FacadeToolContractSpec],
) -> FacadeContractValidation {
    let public_name_set = public_tool_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let contract_names = contracts
        .iter()
        .map(|contract| contract.tool_name)
        .collect::<Vec<_>>();
    let contract_name_set = contract_names.iter().copied().collect::<BTreeSet<_>>();
    let missing_contract_tool_names = public_tool_names
        .iter()
        .filter(|name| !contract_name_set.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let unknown_contract_tool_names = contract_name_set
        .iter()
        .filter(|name| !public_name_set.contains(**name))
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    let mut seen_contract_names = BTreeSet::new();
    let mut duplicate_contract_tool_names = BTreeSet::new();
    let mut duplicate_operation_names = BTreeSet::new();
    let mut invalid_contract_reasons = Vec::new();
    for contract in contracts {
        if contract.tool_name.trim().is_empty() {
            invalid_contract_reasons.push("contract tool_name must not be empty".to_owned());
        }
        if !seen_contract_names.insert(contract.tool_name) {
            duplicate_contract_tool_names.insert(contract.tool_name.to_owned());
        }
        if contract.operation_enum.trim().is_empty() {
            invalid_contract_reasons.push(format!(
                "{} operation_enum must not be empty",
                contract.tool_name
            ));
        }
        if contract.source_of_truth.trim().is_empty() {
            invalid_contract_reasons.push(format!(
                "{} source_of_truth must not be empty",
                contract.tool_name
            ));
        }
        if contract.operations.is_empty() {
            invalid_contract_reasons.push(format!(
                "{} must declare at least one operation",
                contract.tool_name
            ));
        }

        let mut seen_operations = BTreeSet::new();
        for operation in contract.operations {
            if operation.operation.trim().is_empty() {
                invalid_contract_reasons.push(format!(
                    "{} operation name must not be empty",
                    contract.tool_name
                ));
            }
            if !seen_operations.insert(operation.operation) {
                duplicate_operation_names
                    .insert(format!("{}.{}", contract.tool_name, operation.operation));
            }
            if operation.source_of_truth.trim().is_empty() {
                invalid_contract_reasons.push(format!(
                    "{}.{} source_of_truth must not be empty",
                    contract.tool_name, operation.operation
                ));
            }
            if operation.error_code.trim().is_empty() {
                invalid_contract_reasons.push(format!(
                    "{}.{} error_code must not be empty",
                    contract.tool_name, operation.operation
                ));
            }
            if operation.remediation.trim().is_empty() {
                invalid_contract_reasons.push(format!(
                    "{}.{} remediation must not be empty",
                    contract.tool_name, operation.operation
                ));
            }
            if operation.mutates_state
                && operation
                    .readback_source_of_truth
                    .is_none_or(|readback| readback.trim().is_empty())
            {
                invalid_contract_reasons.push(format!(
                    "{}.{} mutates_state requires readback_source_of_truth",
                    contract.tool_name, operation.operation
                ));
            }
        }
    }

    FacadeContractValidation {
        missing_contract_tool_names,
        unknown_contract_tool_names,
        duplicate_contract_tool_names: duplicate_contract_tool_names.into_iter().collect(),
        duplicate_operation_names: duplicate_operation_names.into_iter().collect(),
        invalid_contract_reasons,
    }
}

fn facade_contract_error(
    public_tool_names: &[String],
    contracts: &[FacadeToolContractSpec],
    validation: &FacadeContractValidation,
) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        format!(
            "public facade contract is invalid: public_count={} contract_count={} missing={:?} unknown={:?} duplicate_tools={:?} duplicate_operations={:?} invalid_reasons={:?}",
            public_tool_names.len(),
            contracts.len(),
            validation.missing_contract_tool_names,
            validation.unknown_contract_tool_names,
            validation.duplicate_contract_tool_names,
            validation.duplicate_operation_names,
            validation.invalid_contract_reasons
        ),
        Some(json!({
            "code": error_codes::TOOL_INTERNAL_ERROR,
            "detail_code": FACADE_CONTRACT_ERROR_CODE,
            "operation": FACADE_CONTRACT_OPERATION,
            "source_of_truth": FACADE_CONTRACT_SOURCE_OF_TRUTH,
            "structured_error_contract": FACADE_CONTRACT_STRUCTURED_ERROR,
            "public_tool_count": public_tool_names.len(),
            "contract_tool_count": contracts.len(),
            "missing_contract_tool_names": validation.missing_contract_tool_names,
            "unknown_contract_tool_names": validation.unknown_contract_tool_names,
            "duplicate_contract_tool_names": validation.duplicate_contract_tool_names,
            "duplicate_operation_names": validation.duplicate_operation_names,
            "invalid_contract_reasons": validation.invalid_contract_reasons,
            "remediation": "edit FACADE_TOOL_CONTRACTS so every public facade has unique typed operations, source_of_truth, structured error code, remediation, and readback_source_of_truth for mutations",
        })),
    )
}

fn sort_tools_for_profile(tools: &mut [Tool], profile: ToolProfileKind) {
    tools.sort_by(|left, right| {
        let left_name = left.name.as_ref();
        let right_name = right.name.as_ref();
        tool_rank(profile, left_name)
            .cmp(&tool_rank(profile, right_name))
            .then(left_name.cmp(right_name))
    });
}

fn tool_rank(profile: ToolProfileKind, tool_name: &str) -> usize {
    match profile {
        ToolProfileKind::NormalAgent => NORMAL_ALLOWED_EXACT
            .iter()
            .position(|name| *name == tool_name)
            .unwrap_or(usize::MAX),
        ToolProfileKind::BrowserControl => BROWSER_CONTROL_ALLOWED_EXACT
            .iter()
            .position(|name| *name == tool_name)
            .unwrap_or(usize::MAX),
        ToolProfileKind::BrowserDebugger => BROWSER_DEBUGGER_ALLOWED_EXACT
            .iter()
            .position(|name| *name == tool_name)
            .unwrap_or(usize::MAX),
        ToolProfileKind::BreakGlass | ToolProfileKind::FullCapability => usize::MAX,
    }
}

fn tool_profile_key(session_id: &str) -> Vec<u8> {
    format!("{TOOL_PROFILE_PREFIX}{session_id}").into_bytes()
}

fn audit_readback(
    readback: super::command_audit::CommandAuditRowReadback,
) -> ToolProfileAuditReadback {
    ToolProfileAuditReadback {
        cf_name: readback.cf_name,
        key_hex: readback.key_hex,
        value_len_bytes: readback.value_len_bytes,
        value_sha256: readback.value_sha256,
    }
}

fn sha256_json_hex<T: Serialize>(value: &T) -> Result<String, ErrorData> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("serialize tool profile digest payload failed: {error}"),
        )
    })?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{num::NonZeroUsize, path::Path};

    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    use crate::{m2::M2ServiceConfig, m3::M3ServiceConfig, m4::M4ServiceConfig};

    fn service_with_db(path: &Path) -> SynapseService {
        SynapseService::try_with_m2_shutdown_reason_and_m3_config(
            CancellationToken::new(),
            "test",
            CancellationToken::new(),
            &M2ServiceConfig::default(),
            M3ServiceConfig::from_cli_parts(
                Some(path.join("db")),
                Some(path.to_path_buf()),
                false,
                "127.0.0.1:0".to_owned(),
                NonZeroUsize::new(4).expect("nonzero"),
                false,
                true,
                None,
                false,
                None,
            ),
            M4ServiceConfig::default(),
        )
        .expect("construct service")
    }

    fn tool_names(tools: Vec<Tool>) -> Vec<String> {
        tools
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    fn names() -> Vec<String> {
        let mut names = std::collections::BTreeSet::new();
        names.extend(
            BREAK_GLASS_HAZARDOUS_TOOLS
                .iter()
                .map(|name| (*name).to_owned()),
        );
        names.extend(
            BROWSER_DEBUGGER_ONLY_EXACT
                .iter()
                .map(|name| (*name).to_owned()),
        );
        names.extend(
            [
                "act_run_shell",
                "act_spawn_agent",
                "act_foreground",
                "act_launch",
                "act",
                "cdp_open_tab",
                "profile",
                "health",
                "session",
                "session_list",
                "screenshot",
                "subscribe",
                "observe",
                "find",
                "read_text",
                "target",
                "target_act",
                "browser_adopt_active_tab",
                "browser_aria_snapshot",
                "browser_assert",
                "browser_batch",
                "browser_clock",
                "browser_content",
                "browser_downloads",
                "browser_file_upload",
                "browser_fill_form",
                "browser_frames",
                "browser_inspect",
                "browser_locate",
                "browser_page_events",
                "browser_scroll_into_view",
                "browser_screenshot",
                "browser_set_content",
                "browser_set_value",
                "browser_storage",
                "browser_tabs",
                "browser_wait_for",
                "control_lease_acquire",
                "control_lease_release",
                "tool_profile_set",
                "tool_profile_status",
            ]
            .iter()
            .map(|name| (*name).to_owned()),
        );
        names.into_iter().collect()
    }

    fn assert_debugger_only_hidden(visible: &[String]) {
        for hidden in BROWSER_DEBUGGER_ONLY_EXACT {
            assert!(
                !visible.iter().any(|name| name == hidden),
                "default profile must hide browser debugger tool {hidden}"
            );
        }
    }

    fn assert_debugger_only_visible(visible: &[String]) {
        for required in BROWSER_DEBUGGER_ONLY_EXACT {
            assert!(
                visible.iter().any(|name| name == required),
                "browser_debugger profile must expose browser debugger tool {required}"
            );
        }
    }

    fn registry_error_data(error: &ErrorData) -> &Value {
        error.data.as_ref().expect("registry error data")
    }

    #[test]
    fn public_tool_registry_contract_is_capped_unique_and_facade_only() {
        let public_names = public_tool_names();
        assert_eq!(public_names.len(), PUBLIC_TOOL_LIMIT);
        let validation =
            validate_public_tool_registry_names(&public_names).expect("valid registry");
        assert!(validation.is_valid());
        assert!(public_names.contains(&"health".to_owned()));
        assert!(public_names.contains(&"telemetry".to_owned()));
        assert!(!public_names.contains(&"cdp_open_tab".to_owned()));
        assert!(!public_names.contains(&"storage_put_probe_rows".to_owned()));

        let snapshot =
            public_tool_registry_snapshot_for(&names()).expect("registry snapshot from fixture");
        assert_eq!(snapshot.max_public_tool_count, PUBLIC_TOOL_LIMIT);
        assert_eq!(snapshot.public_tool_count, PUBLIC_TOOL_LIMIT);
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"health".to_owned())
        );
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"profile".to_owned()),
            "#1377 registers the profile facade"
        );
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"session".to_owned()),
            "#1377 registers the session facade"
        );
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"screenshot".to_owned()),
            "#1378 registers the screenshot facade"
        );
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"target".to_owned()),
            "#1379 registers the target facade"
        );
        assert!(
            snapshot
                .registered_tools_present
                .contains(&"act".to_owned()),
            "#1379 registers the act facade"
        );
        assert!(snapshot.duplicate_public_tool_names.is_empty());
        assert!(snapshot.forbidden_public_tool_names.is_empty());
        assert_eq!(snapshot.over_limit_by, 0);
    }

    #[test]
    fn public_tool_registry_rejects_duplicate_public_names() {
        let names = vec!["health".to_owned(), "health".to_owned()];
        let error = validate_public_tool_registry_names(&names).expect_err("duplicate rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some("PUBLIC_TOOL_REGISTRY_INVALID")
        );
        assert_eq!(
            data.get("duplicate_public_tool_names"),
            Some(&json!(["health"]))
        );
    }

    #[test]
    fn public_tool_registry_rejects_forty_first_public_tool() {
        let names = (0..=PUBLIC_TOOL_LIMIT)
            .map(|index| format!("facade_{index:02}"))
            .collect::<Vec<_>>();
        let error = validate_public_tool_registry_names(&names).expect_err("41st tool rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some("PUBLIC_TOOL_REGISTRY_INVALID")
        );
        assert_eq!(data.get("over_limit_by").and_then(Value::as_u64), Some(1));
        assert_eq!(
            data.get("public_tool_count").and_then(Value::as_u64),
            Some(41)
        );
    }

    #[test]
    fn public_tool_registry_rejects_implementation_tool_publication() {
        let names = vec!["health".to_owned(), "cdp_open_tab".to_owned()];
        let error =
            validate_public_tool_registry_names(&names).expect_err("implementation tool rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some("PUBLIC_TOOL_REGISTRY_INVALID")
        );
        assert_eq!(
            data.get("forbidden_public_tool_names"),
            Some(&json!(["cdp_open_tab"]))
        );
    }

    #[test]
    fn facade_contract_covers_every_public_tool_with_readback_rules() {
        let public_names = public_tool_names();
        let snapshot = facade_contract_snapshot_for(&public_names).expect("facade contract");
        assert_eq!(snapshot.public_tool_count, PUBLIC_TOOL_LIMIT);
        assert_eq!(snapshot.contract_tool_count, PUBLIC_TOOL_LIMIT);
        assert_eq!(snapshot.missing_contract_tool_names, Vec::<String>::new());
        assert_eq!(snapshot.unknown_contract_tool_names, Vec::<String>::new());
        assert_eq!(snapshot.duplicate_contract_tool_names, Vec::<String>::new());
        assert_eq!(snapshot.duplicate_operation_names, Vec::<String>::new());
        assert_eq!(snapshot.invalid_contract_reasons, Vec::<String>::new());
        assert!(snapshot.operation_count > PUBLIC_TOOL_LIMIT);
        assert!(snapshot.mutating_operation_count > 0);
        assert!(snapshot.contracts.iter().any(|contract| {
            contract.tool_name == "browser_tabs"
                && contract.operation_enum == "BrowserTabsOperation"
                && contract.operations.iter().any(|operation| {
                    operation.operation == "select"
                        && operation.mutates_state
                        && operation.readback_source_of_truth.is_some()
                })
        }));
        assert!(snapshot.contracts.iter().all(|contract| {
            contract.operations.iter().all(|operation| {
                !operation.mutates_state || operation.readback_source_of_truth.is_some()
            })
        }));
    }

    #[test]
    fn facade_contract_rejects_unknown_contract_tool() {
        const OPS: &[FacadeOperationContractSpec] = &[op(
            "status",
            false,
            false,
            "test SoT",
            None,
            error_codes::TOOL_INTERNAL_ERROR,
            "fix test",
        )];
        const CONTRACTS: &[FacadeToolContractSpec] = &[
            facade_contract("health", "HealthOperation", "test SoT", OPS),
            facade_contract("not_public", "NotPublicOperation", "test SoT", OPS),
        ];
        let error = validate_facade_contracts(&["health".to_owned()], CONTRACTS)
            .expect_err("unknown public contract rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some(FACADE_CONTRACT_ERROR_CODE)
        );
        assert_eq!(
            data.get("unknown_contract_tool_names"),
            Some(&json!(["not_public"]))
        );
    }

    #[test]
    fn facade_contract_rejects_duplicate_operations() {
        const OPS: &[FacadeOperationContractSpec] = &[
            op(
                "status",
                false,
                false,
                "test SoT",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "fix test",
            ),
            op(
                "status",
                false,
                false,
                "test SoT",
                None,
                error_codes::TOOL_INTERNAL_ERROR,
                "fix test",
            ),
        ];
        const CONTRACTS: &[FacadeToolContractSpec] = &[facade_contract(
            "health",
            "HealthOperation",
            "test SoT",
            OPS,
        )];
        let error = validate_facade_contracts(&["health".to_owned()], CONTRACTS)
            .expect_err("duplicate operation rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some(FACADE_CONTRACT_ERROR_CODE)
        );
        assert_eq!(
            data.get("duplicate_operation_names"),
            Some(&json!(["health.status"]))
        );
    }

    #[test]
    fn facade_contract_rejects_mutation_without_readback_source() {
        const OPS: &[FacadeOperationContractSpec] = &[op(
            "set",
            true,
            false,
            "test SoT",
            None,
            error_codes::STORAGE_WRITE_FAILED,
            "fix test",
        )];
        const CONTRACTS: &[FacadeToolContractSpec] = &[facade_contract(
            "health",
            "HealthOperation",
            "test SoT",
            OPS,
        )];
        let error = validate_facade_contracts(&["health".to_owned()], CONTRACTS)
            .expect_err("mutating operation without readback rejected");
        let data = registry_error_data(&error);
        assert_eq!(
            data.get("detail_code").and_then(Value::as_str),
            Some(FACADE_CONTRACT_ERROR_CODE)
        );
        let reasons = data
            .get("invalid_contract_reasons")
            .and_then(Value::as_array)
            .expect("invalid reasons");
        assert!(reasons.iter().any(|reason| {
            reason
                .as_str()
                .is_some_and(|reason| reason.contains("mutates_state requires readback"))
        }));
    }

    #[test]
    fn normal_profile_routes_foreground_capability_without_raw_primitives() {
        let visible = visible_tool_names_for_profile(ToolProfileKind::NormalAgent, &names());
        assert_eq!(
            visible,
            [
                "health",
                "profile",
                "session",
                "subscribe",
                "observe",
                "find",
                "read_text",
                "screenshot",
                "target",
                "act",
                "browser_tabs",
                "browser_storage",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        );
        assert!(
            visible
                .iter()
                .all(|name| PUBLIC_TOOL_NAMES.contains(&name.as_str()))
        );
        assert!(!visible.contains(&"act_run_shell".to_owned()));
        assert!(!visible.contains(&"act_launch".to_owned()));
        assert!(!visible.contains(&"cdp_open_tab".to_owned()));
        assert!(!visible.contains(&"target_act".to_owned()));
        assert!(!visible.contains(&"browser_content".to_owned()));
        assert!(!visible.contains(&"browser_locate".to_owned()));
        assert!(!visible.contains(&"browser_set_value".to_owned()));
        assert!(!visible.contains(&"tool_profile_set".to_owned()));
        assert!(!visible.contains(&"tool_profile_status".to_owned()));
        assert!(!visible.contains(&"act_click".to_owned()));
        assert!(!visible.contains(&"act_type".to_owned()));
        assert!(!visible.contains(&"release_all".to_owned()));
        assert_debugger_only_hidden(&visible);

        let policy = foreground_capability_policy(ToolProfileKind::NormalAgent);
        assert!(policy.profile_preserves_capability);
        assert!(
            policy
                .agent_logical_foreground
                .contains("per-session foreground-equivalent")
        );
        assert!(
            policy
                .human_os_foreground
                .contains("never an implicit fallback")
        );

        let routes = hidden_tool_capability_routes(&visible);
        assert!(
            !routes.iter().any(|route| route.hidden_tool == "act_launch"),
            "act_launch is a launch/target-creation capability with its own policy checks, not a hidden foreground input primitive"
        );
        let act_type_route = routes
            .iter()
            .find(|route| route.hidden_tool == "act_type")
            .expect("act_type route");
        assert!(
            act_type_route
                .preferred_tools
                .contains(&"act operation=invoke set_field".to_owned())
        );
        assert!(
            act_type_route
                .preferred_tools
                .contains(&"browser_form operation=set_value".to_owned())
        );
        assert!(
            act_type_route
                .preferred_tools
                .contains(&"browser_dom operation=query".to_owned())
        );
        let browser_debugger_route = routes
            .iter()
            .find(|route| route.hidden_tool == "browser_console_messages")
            .expect("browser_console_messages route");
        assert!(
            browser_debugger_route
                .preferred_tools
                .iter()
                .any(|tool| tool.contains("profile=browser_debugger"))
        );
        assert!(
            act_type_route
                .agent_logical_foreground_policy
                .contains("agent_logical_foreground")
        );
        assert!(
            act_type_route
                .human_os_foreground_policy
                .contains("never use the human OS foreground")
        );
    }

    #[test]
    fn browser_profile_is_narrower_than_normal_agent() {
        let visible = visible_tool_names_for_profile(ToolProfileKind::BrowserControl, &names());
        assert!(visible.contains(&"cdp_open_tab".to_owned()));
        assert!(visible.contains(&"session_list".to_owned()));
        assert!(visible.contains(&"target_act".to_owned()));
        assert!(visible.contains(&"browser_content".to_owned()));
        assert!(visible.contains(&"browser_locate".to_owned()));
        assert!(visible.contains(&"browser_scroll_into_view".to_owned()));
        assert!(visible.contains(&"browser_set_content".to_owned()));
        assert!(visible.contains(&"browser_set_value".to_owned()));
        assert!(visible.contains(&"browser_wait_for".to_owned()));
        assert!(visible.contains(&"control_lease_acquire".to_owned()));
        assert!(visible.contains(&"control_lease_release".to_owned()));
        assert!(!visible.contains(&"act_run_shell".to_owned()));
        assert!(!visible.contains(&"act_click".to_owned()));
        assert_debugger_only_hidden(&visible);
    }

    #[test]
    fn browser_debugger_profile_exposes_browser_debugger_surface_without_shell_or_foreground() {
        let visible = visible_tool_names_for_profile(ToolProfileKind::BrowserDebugger, &names());
        assert_debugger_only_visible(&visible);
        assert!(visible.contains(&"browser_content".to_owned()));
        assert!(visible.contains(&"browser_locate".to_owned()));
        assert!(visible.contains(&"target_act".to_owned()));
        assert!(visible.contains(&"cdp_open_tab".to_owned()));
        assert!(visible.contains(&"tool_profile_set".to_owned()));
        assert!(!visible.contains(&"act_run_shell".to_owned()));
        assert!(!visible.contains(&"act_spawn_agent".to_owned()));
        assert!(!visible.contains(&"act_click".to_owned()));
        assert!(!visible.contains(&"act_type".to_owned()));
        assert!(!visible.contains(&"release_all".to_owned()));

        let policy = foreground_capability_policy(ToolProfileKind::BrowserDebugger);
        assert!(policy.profile_preserves_capability);
        assert!(policy.preferred_path.contains("raw CDP/chrome.debugger"));
        assert!(policy.preferred_path.contains("raw shell/spawn"));
    }

    #[test]
    fn break_glass_profile_exposes_full_surface() {
        let mut expected = names();
        expected.sort();
        let visible = visible_tool_names_for_profile(ToolProfileKind::BreakGlass, &names());
        assert_eq!(visible, expected);
        assert!(denied_break_glass_tools(&visible).is_empty());
    }

    #[test]
    fn break_glass_requires_confirm_reason_and_lease() {
        let proof = ToolProfileLeaseProof {
            required: true,
            held: false,
            owner_session_id: None,
            caller_is_owner: false,
            expires_in_ms: None,
        };
        assert!(
            validate_profile_set_policy(
                "s1",
                ToolProfileKind::BreakGlass,
                Some("need raw foreground click"),
                false,
                &proof,
            )
            .is_err()
        );
        assert!(
            validate_profile_set_policy("s1", ToolProfileKind::BreakGlass, None, true, &proof,)
                .is_err()
        );
        assert!(
            validate_profile_set_policy(
                "s1",
                ToolProfileKind::BreakGlass,
                Some("need raw foreground click"),
                true,
                &proof,
            )
            .is_err()
        );
    }

    #[test]
    fn break_glass_policy_accepts_owned_lease_proof() {
        let proof = ToolProfileLeaseProof {
            required: true,
            held: true,
            owner_session_id: Some("s1".to_owned()),
            caller_is_owner: true,
            expires_in_ms: Some(10_000),
        };
        validate_profile_set_policy(
            "s1",
            ToolProfileKind::BreakGlass,
            Some("need raw foreground click"),
            true,
            &proof,
        )
        .expect("owned lease proof should allow break-glass");
    }

    #[test]
    fn browser_debugger_requires_confirm_and_reason_but_not_foreground_lease() {
        let proof = ToolProfileLeaseProof {
            required: false,
            held: false,
            owner_session_id: None,
            caller_is_owner: false,
            expires_in_ms: None,
        };
        assert!(
            validate_profile_set_policy(
                "s1",
                ToolProfileKind::BrowserDebugger,
                Some("inspect raw CDP console messages"),
                false,
                &proof,
            )
            .is_err()
        );
        assert!(
            validate_profile_set_policy(
                "s1",
                ToolProfileKind::BrowserDebugger,
                None,
                true,
                &proof,
            )
            .is_err()
        );
        validate_profile_set_policy(
            "s1",
            ToolProfileKind::BrowserDebugger,
            Some("inspect raw CDP console messages"),
            true,
            &proof,
        )
        .expect("browser_debugger requires explicit reason/confirm but not OS foreground lease");
    }

    #[test]
    fn default_normal_profile_persists_policy_row_and_filters_tools() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1008-default-session";
        let before = service
            .read_tool_profile_assignment(session_id)
            .expect("read before");
        assert!(before.is_none());

        let tools = tool_names(
            service
                .tools_for_session_profile(Some(session_id))
                .expect("profile tools"),
        );
        let registry =
            public_tool_registry_snapshot_for(&service.full_tool_names()).expect("registry");
        assert_eq!(tools, registry.registered_tools_present);
        assert!(tools.contains(&"health".to_owned()));
        assert!(tools.contains(&"profile".to_owned()));
        assert!(tools.contains(&"session".to_owned()));
        assert!(tools.contains(&"subscribe".to_owned()));
        assert!(tools.contains(&"browser_tabs".to_owned()));
        assert!(!tools.contains(&"agent_spawn_task_started".to_owned()));
        assert!(!tools.contains(&"cdp_open_tab".to_owned()));
        assert!(!tools.contains(&"suggestion_tick".to_owned()));
        assert!(!tools.contains(&"tool_profile_status".to_owned()));
        assert!(!tools.contains(&"tool_profile_set".to_owned()));
        assert!(!tools.contains(&"demo_record_start".to_owned()));
        assert!(!tools.contains(&"profile_authoring_generate".to_owned()));
        assert!(!tools.contains(&"storage_put_probe_rows".to_owned()));
        assert!(!tools.contains(&"storage_gc_once".to_owned()));
        assert!(
            tools
                .iter()
                .all(|name| PUBLIC_TOOL_NAMES.contains(&name.as_str()))
        );
        assert!(!tools.contains(&"act_click".to_owned()));
        assert!(!tools.contains(&"act_type".to_owned()));
        assert!(!tools.contains(&"release_all".to_owned()));
        assert_debugger_only_hidden(&tools);

        let row = service
            .read_tool_profile_assignment(session_id)
            .expect("read after")
            .expect("row after tools/list profile resolution");
        assert_eq!(row.cf_name, cf::CF_SESSIONS);
        assert_eq!(row.record.profile, ToolProfileKind::NormalAgent);
        assert_eq!(row.record.source, "default_normal_agent");
        assert_eq!(row.record.allowed_tool_count, tools.len());
        assert!(row.value_sha256.starts_with("sha256:"));

        let db = service.m3_storage().expect("storage");
        let stored = db
            .scan_cf_prefix(cf::CF_SESSIONS, &tool_profile_key(session_id))
            .expect("scan policy rows");
        assert_eq!(stored.len(), 1);
        assert_eq!(hex_lower(&stored[0].0), row.key_hex);
        assert_eq!(sha256_hex(&stored[0].1), row.value_sha256);
    }

    #[test]
    fn existing_profile_assignment_self_heals_stale_allowed_surface() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue844-stale-surface-session";
        let fresh = service
            .write_tool_profile_assignment(
                session_id,
                ToolProfileKind::NormalAgent,
                "operator_selected_normal",
                Some("preserve explicit source while refreshing surface".to_owned()),
                Some("operator-session".to_owned()),
            )
            .expect("write fresh row");

        let mut stale = fresh.record.clone();
        stale.allowed_tool_count = 1;
        stale.allowed_tool_sha256 = "sha256:stale-tool-surface".to_owned();
        stale.denied_break_glass_tools = vec!["demo_record_start".to_owned()];
        let stale_encoded = synapse_storage::encode_json(&stale).expect("encode stale row");
        let db = service.m3_storage().expect("storage");
        db.put_batch_pressure_bypass(
            cf::CF_SESSIONS,
            [(tool_profile_key(session_id), stale_encoded.clone())],
        )
        .expect("write stale row");

        let stale_readback = service
            .read_tool_profile_assignment(session_id)
            .expect("read stale row")
            .expect("stale row exists");
        assert_eq!(
            stale_readback.record.allowed_tool_sha256,
            stale.allowed_tool_sha256
        );
        assert_eq!(stale_readback.value_sha256, sha256_hex(&stale_encoded));

        let snapshot = service
            .tool_profile_snapshot(Some(session_id))
            .expect("snapshot self-heals stale surface");
        let row = snapshot.policy_row.as_ref().expect("policy row readback");
        assert_eq!(row.record.profile, ToolProfileKind::NormalAgent);
        assert_eq!(row.record.source, "operator_selected_normal");
        assert_eq!(
            row.record.reason.as_deref(),
            Some("preserve explicit source while refreshing surface")
        );
        assert_eq!(
            row.record.set_by_session_id.as_deref(),
            Some("operator-session")
        );
        assert_eq!(row.record.allowed_tool_count, snapshot.visible_tool_count);
        assert_eq!(row.record.allowed_tool_sha256, snapshot.visible_tool_sha256);
        assert_eq!(
            row.record.denied_break_glass_tools,
            snapshot.denied_break_glass_tools
        );
        assert_ne!(row.value_sha256, sha256_hex(&stale_encoded));
        assert!(snapshot.visible_tool_names.contains(&"profile".to_owned()));
        assert!(
            !snapshot
                .visible_tool_names
                .contains(&"profile_authoring_generate".to_owned())
        );
        assert!(
            !snapshot
                .visible_tool_names
                .contains(&"demo_record_start".to_owned())
        );

        let persisted = service
            .read_tool_profile_assignment(session_id)
            .expect("read healed row")
            .expect("healed row exists");
        assert_eq!(
            persisted.record.allowed_tool_sha256,
            snapshot.visible_tool_sha256
        );
        assert_eq!(persisted.value_sha256, row.value_sha256);
    }

    #[test]
    fn browser_control_profile_excludes_shell_and_foreground_primitives() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1008-browser-session";
        let row = service
            .write_tool_profile_assignment(
                session_id,
                ToolProfileKind::BrowserControl,
                "test_browser_control",
                Some("dashboard inactive tab verification".to_owned()),
                Some(session_id.to_owned()),
            )
            .expect("write browser profile");
        assert_eq!(row.record.profile, ToolProfileKind::BrowserControl);

        let tools = tool_names(
            service
                .tools_for_session_profile(Some(session_id))
                .expect("browser profile tools"),
        );
        assert!(tools.contains(&"cdp_open_tab".to_owned()));
        assert!(tools.contains(&"cdp_target_info".to_owned()));
        assert!(tools.contains(&"target_act".to_owned()));
        assert!(tools.contains(&"control_lease_acquire".to_owned()));
        assert!(tools.contains(&"control_lease_release".to_owned()));
        assert!(tools.contains(&"tool_profile_set".to_owned()));
        assert!(!tools.contains(&"act_run_shell".to_owned()));
        assert!(!tools.contains(&"act_spawn_agent".to_owned()));
        assert!(!tools.contains(&"act_type".to_owned()));
        assert_debugger_only_hidden(&tools);
    }

    #[test]
    fn hidden_tool_call_denial_writes_policy_audit_row() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1008-denied-session";
        let error = service
            .admit_tool_call_for_profile("act_type", Some(session_id))
            .expect_err("normal profile must deny hidden foreground typing tool");
        let code = error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str);
        assert_eq!(code, Some(error_codes::TOOL_PROFILE_POLICY_DENIED));
        let route = error
            .data
            .as_ref()
            .and_then(|data| data.get("capability_route"))
            .expect("capability route in denial");
        assert_eq!(route["hidden_tool"], "act_type");
        let preferred_tools = route["preferred_tools"]
            .as_array()
            .expect("preferred tools array");
        assert!(
            preferred_tools
                .iter()
                .any(|tool| tool.as_str() == Some("act operation=invoke set_field"))
        );
        assert!(
            preferred_tools
                .iter()
                .any(|tool| tool.as_str() == Some("browser_form operation=set_value"))
        );

        let db = service.m3_storage().expect("storage");
        let audit_rows = db
            .scan_cf_prefix(cf::CF_ACTION_LOG, b"")
            .expect("scan command audit");
        let matching = audit_rows
            .iter()
            .filter(|(_, value)| {
                let text = String::from_utf8_lossy(value);
                text.contains("tool_profile_policy")
                    && text.contains("tool_call_denied")
                    && text.contains("act_type")
                    && text.contains(error_codes::TOOL_PROFILE_POLICY_DENIED)
            })
            .count();
        assert_eq!(matching, 1);
    }

    #[test]
    fn hidden_browser_debugger_tool_denial_names_browser_debugger_route() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1318-denied-browser-debugger-session";
        let error = service
            .admit_tool_call_for_profile("browser_console_messages", Some(session_id))
            .expect_err("normal profile must deny raw-CDP browser debugger tool");
        let code = error
            .data
            .as_ref()
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str);
        assert_eq!(code, Some(error_codes::TOOL_PROFILE_POLICY_DENIED));
        let route = error
            .data
            .as_ref()
            .and_then(|data| data.get("capability_route"))
            .expect("capability route in denial");
        assert_eq!(route["hidden_tool"], "browser_console_messages");
        let preferred_tools = route["preferred_tools"]
            .as_array()
            .expect("preferred tools array");
        assert!(preferred_tools.iter().any(|tool| {
            tool.as_str()
                .is_some_and(|text| text.contains("profile=browser_debugger"))
        }));
        let resolution = error
            .data
            .as_ref()
            .and_then(|data| data.get("resolution"))
            .and_then(Value::as_str)
            .expect("resolution");
        assert!(resolution.contains("profile=browser_debugger"));

        let db = service.m3_storage().expect("storage");
        let audit_rows = db
            .scan_cf_prefix(cf::CF_ACTION_LOG, b"")
            .expect("scan command audit");
        let matching = audit_rows
            .iter()
            .filter(|(_, value)| {
                let text = String::from_utf8_lossy(value);
                text.contains("tool_profile_policy")
                    && text.contains("tool_call_denied")
                    && text.contains("browser_console_messages")
                    && text.contains("profile=browser_debugger")
                    && text.contains(error_codes::TOOL_PROFILE_POLICY_DENIED)
            })
            .count();
        assert_eq!(matching, 1);
    }

    /// Seeds the cross-session registry with a real `record_initialized` entry
    /// carrying `client_name`, exactly as the HTTP initialize path does
    /// (`transport.rs` -> `record_registry_initialized`).
    fn seed_session_client(service: &SynapseService, session_id: &str, client_name: &str) {
        use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams};
        use rmcp::transport::streamable_http_server::session::SessionState;
        let state = SessionState::new(InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(client_name, "0.0.0-test"),
        ));
        service
            .session_registry_handle()
            .lock()
            .expect("session registry lock")
            .record_initialized(session_id, &state, "http", 1_000);
    }

    // The four foreground input primitives that #1031 must restore for local
    // models. They are in BREAK_GLASS_HAZARDOUS_TOOLS and hidden from
    // normal_agent — the exact tools gemma/DeepSeek lacked when they "opened
    // Notepad and typed nothing".
    const LOCAL_AGENT_REQUIRED_TOOLS: [&str; 4] = [
        "act_type",
        "act_set_field_text",
        "act_click",
        "act_focus_window",
    ];

    #[test]
    fn local_model_agent_session_gets_full_capability_surface() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1031-local-session";
        seed_session_client(&service, session_id, "synapse-local-model-agent");

        // tools/list surface SoT: the input primitives are present.
        let tools = tool_names(
            service
                .tools_for_session_profile(Some(session_id))
                .expect("local-agent profile tools"),
        );
        for required in LOCAL_AGENT_REQUIRED_TOOLS {
            assert!(
                tools.contains(&required.to_owned()),
                "local-model agent must see {required}; visible tool count = {}",
                tools.len()
            );
        }

        // Durable policy row SoT: full_capability, auto-assigned source.
        let row = service
            .read_tool_profile_assignment(session_id)
            .expect("read profile row")
            .expect("row exists after resolution");
        assert_eq!(row.record.profile, ToolProfileKind::FullCapability);
        assert_eq!(row.record.source, "default_full_capability_local_agent");
        assert!(row.record.denied_break_glass_tools.is_empty());

        // Physical CF_SESSIONS readback proves the row is on disk.
        let db = service.m3_storage().expect("storage");
        let stored = db
            .scan_cf_prefix(cf::CF_SESSIONS, &tool_profile_key(session_id))
            .expect("scan policy rows");
        assert_eq!(stored.len(), 1);
        assert_eq!(sha256_hex(&stored[0].1), row.value_sha256);

        // Call-admission gate SoT: a hidden-for-normal foreground tool is admitted.
        service
            .admit_tool_call_for_profile("act_type", Some(session_id))
            .expect("full_capability must admit act_type for the local-model agent");
    }

    #[test]
    fn non_local_agent_session_stays_least_privilege() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1031-codex-session";
        seed_session_client(&service, session_id, "codex-cli");

        let tools = tool_names(
            service
                .tools_for_session_profile(Some(session_id))
                .expect("codex profile tools"),
        );
        assert!(tools.contains(&"health".to_owned()));
        for hidden in LOCAL_AGENT_REQUIRED_TOOLS {
            assert!(
                !tools.contains(&hidden.to_owned()),
                "non-local agent must NOT see foreground primitive {hidden}"
            );
        }
        let row = service
            .read_tool_profile_assignment(session_id)
            .expect("read profile row")
            .expect("row exists after resolution");
        assert_eq!(row.record.profile, ToolProfileKind::NormalAgent);
        assert_eq!(row.record.source, "default_normal_agent");
        service
            .admit_tool_call_for_profile("act_type", Some(session_id))
            .expect_err("normal_agent must deny act_type");
    }

    #[test]
    fn local_model_session_self_heals_stale_normal_default() {
        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "issue1031-selfheal-session";

        // Simulate a least-privilege row written before the MCP client identity
        // was known to the registry.
        service
            .write_tool_profile_assignment(
                session_id,
                ToolProfileKind::NormalAgent,
                "default_normal_agent",
                None,
                None,
            )
            .expect("seed stale normal-agent row");
        let before = service
            .read_tool_profile_assignment(session_id)
            .expect("read before")
            .expect("row before");
        assert_eq!(before.record.profile, ToolProfileKind::NormalAgent);

        // Identity now classifies the session as the local-model harness.
        seed_session_client(&service, session_id, "synapse-local-model-agent");

        // Resolving the surface upgrades the durable row in place.
        let tools = tool_names(
            service
                .tools_for_session_profile(Some(session_id))
                .expect("post-heal tools"),
        );
        assert!(tools.contains(&"act_type".to_owned()));
        let after = service
            .read_tool_profile_assignment(session_id)
            .expect("read after")
            .expect("row after");
        assert_eq!(after.record.profile, ToolProfileKind::FullCapability);
        assert_eq!(after.record.source, "default_full_capability_local_agent");
    }

    #[test]
    fn explicit_tool_profile_set_to_full_capability_requires_lease_proof() {
        // The frictionless path to full_capability is the auto default; an
        // explicit escalation by hand must carry the same proof as break_glass
        // so no agent can self-escalate to the foreground primitives.
        let proof = ToolProfileLeaseProof {
            required: true,
            held: false,
            owner_session_id: None,
            caller_is_owner: false,
            expires_in_ms: None,
        };
        assert!(
            validate_profile_set_policy(
                "s1",
                ToolProfileKind::FullCapability,
                Some("need full surface"),
                true,
                &proof,
            )
            .is_err()
        );
    }

    /// Measurement probe (not a regression gate): emit the EXACT FullCapability
    /// tool surface a Synapse-spawned local-model agent (gemma/DeepSeek) receives,
    /// as physical source of truth, plus the real byte size + token estimate of
    /// the OpenAI `tools` array that the local-agent harness puts on the
    /// chat-completion request body.
    ///
    /// Faithful reproduction of the production path:
    ///   1. a session whose MCP client identity is the local-model harness
    ///      (`synapse-local-model-agent`) resolves to `ToolProfileKind::FullCapability`,
    ///   2. `tools_for_session_profile` returns `full_sanitized_tools()` (=
    ///      `sanitize_tools(tool_router.list_all())`) sorted exactly as the
    ///      handler emits them — the same `Vec<Tool>` the agent's `tools/list`
    ///      receives,
    ///   3. each `Tool` is mapped through the SAME JSON shape as
    ///      `local_agent::openai_tool_from_mcp` (kept in sync below) to build the
    ///      `tools[]` field of the request body.
    ///
    /// Skipped unless `SYNAPSE_TOOL_SURFACE_OUT` is set to the absolute output
    /// path, so it never writes during a normal `cargo test` run.
    ///
    /// ```text
    /// SYNAPSE_TOOL_SURFACE_OUT=C:/code/synapse-subconscious/artifacts/tool_surface.json \
    ///   cargo test -p synapse-mcp --lib -- --ignored --exact \
    ///   server::tool_profiles::tests::emit_full_capability_tool_surface_artifact --nocapture
    /// ```
    #[test]
    #[ignore = "measurement probe; set SYNAPSE_TOOL_SURFACE_OUT to run"]
    fn emit_full_capability_tool_surface_artifact() {
        let Ok(out_path) = std::env::var("SYNAPSE_TOOL_SURFACE_OUT") else {
            eprintln!("SYNAPSE_TOOL_SURFACE_OUT not set; skipping artifact emission");
            return;
        };

        let dir = TempDir::new().expect("tmp");
        let service = service_with_db(dir.path());
        let session_id = "tool-surface-probe-local-session";
        // Production identity for the Synapse local-model harness; this is what
        // makes the session resolve to FullCapability automatically.
        seed_session_client(&service, session_id, "synapse-local-model-agent");

        // The exact Vec<Tool> the agent's tools/list receives for this session.
        let tools = service
            .tools_for_session_profile(Some(session_id))
            .expect("full-capability tool surface");

        // Hard SoT assertion: this session really is FullCapability.
        let row = service
            .read_tool_profile_assignment(session_id)
            .expect("read profile row")
            .expect("row exists");
        assert_eq!(row.record.profile, ToolProfileKind::FullCapability);

        // Map each Tool exactly as local_agent::openai_tool_from_mcp does.
        // (That fn lives in the binary's local_agent module; its body is a pure
        // JSON wrapper with no schema logic — the schema work already happened in
        // sanitize_tools above. Kept byte-identical here.)
        let openai_tool_from_mcp = |tool: &Tool| -> serde_json::Value {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool.name.as_ref(),
                    "description": tool
                        .description
                        .as_ref()
                        .map(|desc| desc.as_ref())
                        .unwrap_or("Synapse MCP tool"),
                    "parameters": serde_json::Value::Object((*tool.input_schema).clone()),
                }
            })
        };

        let openai_tools: Vec<serde_json::Value> = tools.iter().map(openai_tool_from_mcp).collect();

        // The actual `tools` field of the chat-completion request body.
        let openai_tools_json =
            serde_json::to_string(&openai_tools).expect("serialize openai tools array");
        let openai_tools_bytes = openai_tools_json.len();
        let openai_tools_chars = openai_tools_json.chars().count();

        // Per-tool detail (sanitized schema, exactly as emitted to the model) and
        // the longest descriptions by byte size.
        let mut tool_entries: Vec<serde_json::Value> = Vec::with_capacity(tools.len());
        let mut desc_sizes: Vec<(String, usize)> = Vec::with_capacity(tools.len());
        for tool in &tools {
            let name = tool.name.to_string();
            let description = tool
                .description
                .as_ref()
                .map(|desc| desc.as_ref())
                .unwrap_or("Synapse MCP tool")
                .to_owned();
            desc_sizes.push((name.clone(), description.len()));
            tool_entries.push(serde_json::json!({
                "name": name,
                "description": description,
                "input_schema": serde_json::Value::Object((*tool.input_schema).clone()),
            }));
        }
        desc_sizes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let longest_descriptions: Vec<serde_json::Value> = desc_sizes
            .iter()
            .take(10)
            .map(|(name, bytes)| serde_json::json!({ "name": name, "description_bytes": bytes }))
            .collect();

        // Token estimates over the actual openai_tools payload. No real
        // tokenizer crate is in the workspace, so report both common heuristics,
        // clearly labeled as estimates.
        let approx_tokens_chars_div_4 = (openai_tools_chars as f64 / 4.0).ceil() as u64;
        let approx_tokens_chars_div_3_5 = (openai_tools_chars as f64 / 3.5).ceil() as u64;

        let artifact = serde_json::json!({
            "_meta": {
                "description": "Exact FullCapability MCP tool surface a Synapse-spawned local-model agent (e.g. gemma4) receives, with the real openai tools[] payload size and token estimate.",
                "profile": row.record.profile.as_str(),
                "profile_source": row.record.source,
                "client_identity": "synapse-local-model-agent",
                "produced_by": "server::tool_profiles::tests::emit_full_capability_tool_surface_artifact",
                "env_gates": {
                    "SYNAPSE_DEBUG_TOOLS": std::env::var("SYNAPSE_DEBUG_TOOLS").ok(),
                    "SYNAPSE_ENABLE_EVERQUEST": std::env::var("SYNAPSE_ENABLE_EVERQUEST").ok(),
                },
            },
            "tool_count": tools.len(),
            "openai_tools_bytes": openai_tools_bytes,
            "openai_tools_chars": openai_tools_chars,
            "approx_tokens": {
                "note": "No real tokenizer crate is vendored in this workspace; both values are heuristic estimates over the openai_tools payload.",
                "chars_div_4": approx_tokens_chars_div_4,
                "chars_div_3_5": approx_tokens_chars_div_3_5,
            },
            "longest_descriptions_by_bytes": longest_descriptions,
            "tools": tool_entries,
        });

        let out = std::path::Path::new(&out_path);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).expect("create artifact parent dir");
        }
        let serialized = serde_json::to_string_pretty(&artifact).expect("serialize artifact");
        std::fs::write(out, serialized).expect("write artifact");

        eprintln!("TOOL_SURFACE tool_count={}", tools.len());
        eprintln!("TOOL_SURFACE openai_tools_bytes={openai_tools_bytes}");
        eprintln!("TOOL_SURFACE openai_tools_chars={openai_tools_chars}");
        eprintln!("TOOL_SURFACE approx_tokens_chars_div_4={approx_tokens_chars_div_4}");
        eprintln!("TOOL_SURFACE approx_tokens_chars_div_3_5={approx_tokens_chars_div_3_5}");
        for (name, bytes) in desc_sizes.iter().take(10) {
            eprintln!("TOOL_SURFACE longest_desc {name} {bytes}");
        }
        eprintln!("TOOL_SURFACE written_to={out_path}");
    }
}
