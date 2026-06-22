const PROTOCOL_VERSION = 1;
const BRIDGE_BUILD_ID = "synapse-chrome-bridge-2026-06-22-wait-function-v1";
const BRIDGE_BUILD_SHA256 = "ddc78a20ea4310e18fbb1d220f506006b22f56869fe58c08cfc83ea07f7cdde8";
const COMMAND_CAPABILITIES = Object.freeze([
  "alarmReconnect",
  "externalPopupRiskSuppression",
  "listTabs",
  "openTab",
  "closeTab",
  "targetInfo",
  "targetInfoPageText",
  "navigateTab",
  "activateTab",
  "pageVitals",
  "pageContent",
  "setContent",
  "ariaSnapshot",
  "assertPoll",
  "locateElements",
  "inspectElement",
  "scrollIntoView",
  "waitForText",
  "waitForFunction",
  "waitForSelector",
  "clock",
  "pageEvents",
  "domAction",
  "coordinateClick",
  "reloadSelf",
  "typeActiveElement",
  "setFieldValue"
]);
const EXPECTED_EXTENSION_ID = "leoocgnkjnplbfdbklajepahofecgfbk";
const DAEMON_BASE_URL = "http://127.0.0.1:7700";
const SET_CONTENT_SEED_PATH = "/__synapse_bridge_set_content_seed.html";
const ERROR_ATTACH_FAILED = "A11Y_CDP_ATTACH_FAILED";
const ERROR_AXTREE_FAILED = "A11Y_CDP_AXTREE_FAILED";
const ERROR_DEBUGGER_WARNING_UNSUPPRESSED = "A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED";
const ERROR_EXTENSION_TIMEOUT = "A11Y_CDP_EXTENSION_TIMEOUT";
const ERROR_EXTENSION_STALE = "CHROME_BRIDGE_EXTENSION_STALE";
const ERROR_EXTENSION_ID_MISMATCH = "SYNAPSE_CHROME_EXTENSION_ID_MISMATCH";
const ERROR_DAEMON_UNAVAILABLE = "SYNAPSE_CHROME_DAEMON_UNAVAILABLE";
const ERROR_CHROME_SCRIPTING_EXECUTE_FAILED = "CHROME_SCRIPTING_EXECUTE_FAILED";
const ERROR_CHROME_DOM_SELECTOR_INVALID = "CHROME_DOM_SELECTOR_INVALID";
const ERROR_CHROME_DOM_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
const ERROR_CHROME_DOM_ELEMENT_AMBIGUOUS = "CHROME_DOM_ELEMENT_AMBIGUOUS";
const ERROR_CHROME_DOM_ELEMENT_NOT_ACTIONABLE = "CHROME_DOM_ELEMENT_NOT_ACTIONABLE";
const ERROR_CHROME_DOM_ACTION_UNSUPPORTED = "CHROME_DOM_ACTION_UNSUPPORTED";
const ERROR_CHROME_DOM_ACTION_POSTCONDITION_FAILED = "CHROME_DOM_ACTION_POSTCONDITION_FAILED";
const TAB_TARGET_PREFIX = "chrome-tab:";
const BRIDGE_TOKEN_HEADER = "X-Synapse-Bridge-Token";
const DAEMON_WS_BASE_URL = "ws://127.0.0.1:7700";
const WINDOW_BOUNDS_SCALE_CANDIDATES = Object.freeze([1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3]);
const WEBSOCKET_KEEPALIVE_MS = 20000;
const RECONNECT_INITIAL_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const DISCONNECTED_KEEPALIVE_MS = 20000;
const RECONNECT_WAKE_ALARM_NAME = "synapse-daemon-bridge-reconnect";
const RECONNECT_WAKE_ALARM_DELAY_MINUTES = 0.5;
const RECONNECT_WAKE_ALARM_PERIOD_MINUTES = 0.5;
const AGENT_NAVIGATION_CLAIM_TTL_MS = 30000;
const MAX_RECENT_NAVIGATION_KEYS = 128;
const MAX_PAGE_TEXT_CHARS = 4096;
const MAX_PAGE_EVENT_BUFFER = 1000;
const OPEN_WINDOW_BOUNDS_TOLERANCE_PX = 96;
const EXTERNAL_POPUP_RISK_PERMISSIONS = Object.freeze(["debugger", "nativeMessaging"]);
const POPUP_RISK_SUPPRESSION_RECHECK_MS = 60000;

let hostId = null;
let bridgeToken = null;
let connectInFlight = null;
let webSocket = null;
let keepAliveTimer = null;
let reconnectTimer = null;
let reconnectAttempt = 0;
let disconnectedKeepAliveTimer = null;
let permanentlyDisabled = false;
const agentNavigationClaims = new Map();
const recentNavigationKeys = [];
const pageEventBuffers = new Map();
let popupRiskSuppressionInFlight = null;
let popupRiskSuppressionState = {
  ok: false,
  status: "not_checked",
  reason: "not_checked",
  checked_at_unix_ms: 0,
  hazard_count: 0,
  remaining_hazard_count: 0,
  disabled_count: 0,
  failure_count: 0,
  management_available: Boolean(chrome.management?.getAll),
  hazards: [],
  disabled: [],
  remaining_hazards: [],
  failures: []
};

async function startBridge() {
  if (chrome.runtime.id !== EXPECTED_EXTENSION_ID) {
    disableBridgePermanently(
      `extension id mismatch: actual=${chrome.runtime.id} expected=${EXPECTED_EXTENSION_ID}; ` +
        `reload the unpacked extension from the Synapse extension directory so the daemon can ` +
        `authenticate the bridge origin`,
      ERROR_EXTENSION_ID_MISMATCH
    );
    return;
  }
  if (runtimeDebuggerApiAvailable()) {
    disableBridgePermanently(
      `loaded runtime exposes chrome.debugger; normal Synapse Chrome Bridge must be ` +
        `debugger-free because Chrome's debugger infobar changes browser layout. ` +
        `Reload the bundled extension from the Synapse repo before any normal-profile ` +
        `browser work can proceed.`,
      ERROR_DEBUGGER_WARNING_UNSUPPRESSED
    );
    return;
  }
  ensureReconnectWakeAlarm();
  await ensureExternalPopupRiskSuppression("startup");
  connectDaemon();
}

function startBridgeFromEvent() {
  startBridge().catch((error) => {
    console.error(`Synapse daemon bridge startup failed: ${errorMessage(error)}`);
    connectDaemon();
  });
}

function connectDaemon() {
  if (permanentlyDisabled) {
    return;
  }
  if (connectInFlight) {
    return;
  }
  if (webSocket && isWebSocketUsable(webSocket)) {
    return;
  }
  if (webSocket && !isWebSocketUsable(webSocket)) {
    closeWebSocket();
  }
  if (hostId && bridgeToken) {
    connectWebSocket();
    return;
  }
  clearReconnectTimer();
  connectInFlight = ensureExternalPopupRiskSuppression("connectDaemon")
    .then(() => registerDaemon())
    .catch((error) => {
      if (error?.code === ERROR_DEBUGGER_WARNING_UNSUPPRESSED) {
        scheduleReconnect(
          `direct daemon register refused unsafe profile: ${errorMessage(error)}`,
          ERROR_DEBUGGER_WARNING_UNSUPPRESSED
        );
        return;
      }
      scheduleReconnect(
        `direct daemon register failed: ${errorMessage(error)}`,
        ERROR_DAEMON_UNAVAILABLE
      );
    })
    .finally(() => {
      connectInFlight = null;
    });
}

async function registerDaemon() {
  const registered = await daemonFetchJson("/chrome-debugger/native/register", {
    method: "POST",
    body: {
      origin: chrome.runtime.getURL(""),
      pid: 0,
      parent_window: null,
      bridge_protocol_version: PROTOCOL_VERSION,
      transport: "direct_http"
    }
  });
  if (
    !registered?.ok ||
    typeof registered.host_id !== "string" ||
    !registered.host_id ||
    typeof registered.bridge_token !== "string" ||
    !registered.bridge_token
  ) {
    throw new Error(`daemon register returned invalid response ${JSON.stringify(registered)}`);
  }
  hostId = registered.host_id;
  bridgeToken = registered.bridge_token;
  await postDaemonMessage({
    type: "hello",
    transport: "direct_http",
    userAgent: navigator.userAgent,
    ...bridgeIdentity()
  });
  connectWebSocket();
}

function disableBridgePermanently(detail, code) {
  closeWebSocket();
  clearReconnectTimer();
  stopDisconnectedKeepAlive();
  permanentlyDisabled = true;
  hostId = null;
  bridgeToken = null;
  console.error(
    `Synapse daemon bridge disabled permanently for this extension load: ${detail}; ` +
      `code=${code}`
  );
}

function scheduleReconnect(detail, code) {
  if (permanentlyDisabled) {
    return;
  }
  closeWebSocket();
  hostId = null;
  bridgeToken = null;
  const delayMs = Math.min(
    RECONNECT_MAX_MS,
    RECONNECT_INITIAL_MS * 2 ** Math.min(reconnectAttempt, 5)
  );
  reconnectAttempt += 1;
  console.warn(
    `Synapse daemon bridge disconnected; reconnect scheduled: code=${code} ` +
      `attempt=${reconnectAttempt} delay_ms=${delayMs} detail=${detail}`
  );
  startDisconnectedKeepAlive();
  if (reconnectTimer) {
    return;
  }
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectDaemon();
  }, delayMs);
}

function clearReconnectTimer() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
}

function resetReconnectState() {
  reconnectAttempt = 0;
  clearReconnectTimer();
  stopDisconnectedKeepAlive();
  ensureReconnectWakeAlarm();
}

async function ensureExternalPopupRiskSuppression(reason) {
  const now = Date.now();
  if (
    popupRiskSuppressionState.status !== "not_checked" &&
    now - popupRiskSuppressionState.checked_at_unix_ms < POPUP_RISK_SUPPRESSION_RECHECK_MS
  ) {
    return popupRiskSuppressionState;
  }
  if (popupRiskSuppressionInFlight) {
    return popupRiskSuppressionInFlight;
  }
  popupRiskSuppressionInFlight = refreshExternalPopupRiskSuppression(reason)
    .catch((error) => {
      popupRiskSuppressionState = {
        ok: false,
        status: "suppression_error",
        reason,
        checked_at_unix_ms: Date.now(),
        hazard_count: 0,
        remaining_hazard_count: 0,
        disabled_count: 0,
        failure_count: 1,
        management_available: Boolean(chrome.management?.getAll),
        hazards: [],
        disabled: [],
        remaining_hazards: [],
        failures: [{
          id: "<suppression_scan>",
          name: "suppression_scan",
          error: errorMessage(error)
        }]
      };
      return popupRiskSuppressionState;
    })
    .finally(() => {
      popupRiskSuppressionInFlight = null;
    });
  return popupRiskSuppressionInFlight;
}

async function requireExternalPopupRisksSuppressed(commandKind, params) {
  if (!commandRequiresExternalPopupSuppression(commandKind)) {
    return;
  }
  const state = await ensureExternalPopupRiskSuppression(`command:${String(commandKind)}`);
  if (state.ok && state.remaining_hazard_count === 0) {
    return;
  }
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `normal Synapse Chrome Bridge refused ${String(commandKind)} before queueing a Chrome ` +
      `tabs/scripting command because external debugger/nativeMessaging popup risk remains ` +
      `unsuppressed; hwnd=${String(params?.hwnd ?? "unknown")} ` +
      formatPopupRiskSuppressionForError(state)
  );
}

function commandRequiresExternalPopupSuppression(kind) {
  return [
    "openTab",
    "listTabs",
    "closeTab",
    "targetInfo",
    "targetInfoPageText",
    "typeActiveElement",
    "setFieldValue",
    "pageVitals",
    "pageContent",
    "setContent",
    "ariaSnapshot",
    "assertPoll",
    "locateElements",
    "inspectElement",
    "scrollIntoView",
    "waitForText",
    "waitForFunction",
    "waitForSelector",
    "clock",
    "pageEvents",
    "navigateTab",
    "activateTab",
    "domAction",
    "coordinateClick"
  ].includes(kind);
}

async function refreshExternalPopupRiskSuppression(reason) {
  const checkedAt = Date.now();
  const managementAvailable = Boolean(chrome.management?.getAll);
  if (!managementAvailable) {
    popupRiskSuppressionState = {
      ok: false,
      status: "management_unavailable",
      reason,
      checked_at_unix_ms: checkedAt,
      hazard_count: 0,
      remaining_hazard_count: 0,
      disabled_count: 0,
      failure_count: 1,
      management_available: false,
      hazards: [],
      disabled: [],
      remaining_hazards: [],
      failures: [{
        id: "<chrome.management>",
        name: "chrome.management",
        error: "management permission/API unavailable"
      }]
    };
    return popupRiskSuppressionState;
  }

  const before = await chrome.management.getAll();
  const hazards = enabledExternalPopupRiskExtensions(before);
  const disabled = [];
  const failures = [];

  for (const hazard of hazards) {
    if (hazard.may_disable === false) {
      failures.push({
        ...hazard,
        error: "chrome.management reports mayDisable=false"
      });
      continue;
    }
    try {
      await chrome.management.setEnabled(hazard.id, false);
      disabled.push(hazard);
    } catch (error) {
      failures.push({
        ...hazard,
        error: errorMessage(error)
      });
    }
  }

  const after = disabled.length > 0 ? await chrome.management.getAll() : before;
  const remaining = enabledExternalPopupRiskExtensions(after);
  const remainingIds = new Set(remaining.map((entry) => entry.id));
  for (const entry of disabled) {
    if (remainingIds.has(entry.id)) {
      failures.push({
        ...entry,
        error: "chrome.management.setEnabled returned but extension remained enabled"
      });
    }
  }

  const ok = remaining.length === 0 && failures.length === 0;
  popupRiskSuppressionState = {
    ok,
    status: ok ? (hazards.length > 0 ? "suppressed" : "clear") : "unsuppressed",
    reason,
    checked_at_unix_ms: checkedAt,
    hazard_count: hazards.length,
    remaining_hazard_count: remaining.length,
    disabled_count: disabled.length,
    failure_count: failures.length,
    management_available: true,
    hazards,
    disabled,
    remaining_hazards: remaining,
    failures
  };
  if (!ok) {
    console.warn(
      `Synapse external popup risk suppression incomplete: ` +
        formatPopupRiskSuppressionForError(popupRiskSuppressionState)
    );
  } else if (disabled.length > 0) {
    console.warn(
      `Synapse disabled ${disabled.length} external debugger/nativeMessaging extension(s): ` +
        disabled.map((entry) => `${entry.id}:${entry.name}`).join(" | ")
    );
  }
  return popupRiskSuppressionState;
}

function enabledExternalPopupRiskExtensions(items) {
  return (Array.isArray(items) ? items : [])
    .filter((info) => info?.id && info.id !== chrome.runtime.id && info.enabled === true)
    .map((info) => {
      const permissions = Array.isArray(info.permissions) ? info.permissions : [];
      const hazardPermissions = permissions
        .filter((permission) => EXTERNAL_POPUP_RISK_PERMISSIONS.includes(permission))
        .sort();
      return {
        id: String(info.id),
        name: String(info.name || ""),
        type: String(info.type || ""),
        install_type: String(info.installType || ""),
        may_disable: info.mayDisable !== false,
        permissions: permissions.map(String).sort(),
        hazard_permissions: [...new Set(hazardPermissions)],
        enabled: info.enabled === true,
        disabled_reason: info.disabledReason ? String(info.disabledReason) : ""
      };
    })
    .filter((info) => info.hazard_permissions.length > 0)
    .sort((a, b) => a.id.localeCompare(b.id));
}

function popupRiskSuppressionSnapshot() {
  return JSON.parse(JSON.stringify(popupRiskSuppressionState));
}

function formatPopupRiskSuppressionForError(state) {
  const remaining = Array.isArray(state?.remaining_hazards) ? state.remaining_hazards : [];
  const failures = Array.isArray(state?.failures) ? state.failures : [];
  const remainingText = remaining.length > 0
    ? remaining
        .slice(0, 8)
        .map((entry) => `${entry.id}:${entry.name}:${entry.hazard_permissions.join(",")}`)
        .join(" | ")
    : "<none>";
  const failureText = failures.length > 0
    ? failures
        .slice(0, 8)
        .map((entry) => `${entry.id}:${entry.name}:${entry.error || "unknown"}`)
        .join(" | ")
    : "<none>";
  return (
    `suppression_status=${String(state?.status || "unknown")} ` +
    `suppression_reason=${String(state?.reason || "unknown")} ` +
    `management_available=${Boolean(state?.management_available)} ` +
    `hazard_count=${Number(state?.hazard_count || 0)} ` +
    `disabled_count=${Number(state?.disabled_count || 0)} ` +
    `remaining_hazard_count=${Number(state?.remaining_hazard_count || 0)} ` +
    `failure_count=${Number(state?.failure_count || 0)} ` +
    `remaining_hazards=${remainingText} failures=${failureText} ` +
    `remediation=disable the named extension IDs in Chrome or repair ` +
    `HKCU\\Software\\Policies\\Google\\Chrome ACL so Synapse can apply ` +
    `ExtensionSettings blocked_permissions for debugger/nativeMessaging`
  );
}

function connectWebSocket() {
  if (permanentlyDisabled || !hostId || !bridgeToken || webSocket) {
    return;
  }
  const url = new URL("/chrome-debugger/native/ws", DAEMON_WS_BASE_URL);
  url.searchParams.set("host_id", hostId);
  url.searchParams.set("bridge_token", bridgeToken);
  const socket = new WebSocket(url.toString());
  webSocket = socket;
  socket.onopen = () => {
    resetReconnectState();
    console.info(`Synapse daemon bridge connected: host_id=${hostId}`);
    startWebSocketKeepAlive(socket);
  };
  socket.onmessage = (event) => {
    handleWebSocketMessage(event.data).catch((error) => {
      scheduleReconnect(
        `direct daemon websocket message failed: ${errorMessage(error)}`,
        ERROR_DAEMON_UNAVAILABLE
      );
    });
  };
  socket.onerror = () => {
    if (webSocket === socket) {
      scheduleReconnect("direct daemon websocket error", ERROR_DAEMON_UNAVAILABLE);
    }
  };
  socket.onclose = () => {
    if (webSocket === socket) {
      scheduleReconnect("direct daemon websocket closed", ERROR_DAEMON_UNAVAILABLE);
    }
  };
}

function isWebSocketUsable(socket) {
  return socket.readyState === WebSocket.CONNECTING || socket.readyState === WebSocket.OPEN;
}

async function handleWebSocketMessage(raw) {
  if (typeof raw !== "string") {
    throw new Error(`daemon websocket returned non-text payload type=${typeof raw}`);
  }
  let message;
  try {
    message = JSON.parse(raw);
  } catch (_) {
    throw new Error(`daemon websocket returned non-JSON payload=${JSON.stringify(raw.slice(0, 512))}`);
  }
  if (message?.ok === false) {
    throw new Error(`daemon websocket refused command delivery: ${JSON.stringify(message)}`);
  }
  if (message?.command) {
    await handleCommand(message.command);
  }
}

function startWebSocketKeepAlive(socket) {
  stopWebSocketKeepAlive();
  keepAliveTimer = setInterval(() => {
    if (webSocket !== socket || socket.readyState !== WebSocket.OPEN) {
      stopWebSocketKeepAlive();
      return;
    }
    try {
      socket.send(JSON.stringify({
        type: "keepalive",
        host_id: hostId,
        sent_at_unix_ms: Date.now()
      }));
    } catch (error) {
      scheduleReconnect(
        `direct daemon websocket keepalive failed: ${errorMessage(error)}`,
        ERROR_DAEMON_UNAVAILABLE
      );
    }
  }, WEBSOCKET_KEEPALIVE_MS);
}

function stopWebSocketKeepAlive() {
  if (keepAliveTimer) {
    clearInterval(keepAliveTimer);
    keepAliveTimer = null;
  }
}

function startDisconnectedKeepAlive() {
  ensureReconnectWakeAlarm();
  if (disconnectedKeepAliveTimer) {
    return;
  }
  disconnectedKeepAliveTimer = setInterval(() => {
    if (permanentlyDisabled || (webSocket && isWebSocketUsable(webSocket))) {
      stopDisconnectedKeepAlive();
      return;
    }
    try {
      const platformInfo = chrome.runtime.getPlatformInfo();
      if (platformInfo && typeof platformInfo.catch === "function") {
        platformInfo.catch((error) => {
          console.warn(
            `Synapse daemon bridge reconnect keepalive failed: ${errorMessage(error)}`
          );
        });
      }
    } catch (error) {
      console.warn(`Synapse daemon bridge reconnect keepalive threw: ${errorMessage(error)}`);
    }
  }, DISCONNECTED_KEEPALIVE_MS);
}

function stopDisconnectedKeepAlive() {
  if (disconnectedKeepAliveTimer) {
    clearInterval(disconnectedKeepAliveTimer);
    disconnectedKeepAliveTimer = null;
  }
}

function ensureReconnectWakeAlarm() {
  if (!chrome.alarms?.create) {
    console.error(
      "Synapse daemon bridge cannot register reconnect wake alarm; " +
        "manifest must include required chrome.alarms permission"
    );
    return;
  }
  try {
    const created = chrome.alarms.create(RECONNECT_WAKE_ALARM_NAME, {
      delayInMinutes: RECONNECT_WAKE_ALARM_DELAY_MINUTES,
      periodInMinutes: RECONNECT_WAKE_ALARM_PERIOD_MINUTES
    });
    if (created && typeof created.catch === "function") {
      created.catch((error) => {
        console.warn(`Synapse reconnect wake alarm setup failed: ${errorMessage(error)}`);
      });
    }
  } catch (error) {
    console.warn(`Synapse reconnect wake alarm setup threw: ${errorMessage(error)}`);
  }
}

function handleReconnectWakeAlarm(alarm) {
  if (alarm?.name !== RECONNECT_WAKE_ALARM_NAME || permanentlyDisabled) {
    return;
  }
  startBridgeFromEvent();
}

function handleManagementStateChanged(info) {
  if (!info || info.id === chrome.runtime.id) {
    return;
  }
  popupRiskSuppressionState = {
    ...popupRiskSuppressionState,
    ok: false,
    status: "stale",
    reason: "chrome.management event",
    checked_at_unix_ms: 0
  };
  ensureExternalPopupRiskSuppression("chrome.management event").catch((error) => {
    console.warn(`Synapse popup risk suppression refresh failed: ${errorMessage(error)}`);
  });
}

function closeWebSocket() {
  stopWebSocketKeepAlive();
  const socket = webSocket;
  webSocket = null;
  if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
    try {
      socket.close();
    } catch (_) {
      // Closing is best-effort during reconnect cleanup.
    }
  }
}

chrome.runtime.onInstalled.addListener(startBridgeFromEvent);
chrome.runtime.onStartup.addListener(startBridgeFromEvent);
if (chrome.alarms?.onAlarm?.addListener) {
  chrome.alarms.onAlarm.addListener(handleReconnectWakeAlarm);
}
if (chrome.management?.onEnabled?.addListener) {
  chrome.management.onEnabled.addListener(handleManagementStateChanged);
}
if (chrome.management?.onInstalled?.addListener) {
  chrome.management.onInstalled.addListener(handleManagementStateChanged);
}
if (chrome.management?.onDisabled?.addListener) {
  chrome.management.onDisabled.addListener(handleManagementStateChanged);
}
chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (!changeInfo?.url && !changeInfo?.title && changeInfo?.status !== "complete") {
    return;
  }
  updatePageEventTabState(tabId, tab, changeInfo);
  postTabNavigationEvent("tabs.onUpdated", tab).catch((error) => {
    console.warn(`Synapse tab navigation event dropped: ${errorMessage(error)}`);
  });
});
chrome.tabs.onCreated.addListener((tab) => {
  recordTabCreatedForPageEvents(tab);
});
chrome.tabs.onRemoved.addListener((tabId) => {
  recordTabRemovedForPageEvents(tabId);
});
chrome.tabs.onActivated.addListener((activeInfo) => {
  chrome.tabs.get(activeInfo.tabId)
    .then((tab) => postTabNavigationEvent("tabs.onActivated", tab))
    .catch((error) => {
      console.warn(`Synapse active tab navigation readback failed: ${errorMessage(error)}`);
    });
});
if (chrome.webNavigation?.onBeforeNavigate?.addListener) {
  chrome.webNavigation.onBeforeNavigate.addListener((details) => {
    recordWebNavigationPageEvent("framestartednavigating", details);
  });
}
if (chrome.webNavigation?.onCommitted?.addListener) {
  chrome.webNavigation.onCommitted.addListener((details) => {
    recordWebNavigationPageEvent("framenavigated", details, {
      navigation_type: webNavigationTransition(details)
    });
  });
}
if (chrome.webNavigation?.onDOMContentLoaded?.addListener) {
  chrome.webNavigation.onDOMContentLoaded.addListener((details) => {
    recordWebNavigationPageEvent("domcontentloaded", details);
  });
}
if (chrome.webNavigation?.onCompleted?.addListener) {
  chrome.webNavigation.onCompleted.addListener((details) => {
    recordWebNavigationPageEvent("load", details);
    recordWebNavigationPageEvent("framestoppedloading", details);
  });
}
if (chrome.webNavigation?.onHistoryStateUpdated?.addListener) {
  chrome.webNavigation.onHistoryStateUpdated.addListener((details) => {
    recordWebNavigationPageEvent("framenavigated", details, {
      navigation_type: webNavigationTransition(details) || "history_api"
    });
  });
}

startBridgeFromEvent();

async function handleCommand(command) {
  if (!command || typeof command !== "object") {
    throw bridgeError(ERROR_ATTACH_FAILED, "native command was not an object");
  }
  const { id, kind, params = {} } = command;
  if (!id || typeof id !== "string") {
    throw bridgeError(ERROR_ATTACH_FAILED, "native command id is required");
  }
  try {
    let result;
    let reloadAfterResponse = false;
    rejectIfDebuggerApiAvailable(kind, params);
    await requireExternalPopupRisksSuppressed(kind, params);
    if (kind === "snapshot") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "clickNode") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "typeNode") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "nodeValue") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "openTab") {
      result = await handleOpenTab(params);
    } else if (kind === "listTabs") {
      result = await handleListTabs(params);
    } else if (kind === "closeTab") {
      result = await handleCloseTab(params);
    } else if (kind === "capturePageScreenshot") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "targetInfo" || kind === "targetInfoPageText") {
      result = await handleTargetInfo(params);
    } else if (kind === "pageContent") {
      result = await handlePageContent(params);
    } else if (kind === "setContent") {
      result = await handleSetContent(params);
    } else if (kind === "ariaSnapshot") {
      result = await handleAriaSnapshot(params);
    } else if (kind === "assertPoll") {
      result = await handleAssertPoll(params);
    } else if (kind === "locateElements") {
      result = await handleLocateElements(params);
    } else if (kind === "inspectElement") {
      result = await handleInspectElement(params);
    } else if (kind === "scrollIntoView") {
      result = await handleScrollIntoView(params);
    } else if (kind === "waitForText") {
      result = await handleWaitForText(params);
    } else if (kind === "waitForFunction") {
      result = await handleWaitForFunction(params);
    } else if (kind === "waitForSelector") {
      result = await handleWaitForSelector(params);
    } else if (kind === "clock") {
      result = await handleClock(params);
    } else if (kind === "pageEvents") {
      result = await handlePageEvents(params);
    } else if (kind === "typeActiveElement") {
      result = await handleTypeActiveElement(params);
    } else if (kind === "setFieldValue") {
      result = await handleSetFieldValue(params);
    } else if (kind === "evaluateScript") {
      result = rejectAttachCommand(kind, params);
    } else if (kind === "pageVitals") {
      result = await handlePageVitals(params);
    } else if (kind === "navigateTab") {
      result = await handleNavigateTab(params);
    } else if (kind === "activateTab") {
      result = await handleActivateTab(params);
    } else if (kind === "domAction") {
      result = await handleDomAction(params);
    } else if (kind === "coordinateClick") {
      result = await handleCoordinateClick(params);
    } else if (kind === "reloadSelf") {
      result = handleReloadSelf(params);
      reloadAfterResponse = true;
    } else {
      throw bridgeError(
        ERROR_EXTENSION_STALE,
        `unknown command kind ${String(kind)}; loaded_build_id=${BRIDGE_BUILD_ID} ` +
          `loaded_capabilities=${COMMAND_CAPABILITIES.join(",")}`
      );
    }
    await postResponse(id, true, result, null);
    if (reloadAfterResponse) {
      scheduleRuntimeReload(result.reload_delay_ms);
    }
  } catch (error) {
    await postResponse(id, false, null, errorPayload(error));
  }
}

function bridgeIdentity() {
  return {
    extensionId: chrome.runtime.id,
    version: chrome.runtime.getManifest().version,
    protocolVersion: PROTOCOL_VERSION,
    buildId: BRIDGE_BUILD_ID,
    buildSha256: BRIDGE_BUILD_SHA256,
    debuggerApiAvailable: runtimeDebuggerApiAvailable(),
    capabilities: [...COMMAND_CAPABILITIES],
    commandCapabilities: [...COMMAND_CAPABILITIES],
    popupRiskSuppression: popupRiskSuppressionSnapshot()
  };
}

function runtimeDebuggerApiAvailable() {
  return Boolean(
    chrome.debugger &&
    (chrome.debugger.attach || chrome.debugger.sendCommand || chrome.debugger.getTargets)
  );
}

function rejectIfDebuggerApiAvailable(kind, params) {
  if (!runtimeDebuggerApiAvailable()) {
    return;
  }
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `normal Synapse Chrome Bridge refused ${String(kind)} before browser work; ` +
      `loaded runtime exposes chrome.debugger even though the normal bridge must be ` +
      `debugger-free. Chrome's debugger infobar changes browser layout and breaks ` +
      `coordinate truth. hwnd=${String(params?.hwnd ?? "unknown")} remediation=reload ` +
      `the bundled Synapse extension from the repo and re-read mcp__synapse.health ` +
      `until debuggerApiAvailable=false`
  );
}

function rejectAttachCommand(kind, params) {
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `normal Synapse Chrome Bridge refused ${String(kind)} before browser debugger startup; ` +
      `the normal end-user extension is debugger-free and cannot call chrome.debugger. ` +
      `hwnd=${String(params?.hwnd ?? "unknown")} remediation=use raw CDP with a dedicated ` +
      `Synapse-launched automation profile`
  );
}

async function handleOpenTab(params) {
  const requestedUrl = normalizeOpenUrl(params.url);
  const agentSessionId = normalizeOptionalSessionId(params.agentSessionId);
  const beforePages = await tabTargets();
  const openWindow = await selectOpenWindowForHwndHint(params);
  let tab;
  const createParams = {
    url: requestedUrl || "about:blank",
    active: false
  };
  if (Number.isInteger(openWindow?.windowId)) {
    createParams.windowId = openWindow.windowId;
  }
  try {
    tab = await chrome.tabs.create(createParams);
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.tabs.create(active=false): ${errorMessage(error)}`);
  }
  if (!tab || typeof tab.id !== "number") {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.tabs.create returned no numeric tab id");
  }
  markAgentNavigation(tab.id, {
    action: "open",
    requestedUrl: requestedUrl || "about:blank",
    sessionId: agentSessionId
  });
  const target = await waitForTabTarget(tab.id, 10000);
  const state = await tabPageState(tab.id, target);
  if (
    Number.isInteger(openWindow?.windowId) &&
    Number.isInteger(state.chrome_window_id) &&
    state.chrome_window_id !== openWindow.windowId
  ) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.tabs.create returned tab in Chrome window ${state.chrome_window_id}, expected ${openWindow.windowId}`
    );
  }
  const openedWindow = Number.isInteger(state.chrome_window_id)
    ? await chromeWindowState(state.chrome_window_id)
    : null;
  const afterPages = await tabTargets();
  return {
    extension_id: chrome.runtime.id,
    target_id: target.id,
    tab_id: tab.id,
    chrome_window_id: state.chrome_window_id,
    chrome_window_focused: typeof openedWindow?.focused === "boolean" ? openedWindow.focused : null,
    chrome_window_state: typeof openedWindow?.state === "string" ? openedWindow.state : "",
    chrome_window_selection_reason: openWindow?.selectionReason || "default_chrome_tabs_create",
    chrome_window_candidate_count: openWindow?.candidateCount || 0,
    chrome_window_non_focused_count: openWindow?.nonFocusedCount || 0,
    target_active: Boolean(state.active),
    target_highlighted: Boolean(state.highlighted),
    target_type: target.type || "page",
    url: state.url || target.url || tab.url || requestedUrl || "about:blank",
    title: state.title || target.title || tab.title || "",
    target_attached: Boolean(target.attached),
    target_count_before: beforePages.length,
    target_count_after: afterPages.length
  };
}

async function handleCloseTab(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const beforePages = await tabTargets();
  try {
    await chrome.tabs.remove(selected.tabId);
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.tabs.remove(${selected.tabId}): ${errorMessage(error)}`);
  }
  const afterPages = await waitForTargetAbsent(selected.target.id, 10000);
  return {
    extension_id: chrome.runtime.id,
    target_id: selected.target.id,
    tab_id: selected.tabId,
    target_count_before: beforePages.length,
    target_count_after: afterPages.length
  };
}

async function handleListTabs(params) {
  const openWindow = await selectOpenWindowForHwndHint(params);
  const query = {};
  if (Number.isInteger(openWindow?.windowId)) {
    query.windowId = openWindow.windowId;
  }
  let tabs;
  try {
    tabs = await chrome.tabs.query(query);
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.tabs.query(${JSON.stringify(query)}): ${errorMessage(error)}`);
  }
  const targets = tabs
    .filter((tab) => typeof tab.id === "number")
    .map((tab) => tabListEntryFromTabTarget(tabTargetFromTab(tab), tab));
  let selectedWindow = null;
  if (Number.isInteger(openWindow?.windowId)) {
    selectedWindow = await chromeWindowState(openWindow.windowId);
  }
  return {
    extension_id: chrome.runtime.id,
    chrome_window_id: Number.isInteger(openWindow?.windowId) ? openWindow.windowId : null,
    chrome_window_focused: typeof selectedWindow?.focused === "boolean" ? selectedWindow.focused : null,
    chrome_window_state: typeof selectedWindow?.state === "string" ? selectedWindow.state : "",
    chrome_window_selection_reason: openWindow?.selectionReason || "all_chrome_windows",
    chrome_window_candidate_count: openWindow?.candidateCount || 0,
    chrome_window_non_focused_count: openWindow?.nonFocusedCount || 0,
    target_count: targets.length,
    active_tab_count: targets.filter((target) => target.active).length,
    tabs: targets
  };
}

async function handleTargetInfo(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = await tabPageState(selected.tabId, selected.target);
  const activeElement = await tabActiveElementState(selected.tabId);
  const pageText = await tabPageTextState(selected.tabId);
  const pageVitals = await tabPageVitalsState(selected.tabId);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    target_type: state.target_type || selected.target.type || "page",
    url: state.url || "",
    title: state.title || "",
    ready_state: state.ready_state || "",
    active: Boolean(state.active),
    highlighted: Boolean(state.highlighted),
    pinned: Boolean(state.pinned),
    readback_backend: "chrome.tabs.get+chrome.scripting.executeScript",
    active_element: activeElement,
    page_text: pageText,
    page_vitals: pageVitals,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handlePageVitals(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = await tabPageState(selected.tabId, selected.target);
  const pageVitals = await tabPageVitalsState(selected.tabId);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: state.url || "",
    title: state.title || "",
    ready_state: state.ready_state || "",
    readback_backend: "chrome.tabs.get+chrome.scripting.executeScript",
    page_vitals: pageVitals,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handlePageContent(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const maxBytes = normalizeMaxContentBytes(params.maxBytes);
  const state = await tabPageState(selected.tabId, selected.target);
  const content = await tabPageContentState(selected.tabId, maxBytes);
  if (!content.available) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `pageContent read failed for tab ${selected.tabId}: code=${String(content.error_code || "UNKNOWN")} ` +
        `detail=${String(content.error_detail || "")}`
    );
  }
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: state.url || content.url || "",
    title: state.title || content.title || "",
    ready_state: content.ready_state || state.ready_state || "",
    history_current_index: Number.isSafeInteger(content.history_current_index) ? content.history_current_index : -1,
    history_entry_count: Number.isSafeInteger(content.history_entry_count) ? content.history_entry_count : 0,
    html: content.html || "",
    html_len: Number.isSafeInteger(content.html_len) ? content.html_len : 0,
    truncated: Boolean(content.truncated),
    max_bytes: maxBytes,
    readback_backend: "chrome.scripting.executeScript(document.documentElement.outerHTML)",
    frame_id: content.frame_id,
    frame_document_id: content.frame_document_id,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleSetContent(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const html = String(params.html ?? "");
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  const agentSessionId = normalizeOptionalSessionId(params.agentSessionId);
  if (html.length === 0) {
    throw bridgeError(ERROR_AXTREE_FAILED, "setContent requires non-empty html");
  }
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  const before = await tabPageState(selected.tabId, selected.target);
  const execution = await executeSetContentScript(selected, html, waitTimeoutMs, agentSessionId, before);
  const { injected, seed } = execution;
  const frameResults = frameExecutionResults(injected);
  const first = frameResults.find((frame) => frame.result) || null;
  const result = first?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.scripting.executeScript setContent returned no structured result");
  }
  if (!result.ok) {
    throw bridgeError(
      String(result.error_code || ERROR_AXTREE_FAILED),
      `setContent failed: ${String(result.error_detail || "")}`
    );
  }
  const after = await waitForTabPageState(
    selected.tabId,
    selected.target,
    waitTimeoutMs,
    {
      description: "document.readyState complete after setContent",
      matches: (state) => state.ready_state === "complete"
    }
  );
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: after.chrome_window_id,
    before_url: before.url || "",
    before_title: before.title || "",
    after_url: after.url || result.url || "",
    after_title: after.title || result.title || "",
    ready_state: result.ready_state || after.ready_state || "",
    history_current_index: Number.isSafeInteger(result.history_current_index) ? result.history_current_index : -1,
    history_entry_count: Number.isSafeInteger(result.history_entry_count) ? result.history_entry_count : 0,
    html_len: Number.isSafeInteger(result.html_len) ? result.html_len : html.length,
    seeded_url: seed?.url || "",
    seeded_from_url: seed?.from_url || "",
    seeded_reason: seed?.reason || "",
    readback_backend: "chrome.scripting.executeScript(document.open/write/close)+chrome.tabs.get",
    backend_tier_used: "chrome_tabs_extension",
    frame_id: Number.isSafeInteger(first.frame_id) ? first.frame_id : null,
    frame_document_id: first.document_id,
    frame_result_count: frameResults.length,
    frame_results: frameResults.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function executeSetContentScript(selected, html, waitTimeoutMs, agentSessionId, before) {
  try {
    const injected = await runSetContentScript(selected.tabId, html, waitTimeoutMs);
    return { injected, seed: null };
  } catch (error) {
    if (!isScriptingAccessDeniedError(error)) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `chrome.scripting.executeScript setContent(${selected.tabId}) failed: ${errorMessage(error)}`
      );
    }
    const seed = await seedTabForSetContent(selected, waitTimeoutMs, agentSessionId, before, error);
    try {
      const injected = await runSetContentScript(selected.tabId, html, waitTimeoutMs);
      return { injected, seed };
    } catch (retryError) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `chrome.scripting.executeScript setContent(${selected.tabId}) failed after seed navigation ` +
          `url=${JSON.stringify(seed.url)} original=${errorMessage(error)} retry=${errorMessage(retryError)}`
      );
    }
  }
}

async function runSetContentScript(tabId, html, waitTimeoutMs) {
  return chrome.scripting.executeScript({
    target: { tabId },
    world: "MAIN",
    func: setContentInPage,
    args: [{ html, waitTimeoutMs }]
  });
}

async function seedTabForSetContent(selected, waitTimeoutMs, agentSessionId, before, cause) {
  const seedUrl = setContentSeedUrl(selected.tabId);
  markAgentNavigation(selected.tabId, {
    action: "setContentSeed",
    requestedUrl: seedUrl,
    sessionId: agentSessionId
  });
  try {
    await chrome.tabs.update(selected.tabId, { url: seedUrl });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.tabs.update(${selected.tabId}) seed for setContent failed: ${errorMessage(error)}; ` +
        `original executeScript failure=${errorMessage(cause)}`
    );
  }
  await waitForTabPageState(selected.tabId, selected.target, waitTimeoutMs, {
    description: `setContent seed url ${JSON.stringify(seedUrl)}`,
    matches: (state) => state.url === seedUrl || state.url.startsWith(`${seedUrl}#`)
  });
  return {
    url: seedUrl,
    from_url: before?.url || "",
    reason: errorMessage(cause)
  };
}

function setContentSeedUrl(tabId) {
  const nonce = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `${DAEMON_BASE_URL}${SET_CONTENT_SEED_PATH}?tab=${encodeURIComponent(String(tabId))}&nonce=${encodeURIComponent(nonce)}`;
}

function isScriptingAccessDeniedError(error) {
  const message = errorMessage(error);
  return (
    message.includes("Cannot access contents of url") ||
    message.includes("Extension manifest must request permission") ||
    message.includes("Cannot access a chrome:// URL") ||
    message.includes("Cannot access chrome://") ||
    message.includes("The extensions gallery cannot be scripted")
  );
}

async function handleAriaSnapshot(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = await tabPageState(selected.tabId, selected.target);
  const maxNodes = normalizeAriaMaxNodes(params.maxNodes);
  const maxDepth = normalizeAriaMaxDepth(params.maxDepth);
  const root = parseChromeBridgeElementId(params.rootElementId, selected.tabId, "ariaSnapshot");
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let results;
  try {
    results = await chrome.scripting.executeScript({
      target: root.frameId === null ? { tabId: selected.tabId } : { tabId: selected.tabId, frameIds: [root.frameId] },
      func: runAriaSnapshotInPage,
      args: [{ rootPath: root.path, maxNodes, maxDepth }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript ariaSnapshot(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(results);
  const selectedFrame =
    frames.find((frame) => frame.result?.ok) ||
    frames.find((frame) => frame.result) ||
    null;
  const result = selectedFrame?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.scripting.executeScript ariaSnapshot returned no structured result");
  }
  if (!result.ok) {
    throw bridgeError(
      String(result.error_code || ERROR_AXTREE_FAILED),
      `ariaSnapshot failed: ${String(result.error_detail || "")}`
    );
  }
  const frameId = Number.isSafeInteger(selectedFrame.frame_id) ? selectedFrame.frame_id : 0;
  const nodes = Array.isArray(result.nodes)
    ? result.nodes.map((node) => prefixAriaSnapshotNode(node, selected.tabId, frameId))
    : [];
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: result.url || state.url || "",
    title: result.title || state.title || "",
    ready_state: result.ready_state || state.ready_state || "",
    root_element_id: root.raw || null,
    snapshot: renderPrefixedAriaSnapshot(nodes),
    nodes,
    node_count: nodes.length,
    total_ax_nodes: Number.isSafeInteger(result.total_ax_nodes) ? result.total_ax_nodes : nodes.length,
    max_nodes: maxNodes,
    max_depth: maxDepth,
    truncated_by_max_nodes: Boolean(result.truncated_by_max_nodes),
    truncated_by_depth: Boolean(result.truncated_by_depth),
    frame_tree_frame_count: 1,
    attached_frame_target_count: 0,
    blocked_frame_targets: [],
    frame_snapshot_errors: [],
    readback_backend: "chrome.scripting.executeScript(debugger-free DOM/ARIA snapshot)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_id: frameId,
    frame_document_id: selectedFrame.document_id,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleAssertPoll(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = await tabPageState(selected.tabId, selected.target);
  const locator = plainObjectOrEmpty(params.locator);
  const limit = normalizeAssertLimit(params.limit);
  const root = parseChromeBridgeElementId(locator.root_element_id, selected.tabId, "assertPoll");
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let results;
  try {
    results = await chrome.scripting.executeScript({
      target: root.frameId === null ? { tabId: selected.tabId, allFrames: true } : { tabId: selected.tabId, frameIds: [root.frameId] },
      func: runAssertPollInPage,
      args: [{ locator, limit, rootPath: root.path }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript assertPoll(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(results);
  const resultFrames = frames.filter((frame) => frame.result && typeof frame.result === "object");
  if (resultFrames.length === 0) {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript assertPoll returned no structured result");
  }
  const failingFrame = resultFrames.find((frame) => frame.result && frame.result.ok === false);
  if (failingFrame) {
    throw bridgeError(
      String(failingFrame.result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `assertPoll failed: ${String(failingFrame.result.error_detail || "")}`
    );
  }
  const matchCount = resultFrames.reduce((sum, frame) => {
    const value = frame.result?.match_count;
    return sum + (Number.isSafeInteger(value) && value > 0 ? value : 0);
  }, 0);
  const elementFrame = resultFrames.find((frame) => frame.result?.element_state) || null;
  const elementState = elementFrame?.result?.element_state || null;
  const elementId = elementFrame && elementFrame.result?.local_element_id
    ? chromeBridgeElementId(selected.tabId, Number.isSafeInteger(elementFrame.frame_id) ? elementFrame.frame_id : 0, String(elementFrame.result.local_element_id))
    : null;
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: elementFrame?.result?.url || state.url || "",
    title: elementFrame?.result?.title || state.title || "",
    ready_state: elementFrame?.result?.ready_state || state.ready_state || "",
    match_count: matchCount,
    element_id: elementId,
    state: elementState,
    readback_backend: "chrome.scripting.executeScript(debugger-free DOM assertion poll)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleLocateElements(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = await tabPageState(selected.tabId, selected.target);
  const locator = plainObjectOrEmpty(params.locator);
  const limit = normalizeAssertLimit(params.limit);
  const root = parseChromeBridgeElementId(locator.root_element_id, selected.tabId, "locateElements");
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let results;
  try {
    results = await chrome.scripting.executeScript({
      target: root.frameId === null ? { tabId: selected.tabId, allFrames: true } : { tabId: selected.tabId, frameIds: [root.frameId] },
      func: runAssertPollInPage,
      args: [{ locator, limit, rootPath: root.path }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript locateElements(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(results);
  const resultFrames = frames.filter((frame) => frame.result && typeof frame.result === "object");
  if (resultFrames.length === 0) {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript locateElements returned no structured result");
  }
  const failingFrame = resultFrames.find((frame) => frame.result && frame.result.ok === false);
  if (failingFrame) {
    throw bridgeError(
      String(failingFrame.result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `locateElements failed: ${String(failingFrame.result.error_detail || "")}`
    );
  }
  const matchCount = resultFrames.reduce((sum, frame) => {
    const value = frame.result?.match_count;
    return sum + (Number.isSafeInteger(value) && value > 0 ? value : 0);
  }, 0);
  if (locator.strict === true && !Number.isSafeInteger(locator.nth) && matchCount > 1) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `strict mode: ${String(locator.engine || "css")} selector ${JSON.stringify(locator.query || "")} resolved to ${matchCount} elements; refine the query, set nth, or disable strict`
    );
  }
  const elementIds = [];
  for (const frame of resultFrames) {
    if (!Number.isSafeInteger(frame.frame_id)) {
      continue;
    }
    const localIds = Array.isArray(frame.result?.local_element_ids)
      ? frame.result.local_element_ids
      : (frame.result?.local_element_id ? [frame.result.local_element_id] : []);
    for (const localId of localIds) {
      if (elementIds.length >= limit) {
        break;
      }
      elementIds.push(chromeBridgeElementId(selected.tabId, frame.frame_id, String(localId)));
    }
    if (elementIds.length >= limit) {
      break;
    }
  }
  const nthSet = Number.isSafeInteger(locator.nth);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: state.url || "",
    title: state.title || "",
    ready_state: state.ready_state || "",
    engine: String(locator.engine || "css").toLowerCase(),
    query: String(locator.query || ""),
    match_count: matchCount,
    returned_count: elementIds.length,
    truncated: !nthSet && matchCount > elementIds.length,
    element_ids: elementIds,
    readback_backend: "chrome.scripting.executeScript(debugger-free DOM locator)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleInspectElement(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const parsed = parseChromeBridgeElementId(params.elementId, selected.tabId, "inspectElement");
  if (parsed.frameId === null || !parsed.path) {
    throw bridgeError(ERROR_ATTACH_FAILED, "inspectElement requires a normal bridge element id with frame and path");
  }
  const maxHtmlBytes = normalizeInspectMaxHtmlBytes(params.maxHtmlBytes);
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, frameIds: [parsed.frameId] },
      func: runBridgeElementCommandInPage,
      args: [{ operation: "inspect", elementPath: parsed.path, maxHtmlBytes }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript inspectElement(${selected.tabId}, frame=${parsed.frameId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(injected);
  const frame = frames.find((item) => item.result) || null;
  const result = frame?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript inspectElement returned no structured result");
  }
  if (!result.ok) {
    throw bridgeError(
      String(result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `inspectElement failed: ${String(result.error_detail || "")}`
    );
  }
  const state = await tabPageState(selected.tabId, selected.target);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    element_id: params.elementId,
    url: result.url || state.url || "",
    title: result.title || state.title || "",
    ready_state: result.ready_state || state.ready_state || "",
    element: result.element,
    readback_backend: "chrome.scripting.executeScript(debugger-free DOM inspect + elementFromPoint)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_id: parsed.frameId,
    frame_document_id: frame?.document_id,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleScrollIntoView(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const parsed = parseChromeBridgeElementId(params.elementId, selected.tabId, "scrollIntoView");
  if (parsed.frameId === null || !parsed.path) {
    throw bridgeError(ERROR_ATTACH_FAILED, "scrollIntoView requires a normal bridge element id with frame and path");
  }
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, frameIds: [parsed.frameId] },
      func: runBridgeElementCommandInPage,
      args: [{ operation: "scroll", elementPath: parsed.path }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript scrollIntoView(${selected.tabId}, frame=${parsed.frameId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(injected);
  const frame = frames.find((item) => item.result) || null;
  const result = frame?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript scrollIntoView returned no structured result");
  }
  if (!result.ok) {
    throw bridgeError(
      String(result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `scrollIntoView failed: ${String(result.error_detail || "")}`
    );
  }
  const state = await tabPageState(selected.tabId, selected.target);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    element_id: params.elementId,
    url: result.url || state.url || "",
    title: result.title || state.title || "",
    ready_state: result.ready_state || state.ready_state || "",
    scroll: {
      target_id: state.target_id || selected.target.id,
      element_id: params.elementId,
      ...result.scroll
    },
    readback_backend: "chrome.scripting.executeScript(debugger-free Element.scrollIntoView + geometry readback)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_id: parsed.frameId,
    frame_document_id: frame?.document_id,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleWaitForText(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = normalizeWaitForTextState(params.state);
  const expectedText = String(params.text ?? "");
  const timeoutMs = normalizeWaitTimeout(params.timeoutMs);
  const pollingIntervalMs = normalizePollingInterval(params.pollingIntervalMs);
  if (state !== "timeout" && expectedText.length === 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "waitForText text must be non-empty unless state=timeout");
  }
  if (state === "timeout" && expectedText.length > 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "waitForText state=timeout does not accept text");
  }
  const startedAt = Date.now();
  let pollCount = 0;
  let lastTextLen = 0;
  let lastPageText = null;
  while (true) {
    if (state === "timeout") {
      await sleep(timeoutMs);
      lastPageText = await tabPageTextState(selected.tabId);
      lastTextLen = Number.isSafeInteger(lastPageText?.text_len) ? lastPageText.text_len : 0;
      break;
    }
    pollCount += 1;
    lastPageText = await tabPageTextState(selected.tabId);
    if (!lastPageText.available) {
      throw bridgeError(
        String(lastPageText.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
        `waitForText page text read failed: ${String(lastPageText.error_detail || "")}`
      );
    }
    const pageText = String(lastPageText.text || "");
    lastTextLen = Number.isSafeInteger(lastPageText.text_len) ? lastPageText.text_len : pageText.length;
    const contains = pageText.includes(expectedText);
    const conditionMet = state === "text_gone" ? !contains : contains;
    if (conditionMet) {
      break;
    }
    const elapsed = elapsedSince(startedAt);
    if (elapsed >= timeoutMs) {
      const stateNow = await tabPageState(selected.tabId, selected.target);
      return waitForTextResult(selected, stateNow, lastPageText, state, expectedText, false, true, elapsed, timeoutMs, pollingIntervalMs, pollCount, lastTextLen);
    }
    await sleep(Math.min(pollingIntervalMs, Math.max(1, timeoutMs - elapsed)));
  }
  const pageState = await tabPageState(selected.tabId, selected.target);
  return waitForTextResult(
    selected,
    pageState,
    lastPageText,
    state,
    expectedText,
    true,
    false,
    elapsedSince(startedAt),
    timeoutMs,
    pollingIntervalMs,
    pollCount,
    lastTextLen
  );
}

async function handleWaitForFunction(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const expression = String(params.expression ?? "");
  if (expression.trim().length === 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "waitForFunction expression must be non-empty");
  }
  const args = Array.isArray(params.args) ? params.args : [];
  const timeoutMs = normalizeWaitTimeout(params.timeoutMs);
  const pollingIntervalMs = normalizePollingInterval(params.pollingIntervalMs);
  const startedAt = Date.now();
  let pollCount = 0;
  let last = null;
  while (true) {
    pollCount += 1;
    const poll = await waitForFunctionProbe(selected, expression, args);
    last = poll;
    if (poll.condition_met) {
      const pageState = await tabPageState(selected.tabId, selected.target);
      return waitForFunctionResult(
        selected,
        pageState,
        poll,
        true,
        false,
        elapsedSince(startedAt),
        timeoutMs,
        pollingIntervalMs,
        pollCount,
        expression,
        args
      );
    }
    const elapsed = elapsedSince(startedAt);
    if (elapsed >= timeoutMs) {
      const pageState = await tabPageState(selected.tabId, selected.target);
      return waitForFunctionResult(
        selected,
        pageState,
        last,
        false,
        true,
        elapsed,
        timeoutMs,
        pollingIntervalMs,
        pollCount,
        expression,
        args
      );
    }
    await sleep(Math.min(pollingIntervalMs, Math.max(1, timeoutMs - elapsed)));
  }
}

async function handleWaitForSelector(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const state = normalizeWaitForSelectorState(params.state);
  const timeoutMs = normalizeWaitTimeout(params.timeoutMs);
  const pollingIntervalMs = normalizePollingInterval(params.pollingIntervalMs);
  const locator = plainObjectOrEmpty(params.locator);
  const limit = normalizeAssertLimit(params.limit);
  const root = parseChromeBridgeElementId(locator.root_element_id, selected.tabId, "waitForSelector");
  const startedAt = Date.now();
  let pollCount = 0;
  let last = null;
  while (true) {
    pollCount += 1;
    const poll = await waitForSelectorPoll(selected, locator, limit, root, state);
    last = poll;
    if (poll.condition_met) {
      const pageState = await tabPageState(selected.tabId, selected.target);
      return waitForSelectorResult(selected, pageState, locator, state, true, false, elapsedSince(startedAt), timeoutMs, pollingIntervalMs, pollCount, poll);
    }
    const elapsed = elapsedSince(startedAt);
    if (elapsed >= timeoutMs) {
      const pageState = await tabPageState(selected.tabId, selected.target);
      return waitForSelectorResult(selected, pageState, locator, state, false, true, elapsed, timeoutMs, pollingIntervalMs, pollCount, last);
    }
    await sleep(Math.min(pollingIntervalMs, Math.max(1, timeoutMs - elapsed)));
  }
}

async function handleClock(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const operation = normalizeClockOperation(params.operation);
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  const state = await tabPageState(selected.tabId, selected.target);
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId },
      world: "MAIN",
      func: runClockInPage,
      args: [{
        operation,
        timeMs: params.timeMs ?? null,
        deltaMs: params.deltaMs ?? null,
        loopLimit: 10000
      }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript clock(${selected.tabId}, ${operation}) failed: ${errorMessage(error)}`
    );
  }
  const frameResults = frameExecutionResults(injected);
  const first = frameResults.find((frame) => frame.result) || null;
  const result = first?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.scripting.executeScript clock returned no structured result");
  }
  if (!result.ok) {
    throw bridgeError(
      String(result.error_code || ERROR_AXTREE_FAILED),
      `clock failed: ${String(result.error_detail || "")}`
    );
  }
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    operation,
    init_script_identifier: result.init_script_identifier || "chrome.scripting.executeScript:MAIN:synapse-clock",
    init_script_newly_added: Boolean(result.init_script_newly_added),
    installed_at_unix_ms: Number.isSafeInteger(result.installed_at_unix_ms) ? result.installed_at_unix_ms : 0,
    readback: result.readback || {},
    url: result.url || state.url || "",
    title: result.title || state.title || "",
    ready_state: result.ready_state || state.ready_state || "",
    readback_backend: "chrome.scripting.executeScript(MAIN synapse clock shim)",
    backend_tier_used: "chrome_tabs_extension",
    frame_id: Number.isSafeInteger(first.frame_id) ? first.frame_id : null,
    frame_document_id: first.document_id,
    frame_result_count: frameResults.length,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handlePageEvents(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const filters = normalizePageEventsFilter(params);
  const state = await tabPageState(selected.tabId, selected.target);
  const bufferResult = ensurePageEventBuffer(selected.tabId, state);
  const workerProbe = await drainPageWorkerEvents(selected.tabId, state);
  const read = readPageEventBuffer(bufferResult.buffer, filters);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: state.url || "",
    title: state.title || "",
    ready_state: state.ready_state || "",
    capture_newly_armed: bufferResult.newlyArmed,
    armed_at_unix_ms: bufferResult.buffer.armedAtUnixMs,
    capacity: bufferResult.buffer.capacity,
    next_cursor: read.next_cursor,
    returned: read.returned,
    total_buffered: read.total_buffered,
    dropped: read.dropped,
    filters: {
      since_seq: filters.sinceSeq,
      limit: filters.limit,
      event_kind: filters.eventKind,
      worker_type: filters.workerType
    },
    entries: read.entries,
    pages: read.pages,
    workers: read.workers,
    readback_backend: "chrome.webNavigation+chrome.tabs+chrome.scripting.executeScript(MAIN synapse page-events shim)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    web_navigation_available: Boolean(chrome.webNavigation),
    worker_probe_available: workerProbe.available,
    worker_probe_error_code: workerProbe.error_code || null,
    worker_probe_error_detail: workerProbe.error_detail || null,
    worker_probe_frame_result_count: workerProbe.frame_result_count || 0,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function drainPageWorkerEvents(tabId, state) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    return {
      available: false,
      error_code: "CHROME_SCRIPTING_UNAVAILABLE",
      error_detail: "Chrome scripting API is unavailable; extension is missing scripting permission",
      frame_result_count: 0
    };
  }
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      world: "MAIN",
      func: runPageEventsWorkerProbe
    });
  } catch (error) {
    return {
      available: false,
      error_code: ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      error_detail: errorMessage(error),
      frame_result_count: 0
    };
  }
  const frames = frameExecutionResults(injected);
  for (const frame of frames) {
    const result = frame.result;
    if (!result || typeof result !== "object" || !Array.isArray(result.events)) {
      continue;
    }
    for (const event of result.events) {
      pushPageEvent(tabId, {
        ...event,
        target_id: state.target_id || targetIdForTabId(tabId),
        frame_id: event.frame_id == null && Number.isSafeInteger(frame.frame_id)
          ? String(frame.frame_id)
          : event.frame_id,
        url: event.url || result.url || state.url || "",
        title: event.title || result.title || state.title || ""
      });
    }
  }
  return {
    available: true,
    error_code: null,
    error_detail: null,
    frame_result_count: frames.length
  };
}

async function handleCapturePageScreenshot(params) {
  return rejectAttachCommand("capturePageScreenshot", params);
}

async function handleTypeActiveElement(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const text = String(params.text ?? "");
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let results;
  try {
    results = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, allFrames: true },
      func: typeActiveElementInPage,
      args: [text]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript typeActiveElement(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frameResults = frameExecutionResults(results);
  const okFrames = frameResults.filter((frame) => frame.result && frame.result.ok);
  const first = okFrames[0] || frameResults.find((frame) => frame.result) || null;
  const result = first?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript typeActiveElement returned no structured result"
    );
  }
  if (!result.ok) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `typeActiveElement failed: code=${String(result.error_code || "UNKNOWN")} detail=${String(result.error_detail || "")}`
    );
  }
  return {
    extension_id: chrome.runtime.id,
    target_id: selected.target.id,
    tab_id: selected.tabId,
    chars_typed: [...text].length,
    readback_backend: "chrome.scripting.executeScript",
    frame_id: first.frame_id,
    frame_document_id: first.document_id,
    frame_result_count: frameResults.length,
    frame_results: frameResults.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...result
  };
}

async function createDomActionPopupTabs(selected, beforeState, actionResult) {
  const intents = [];
  const seen = new Set();
  const pushIntent = (intent, fallbackKind) => {
    if (!intent || typeof intent !== "object") {
      return;
    }
    const kind = String(intent.kind || fallbackKind || "").trim();
    const requestedUrl = String(intent.resolved_url || intent.url || "").trim();
    if (!kind || !requestedUrl || intent.opened === true) {
      return;
    }
    const key = `${kind}\n${requestedUrl}\n${String(intent.target || "")}`;
    if (seen.has(key)) {
      return;
    }
    seen.add(key);
    intents.push({ ...intent, kind, requested_url: requestedUrl });
  };

  const actionReadback = actionResult?.action_readback || {};
  const popupIntents = Array.isArray(actionResult?.popup_intents)
    ? actionResult.popup_intents
    : Array.isArray(actionReadback.popup_intents)
      ? actionReadback.popup_intents
      : [];
  for (const intent of popupIntents) {
    pushIntent(intent, "window_open");
  }
  pushIntent(
    actionResult?.anchor_target_blank_intent || actionReadback.anchor_target_blank_intent,
    "anchor_target_blank"
  );

  const created = [];
  for (const intent of intents) {
    const createParams = {
      url: normalizeOpenUrl(intent.requested_url) || "about:blank",
      active: false,
      openerTabId: selected.tabId
    };
    if (Number.isInteger(beforeState?.chrome_window_id)) {
      createParams.windowId = beforeState.chrome_window_id;
    }
    let tab;
    try {
      tab = await chrome.tabs.create(createParams);
    } catch (error) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `domAction popup intent create failed kind=${intent.kind} url=${JSON.stringify(createParams.url)} opener_tab_id=${selected.tabId}: ${errorMessage(error)}`
      );
    }
    if (!tab || typeof tab.id !== "number") {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `domAction popup intent create returned no numeric tab id kind=${intent.kind} opener_tab_id=${selected.tabId}`
      );
    }
    const target = await waitForTabTarget(tab.id, 10000);
    const state = await tabPageState(tab.id, target);
    pushPageEvent(selected.tabId, {
      event_kind: "page_created",
      target_id: selected.target.id,
      target_type: "page",
      target_attached: false,
      page_target_id: target.id,
      opener_id: selected.target.id,
      can_access_opener: true,
      url: state.url || target.url || tab.url || createParams.url,
      title: state.title || target.title || tab.title || "",
      observed_at_unix_ms: Date.now()
    });
    created.push({
      kind: intent.kind,
      opener_tab_id: selected.tabId,
      tab_id: tab.id,
      target_id: target.id,
      chrome_window_id: state.chrome_window_id,
      url: state.url || target.url || tab.url || createParams.url,
      title: state.title || target.title || tab.title || "",
      target_attached: Boolean(target.attached),
      active: Boolean(state.active),
      highlighted: Boolean(state.highlighted),
      source_opened_by_browser: Boolean(intent.opened),
      source_was_blocked: intent.opened === false
    });
  }
  return created;
}

async function handleNavigateTab(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const action = normalizeNavigateAction(params.action);
  const requestedUrl = action === "navigate" ? requiredUrl(params.url) : null;
  const agentSessionId = normalizeOptionalSessionId(params.agentSessionId);
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  const ignoreCache = Boolean(params.ignoreCache);
  const before = await tabPageState(selected.tabId, selected.target);
  let readbackExpectation = null;
  try {
    if (action === "navigate") {
      markAgentNavigation(selected.tabId, {
        action,
        requestedUrl,
        sessionId: agentSessionId
      });
      await chrome.tabs.update(selected.tabId, { url: requestedUrl });
      if (requestedUrl !== before.url) {
        readbackExpectation = {
          description: `tab url to become ${JSON.stringify(requestedUrl)} or differ from ${JSON.stringify(before.url)}`,
          matches: (state) => state.url === requestedUrl || state.url !== before.url
        };
      }
    } else if (action === "reload") {
      markAgentNavigation(selected.tabId, {
        action,
        requestedUrl: before.url,
        sessionId: agentSessionId
      });
      await chrome.tabs.reload(selected.tabId, { bypassCache: ignoreCache });
    } else if (action === "back") {
      markAgentNavigation(selected.tabId, {
        action,
        requestedUrl: null,
        sessionId: agentSessionId
      });
      await chrome.tabs.goBack(selected.tabId);
      readbackExpectation = {
        description: `tab url to change after chrome.tabs.goBack from ${JSON.stringify(before.url)}`,
        matches: (state) => state.url !== before.url
      };
    } else if (action === "forward") {
      markAgentNavigation(selected.tabId, {
        action,
        requestedUrl: null,
        sessionId: agentSessionId
      });
      await chrome.tabs.goForward(selected.tabId);
      readbackExpectation = {
        description: `tab url to change after chrome.tabs.goForward from ${JSON.stringify(before.url)}`,
        matches: (state) => state.url !== before.url
      };
    }
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.tabs.${tabsNavigationMethod(action)}(${selected.tabId}) failed: ${errorMessage(error)}; ` +
        `before url=${JSON.stringify(before.url)} title=${JSON.stringify(before.title)} ` +
        `status=${JSON.stringify(before.ready_state)}`
    );
  }
  const after = await waitForTabPageState(selected.tabId, selected.target, waitTimeoutMs, readbackExpectation);
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    action,
    requested_url: requestedUrl,
    before_url: before.url,
    before_title: before.title,
    after_url: after.url,
    after_title: after.title,
    ready_state: after.ready_state,
    history_current_index: -1,
    history_entry_count: 0,
    history_readback_source: "not_available_chrome_tabs",
    readback_backend: "chrome.tabs.get",
    navigation_error_text: null,
    is_download: null,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

// Make a specific tab the active tab WITHIN ITS OWN WINDOW without taking the
// OS foreground. chrome.tabs.update({active:true}) selects the tab in its
// window; per the Chrome API it "does not affect whether the window is focused"
// (windows.update handles that, which we deliberately never call). This is the
// background-safe Playwright bringToFront analogue (#1189) that lets agents
// activate a tab for render-dependent capture without seizing the human's
// foreground — eliminating the global SendKeys workaround (#1204).
async function handleActivateTab(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  const before = await tabPageState(selected.tabId, selected.target);
  try {
    await chrome.tabs.update(selected.tabId, { active: true });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.tabs.update(${selected.tabId}, {active:true}) failed: ${errorMessage(error)}; ` +
        `before active=${before.active} url=${JSON.stringify(before.url)}`
    );
  }
  const after = await waitForTabPageState(selected.tabId, selected.target, waitTimeoutMs, {
    description: "tab to become the active tab in its window (active=true)",
    matches: (state) => state.active === true
  });
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: after.chrome_window_id,
    before_active: before.active,
    active: after.active,
    highlighted: after.highlighted,
    url: after.url,
    title: after.title,
    ready_state: after.ready_state,
    readback_backend: "chrome.tabs.get",
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleDomAction(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const action = normalizeDomAction(params.action);
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  const autoWait = Boolean(params.autoWait);
  const autoWaitTimeoutMs = normalizeDomActionAutoWaitTimeout(params.autoWaitTimeoutMs, autoWait);
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }

  const before = await tabPageState(selected.tabId, selected.target);
  const beforePageText = await tabPageTextState(selected.tabId);
  const request = {
    action,
    selector: stringOrNull(params.selector),
    elementId: stringOrNull(params.elementId),
    role: stringOrNull(params.role),
    name: stringOrNull(params.name),
    value: stringOrNull(params.value),
    option: stringOrNull(params.option),
    optionLabel: stringOrNull(params.optionLabel),
    optionIndex: params.optionIndex === null || params.optionIndex === undefined ? null : normalizeInteger(params.optionIndex, "optionIndex"),
    options: action === "select" ? normalizeSelectOptions(params.options) : [],
    eventType: action === "dispatch_event" ? normalizeEventType(params.eventType) : null,
    eventInit: action === "dispatch_event" ? normalizeEventInit(params.eventInit) : null,
    clicks: normalizeClickCount(params.clicks, action),
    button: normalizeMouseButton(params.button, "domAction"),
    modifiers: normalizeClickModifiers(params.modifiers, "domAction"),
    position: normalizeClickPosition(params, "domAction"),
    autoWait,
    autoWaitTimeoutMs,
    maxPageTextChars: MAX_PAGE_TEXT_CHARS
  };
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, allFrames: true },
      func: performDomActionInPage,
      args: [{ ...request, resolveOnly: true }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript domAction(${selected.tabId}, ${action}) resolve failed: ${errorMessage(error)}`
    );
  }

  const frameResults = frameExecutionResults(injected);
  const okFrames = frameResults.filter((frame) => frame.result && frame.result.ok);
  const totalMatches = frameResults.reduce((sum, frame) => sum + frameResolvedMatchCount(frame), 0);
  const ambiguousFrames = frameResults.filter((frame) =>
    frame.result?.error_code === ERROR_CHROME_DOM_ELEMENT_AMBIGUOUS
  );
  if (okFrames.length > 1 || totalMatches > 1 || ambiguousFrames.length > 0) {
    throw bridgeError(
      ERROR_CHROME_DOM_ELEMENT_AMBIGUOUS,
      `domAction ${action} matched ${totalMatches} actionable element(s) across ${frameResults.length} frame(s); ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }
  const first = okFrames[0] || null;
  if (!first || !first.result || typeof first.result !== "object") {
    const failed = frameResults.find((frame) => frame.result)?.result;
    throw bridgeError(
      String(failed?.error_code || ERROR_CHROME_DOM_ELEMENT_NOT_FOUND),
      `domAction ${action} failed across ${frameResults.length} frame(s): ${String(failed?.error_detail || "no matching frame result")}; ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }
  if (!Number.isSafeInteger(first.frame_id)) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `domAction ${action} resolved a frame without a targetable frame_id; ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }

  let actionInjected;
  try {
    actionInjected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, frameIds: [first.frame_id] },
      world: "MAIN",
      func: performDomActionInPage,
      args: [request]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript domAction(${selected.tabId}, ${action}, frame=${first.frame_id}) failed: ${errorMessage(error)}`
    );
  }
  const actionFrameResults = frameExecutionResults(actionInjected);
  const actionFrame = actionFrameResults.find((frame) => frame.result) || null;
  const actionResult = actionFrame?.result;
  if (!actionResult || typeof actionResult !== "object") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript domAction returned no structured action result"
    );
  }
  if (!actionResult.ok) {
    throw bridgeError(
      String(actionResult.error_code || ERROR_AXTREE_FAILED),
      `domAction ${action} failed in resolved frame ${first.frame_id}: ${String(actionResult.error_detail || "")}; ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }

  const createdPopupTabs = await createDomActionPopupTabs(selected, before, actionResult);
  const after = await waitForTabPageState(selected.tabId, selected.target, waitTimeoutMs);
  const afterPageText = await tabPageTextState(selected.tabId);
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: after.chrome_window_id,
    action,
    before_page: before,
    after_page: after,
    before_page_text: beforePageText,
    after_page_text: afterPageText,
    created_popup_tabs: createdPopupTabs,
    readback_backend: "chrome.scripting.executeScript+chrome.tabs.get",
    required_foreground: false,
    frame_id: first.frame_id,
    frame_document_id: first.document_id,
    frame_result_count: frameResults.length,
    frame_results: frameResults.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...actionResult
  };
}

function handleReloadSelf(params = {}) {
  const expectedExtensionId = String(params.expectedExtensionId || EXPECTED_EXTENSION_ID).trim();
  if (expectedExtensionId !== chrome.runtime.id) {
    throw bridgeError(
      ERROR_EXTENSION_ID_MISMATCH,
      `reloadSelf refused extension_id mismatch: actual=${chrome.runtime.id} expected=${expectedExtensionId}`
    );
  }
  const expectedBuildId = String(params.expectedBuildId || BRIDGE_BUILD_ID).trim();
  if (expectedBuildId && expectedBuildId !== BRIDGE_BUILD_ID) {
    throw bridgeError(
      ERROR_EXTENSION_STALE,
      `reloadSelf refused stale loaded build: loaded_build_id=${BRIDGE_BUILD_ID} expected_build_id=${expectedBuildId}`
    );
  }
  const delayMs = normalizeReloadDelay(params.reloadDelayMs);
  return {
    ok: true,
    ...bridgeIdentity(),
    host_id: hostId,
    reload_requested_at_unix_ms: Date.now(),
    reload_delay_ms: delayMs
  };
}

function scheduleRuntimeReload(delayMs) {
  const boundedDelayMs = normalizeReloadDelay(delayMs);
  postDaemonMessage({
    type: "event",
    event: "extensionReloadScheduled",
    ...bridgeIdentity(),
    reload_delay_ms: boundedDelayMs,
    scheduled_at_unix_ms: Date.now()
  }).catch((error) => {
    console.warn(`Synapse reload schedule event dropped: ${errorMessage(error)}`);
  });
  setTimeout(() => {
    try {
      chrome.runtime.reload();
    } catch (error) {
      console.error(`Synapse runtime.reload failed: ${errorMessage(error)}`);
    }
  }, boundedDelayMs);
}

async function selectTabTarget(params, options = {}) {
  const tabs = await tabTargets();
  if (tabs.length === 0) {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.tabs.query returned no tab targets");
  }
  const targetIdHint = String(params.targetIdHint || "").trim();
  let expectedWindowId = optionalInteger(params.expectedChromeWindowId);
  const expectedWindow = await expectedChromeWindowForHwndHint(params);
  if (Number.isInteger(expectedWindow?.windowId)) {
    if (Number.isInteger(expectedWindowId) && expectedWindowId !== expectedWindow.windowId) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `expectedChromeWindowId ${expectedWindowId} disagrees with hwnd ${String(params.hwnd)} mapping ${expectedWindow.windowId}`
      );
    }
    expectedWindowId = expectedWindow.windowId;
  }
  const tabIdHint = tabIdFromTargetId(targetIdHint);
  if (Number.isInteger(tabIdHint)) {
    const selectedById = tabs.find((target) => target.tabId === tabIdHint);
    if (selectedById) {
      if (
        Number.isInteger(expectedWindowId) &&
        Number.isInteger(selectedById.chromeWindowId) &&
        selectedById.chromeWindowId !== expectedWindowId
      ) {
        throw bridgeError(
          ERROR_AXTREE_FAILED,
          `targetIdHint ${targetIdHint} is in Chrome window ${selectedById.chromeWindowId}, expected ${expectedWindowId}`
        );
      }
      return selectedPage(selectedById, tabs.length, "chrome_tab_id_hint");
    }
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `targetIdHint ${targetIdHint} did not match any chrome.tabs tab id`
    );
  }
  if (options.requireTargetId) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      targetIdHint
        ? `targetIdHint ${targetIdHint} is not a tabs bridge target id; expected ${TAB_TARGET_PREFIX}<tabId>`
        : `targetIdHint is required for mutating tab navigation; expected ${TAB_TARGET_PREFIX}<tabId>`
    );
  }
  const urlHint = String(params.foregroundUrlHint || "").trim();
  const titleHint = String(params.foregroundTitle || "").trim();
  if (urlHint) {
    const matches = tabs.filter((target) => urlMatchesHint(target.url || "", urlHint));
    if (matches.length === 1) {
      return selectedPage(matches[0], tabs.length, "url_hint");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `url hint matched ${matches.length} tab targets`);
    }
  }
  if (titleHint) {
    const matches = tabs.filter((target) => {
      const title = target.title || "";
      return title && (titleHint.includes(title) || title.includes(titleHint));
    });
    if (matches.length === 1) {
      return selectedPage(matches[0], tabs.length, "foreground_title");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `title hint matched ${matches.length} tab targets`);
    }
  }
  return selectedPage(tabs[0], tabs.length, "fallback_first_tab");
}

async function handleCoordinateClick(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }

  const before = await tabPageState(selected.tabId, selected.target);
  const beforePageText = await tabPageTextState(selected.tabId);
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId },
      func: performCoordinateClickInPage,
      args: [{
        x: normalizeCoordinateValue(params.x, "x"),
        y: normalizeCoordinateValue(params.y, "y"),
        coordinateSpace: normalizeCoordinateSpace(params.coordinateSpace),
        clicks: normalizeClickCount(params.clicks),
        button: normalizeMouseButton(params.button, "coordinateClick"),
        modifiers: normalizeClickModifiers(params.modifiers, "coordinateClick"),
        maxPageTextChars: MAX_PAGE_TEXT_CHARS
      }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript coordinateClick(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }

  const first = Array.isArray(injected) ? injected[0] : null;
  const clickResult = first?.result;
  if (!clickResult || typeof clickResult !== "object") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript coordinateClick returned no structured result"
    );
  }
  if (!clickResult.ok) {
    throw bridgeError(
      String(clickResult.error_code || ERROR_AXTREE_FAILED),
      `coordinateClick failed: ${String(clickResult.error_detail || "")}`
    );
  }

  const after = await waitForTabPageState(selected.tabId, selected.target, waitTimeoutMs);
  const afterPageText = await tabPageTextState(selected.tabId);
  const activeElement = await tabActiveElementState(selected.tabId);
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: after.chrome_window_id,
    before_page: before,
    after_page: after,
    before_page_text: beforePageText,
    after_page_text: afterPageText,
    active_element: activeElement,
    readback_backend: "chrome.scripting.executeScript+chrome.tabs.get",
    required_foreground: false,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...clickResult
  };
}

async function selectOpenWindowForHwndHint(params) {
  if (params.hwnd === undefined || params.hwnd === null) {
    return null;
  }
  let windows;
  try {
    windows = await chrome.windows.getAll({ windowTypes: ["normal"] });
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.windows.getAll(normal): ${errorMessage(error)}`);
  }
  const candidates = windows
    .filter((windowInfo) => Number.isInteger(windowInfo.id))
    .map((windowInfo) => ({
      id: windowInfo.id,
      focused: Boolean(windowInfo.focused),
      state: String(windowInfo.state || ""),
      type: String(windowInfo.type || ""),
      bounds: chromeWindowBounds(windowInfo),
      activeTabTitle: "",
      activeTabUrl: ""
    }));
  await Promise.all(candidates.map(async (candidate) => {
    const activeTab = await activeTabForWindow(candidate.id);
    candidate.activeTabTitle = String(activeTab?.title || "");
    candidate.activeTabUrl = String(activeTab?.url || "");
  }));
  const expectedBounds = normalizeExpectedWindowBounds(params.expectedWindowBounds);
  if (expectedBounds) {
    const boundsMatches = candidates
      .map((windowInfo) => ({
        windowInfo,
        score: windowBoundsDeltaScore(windowInfo.bounds, expectedBounds)
      }))
      .filter((candidate) => Number.isFinite(candidate.score) && candidate.score <= OPEN_WINDOW_BOUNDS_TOLERANCE_PX)
      .sort((left, right) => left.score - right.score);
    if (boundsMatches.length === 1 || (boundsMatches.length > 1 && boundsMatches[0].score < boundsMatches[1].score)) {
      const selected = boundsMatches[0].windowInfo;
      return {
        windowId: selected.id,
        focused: selected.focused,
        state: selected.state,
        selectionReason: "passive_window_bounds_match_for_hwnd_hint",
        candidateCount: candidates.length,
        nonFocusedCount: candidates.filter((windowInfo) => !windowInfo.focused).length
      };
    }
    if (boundsMatches.length > 1) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `Chrome window mapping refused hwnd hint ${String(params.hwnd)}: expected_window_bounds matched ${boundsMatches.length} Chrome windows within ${OPEN_WINDOW_BOUNDS_TOLERANCE_PX}px`
      );
    }
  }
  const expectedTitle = normalizeExpectedWindowTitle(params.expectedWindowTitle);
  if (expectedTitle) {
    const titleMatches = candidates.filter((windowInfo) =>
      chromeWindowTitleMatches(windowInfo.activeTabTitle, expectedTitle)
    );
    if (titleMatches.length === 1) {
      const selected = titleMatches[0];
      return {
        windowId: selected.id,
        focused: selected.focused,
        state: selected.state,
        selectionReason: "passive_active_tab_title_match_for_hwnd_hint",
        candidateCount: candidates.length,
        nonFocusedCount: candidates.filter((windowInfo) => !windowInfo.focused).length
      };
    }
    if (titleMatches.length > 1) {
      throw bridgeError(
        ERROR_AXTREE_FAILED,
        `Chrome window mapping refused hwnd hint ${String(params.hwnd)}: expected_window_title matched ${titleMatches.length} Chrome windows`
      );
    }
  }
  if (expectedBounds || expectedTitle) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `Chrome window mapping refused hwnd hint ${String(params.hwnd)}: expected_window_bounds/title did not identify a Chrome window; ` +
        `candidate_count=${candidates.length} candidates=[${chromeWindowCandidateSummary(candidates, expectedBounds)}]`
    );
  }
  const nonFocused = candidates.filter((windowInfo) => !windowInfo.focused);
  const nonFocusedNonMinimized = nonFocused.filter((windowInfo) => windowInfo.state !== "minimized");
  if (nonFocusedNonMinimized.length === 1) {
    const selected = nonFocusedNonMinimized[0];
    return {
      windowId: selected.id,
      focused: selected.focused,
      state: selected.state,
      selectionReason: "single_non_focused_non_minimized_chrome_window_for_hwnd_hint",
      candidateCount: candidates.length,
      nonFocusedCount: nonFocused.length
    };
  }
  if (nonFocused.length === 1) {
    return {
      windowId: nonFocused[0].id,
      focused: nonFocused[0].focused,
      state: nonFocused[0].state,
      selectionReason: "single_non_focused_chrome_window_for_hwnd_hint",
      candidateCount: candidates.length,
      nonFocusedCount: nonFocused.length
    };
  }
  if (nonFocused.length === 0 && candidates.length === 1 && candidates[0].focused) {
    return {
      windowId: candidates[0].id,
      focused: true,
      state: candidates[0].state,
      selectionReason: "focused_chrome_window_inactive_tab_for_hwnd_hint",
      candidateCount: candidates.length,
      nonFocusedCount: nonFocused.length
    };
  }
  const candidateSummary = candidates
    .map((windowInfo) => chromeWindowCandidateSummaryItem(windowInfo, expectedBounds))
    .join(",");
  throw bridgeError(
    ERROR_AXTREE_FAILED,
    `Chrome window mapping refused hwnd hint ${String(params.hwnd)}: candidate_count=${candidates.length} non_focused_count=${nonFocused.length} non_focused_non_minimized_count=${nonFocusedNonMinimized.length} candidates=[${candidateSummary}]; OS HWND cannot be mapped inside the normal extension bridge`
  );
}

async function expectedChromeWindowForHwndHint(params) {
  if (params.hwnd === undefined || params.hwnd === null) {
    return null;
  }
  if (!normalizeExpectedWindowBounds(params.expectedWindowBounds) && !normalizeExpectedWindowTitle(params.expectedWindowTitle)) {
    return null;
  }
  return selectOpenWindowForHwndHint(params);
}

async function activeTabForWindow(windowId) {
  if (!Number.isInteger(windowId)) {
    return null;
  }
  try {
    const tabs = await chrome.tabs.query({ windowId, active: true });
    return tabs[0] || null;
  } catch (_) {
    return null;
  }
}

function normalizeExpectedWindowBounds(raw) {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const x = Number(raw.x);
  const y = Number(raw.y);
  const w = Number(raw.w);
  const h = Number(raw.h);
  if (![x, y, w, h].every(Number.isFinite) || w <= 0 || h <= 0) {
    return null;
  }
  return { x, y, w, h };
}

function normalizeExpectedWindowTitle(raw) {
  if (raw === undefined || raw === null) {
    return "";
  }
  return String(raw).trim();
}

function titleCompareVariants(raw) {
  const base = String(raw || "")
    .toLowerCase()
    .replace(/\s+/g, " ")
    .trim();
  if (!base) {
    return [];
  }
  const withoutChromeSuffix = base.replace(/\s+-\s+google chrome$/, "").trim();
  const withoutAttentionPrefix = withoutChromeSuffix.replace(/^\(\d+\)\s*/, "").trim();
  return Array.from(new Set([
    base,
    withoutChromeSuffix,
    withoutAttentionPrefix
  ].filter((value) => value.length >= 3)));
}

function chromeWindowTitleMatches(activeTabTitle, expectedWindowTitle) {
  const actualVariants = titleCompareVariants(activeTabTitle);
  const expectedVariants = titleCompareVariants(expectedWindowTitle);
  for (const actual of actualVariants) {
    for (const expected of expectedVariants) {
      if (actual === expected || actual.includes(expected) || expected.includes(actual)) {
        return true;
      }
    }
  }
  return false;
}

function chromeWindowCandidateSummary(candidates, expectedBounds) {
  return candidates
    .map((windowInfo) => chromeWindowCandidateSummaryItem(windowInfo, expectedBounds))
    .join(",");
}

function chromeWindowCandidateSummaryItem(windowInfo, expectedBounds) {
  const score = expectedBounds
    ? windowBoundsDeltaScore(windowInfo.bounds, expectedBounds)
    : Number.POSITIVE_INFINITY;
  const scoreText = Number.isFinite(score) ? String(score) : "na";
  const title = String(windowInfo.activeTabTitle || "").replace(/\s+/g, " ").trim().slice(0, 80);
  return `id=${windowInfo.id}/focused=${windowInfo.focused}/state=${windowInfo.state || "unknown"}/score=${scoreText}/title=${JSON.stringify(title)}`;
}

function chromeWindowBounds(windowInfo) {
  const left = Number(windowInfo.left);
  const top = Number(windowInfo.top);
  const width = Number(windowInfo.width);
  const height = Number(windowInfo.height);
  if (![left, top, width, height].every(Number.isFinite) || width <= 0 || height <= 0) {
    return null;
  }
  return { x: left, y: top, w: width, h: height };
}

function windowBoundsDeltaScore(actual, expected) {
  if (!actual || !expected) {
    return Number.POSITIVE_INFINITY;
  }
  let best = Number.POSITIVE_INFINITY;
  for (const scale of WINDOW_BOUNDS_SCALE_CANDIDATES) {
    const score = Math.max(
      Math.abs((actual.x * scale) - expected.x),
      Math.abs((actual.y * scale) - expected.y),
      Math.abs((actual.w * scale) - expected.w),
      Math.abs((actual.h * scale) - expected.h)
    );
    if (score < best) {
      best = score;
    }
  }
  return best;
}

async function chromeWindowState(windowId) {
  if (!Number.isInteger(windowId)) {
    return null;
  }
  try {
    const windowInfo = await chrome.windows.get(windowId);
    return {
      id: windowInfo.id,
      focused: Boolean(windowInfo.focused),
      state: String(windowInfo.state || ""),
      type: String(windowInfo.type || "")
    };
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.windows.get(${windowId}) failed: ${errorMessage(error)}`
    );
  }
}

async function tabTargets() {
  let tabs;
  try {
    tabs = await chrome.tabs.query({});
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.tabs.query: ${errorMessage(error)}`);
  }
  return tabs
    .filter((tab) => typeof tab.id === "number")
    .map((tab) => tabTargetFromTab(tab));
}

function selectedPage(target, targetCandidateCount, selectionReason) {
  return {
    target,
    tabId: target.tabId,
    url: target.url || "",
    title: target.title || "",
    targetCandidateCount,
    selectionReason
  };
}

function tabTargetFromTab(tab) {
  return {
    id: targetIdForTabId(tab.id),
    tabId: tab.id,
    chromeWindowId: typeof tab.windowId === "number" ? tab.windowId : null,
    type: "page",
    url: String(tab.pendingUrl || tab.url || ""),
    title: String(tab.title || ""),
    status: String(tab.status || ""),
    active: Boolean(tab.active),
    highlighted: Boolean(tab.highlighted),
    pinned: Boolean(tab.pinned),
    attached: false
  };
}

function tabPageStateFromTab(tab) {
  const target = tabTargetFromTab(tab);
  return {
    target_id: target.id,
    tab_id: target.tabId,
    chrome_window_id: target.chromeWindowId,
    target_type: target.type || "page",
    url: target.url || "",
    title: target.title || "",
    ready_state: target.status || "",
    active: Boolean(target.active),
    highlighted: Boolean(target.highlighted),
    pinned: Boolean(target.pinned),
    history_current_index: -1,
    history_entry_count: 0
  };
}

function tabListEntryFromTabTarget(target, tab) {
  return {
    target_id: target.id,
    tab_id: target.tabId,
    chrome_window_id: target.chromeWindowId,
    index: Number.isInteger(tab.index) ? tab.index : -1,
    target_type: target.type || "page",
    url: target.url || "",
    title: target.title || "",
    ready_state: target.status || "",
    active: Boolean(target.active),
    highlighted: Boolean(target.highlighted),
    pinned: Boolean(target.pinned),
    target_attached: Boolean(target.attached)
  };
}

function targetIdForTabId(tabId) {
  return `${TAB_TARGET_PREFIX}${tabId}`;
}

function tabIdFromTargetId(targetId) {
  if (!targetId.startsWith(TAB_TARGET_PREFIX)) {
    return null;
  }
  const value = Number(targetId.slice(TAB_TARGET_PREFIX.length));
  if (!Number.isSafeInteger(value) || value < 0) {
    return null;
  }
  return value;
}

function urlMatchesHint(url, hint) {
  return url === hint || url.startsWith(hint) || hint.startsWith(url);
}

async function waitForTabTarget(tabId, waitTimeoutMs) {
  const started = Date.now();
  let lastCount = 0;
  while (Date.now() - started <= waitTimeoutMs) {
    const pages = await tabTargets();
    lastCount = pages.length;
    const target = pages.find((candidate) => candidate.tabId === tabId);
    if (target?.id) {
      return target;
    }
    await sleep(100);
  }
  throw bridgeError(
    ERROR_EXTENSION_TIMEOUT,
    `chrome.tabs.query did not expose a tab target for new tab ${tabId} within ${waitTimeoutMs} ms; lastTabTargetCount=${lastCount}`
  );
}

async function waitForTargetAbsent(targetId, waitTimeoutMs) {
  const started = Date.now();
  let pages = [];
  while (Date.now() - started <= waitTimeoutMs) {
    pages = await tabTargets();
    if (!pages.some((candidate) => candidate.id === targetId)) {
      return pages;
    }
    await sleep(100);
  }
  throw bridgeError(
    ERROR_EXTENSION_TIMEOUT,
    `chrome.tabs.query still contains closed target ${JSON.stringify(targetId)} after ${waitTimeoutMs} ms; lastTabTargetCount=${pages.length}`
  );
}

async function waitForTabPageState(tabId, fallbackTarget, waitTimeoutMs, expectation = null) {
  const started = Date.now();
  let last = null;
  let lastError = null;
  while (Date.now() - started <= waitTimeoutMs) {
    try {
      last = await tabPageState(tabId, fallbackTarget);
      lastError = null;
      const loaded = last.ready_state === "complete";
      if (loaded && (!expectation || expectation.matches(last))) {
        return last;
      }
    } catch (error) {
      lastError = error?.message ? String(error.message) : String(error);
    }
    await sleep(100);
  }
  const detail = last
    ? `waiting for ${expectation?.description || "complete tab state"}; ` +
      `last url=${JSON.stringify(last.url)} title=${JSON.stringify(last.title)} ` +
      `status=${JSON.stringify(last.ready_state)} targetId=${JSON.stringify(last.target_id)}`
    : lastError
      ? `last readback error=${JSON.stringify(lastError)}`
    : "no tab state readback";
  throw bridgeError(ERROR_EXTENSION_TIMEOUT, `tab readback did not settle within ${waitTimeoutMs} ms; ${detail}`);
}

async function tabPageState(tabId, fallbackTarget = null) {
  let tab;
  try {
    tab = await chrome.tabs.get(tabId);
  } catch (error) {
    throw bridgeError(ERROR_AXTREE_FAILED, `chrome.tabs.get(${tabId}): ${errorMessage(error)}`);
  }
  const target = tabTargetFromTab(tab);
  return {
    target_id: String(target?.id || fallbackTarget?.id || ""),
    tab_id: tabId,
    chrome_window_id: typeof tab.windowId === "number" ? tab.windowId : target?.chromeWindowId || null,
    target_type: String(target?.type || fallbackTarget?.type || "page"),
    url: String(tab.pendingUrl || tab.url || target?.url || fallbackTarget?.url || ""),
    title: String(tab.title || target?.title || fallbackTarget?.title || ""),
    ready_state: String(tab.status || ""),
    active: Boolean(tab.active),
    highlighted: Boolean(tab.highlighted),
    pinned: Boolean(tab.pinned),
    history_current_index: -1,
    history_entry_count: 0
  };
}

async function tabActiveElementState(tabId) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      error_code: "CHROME_SCRIPTING_UNAVAILABLE",
      error_detail: "Chrome scripting API is unavailable; extension is missing scripting permission"
    };
  }
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: readActiveElementInPage
    });
    const frames = frameExecutionResults(results);
    const selected =
      frames.find((frame) => frame.result?.has_active_element && frame.result?.is_editable) ||
      frames.find((frame) => frame.result?.has_active_element) ||
      frames.find((frame) => frame.result) ||
      null;
    if (!selected || typeof selected.result !== "object" || selected.result === null) {
      return {
        available: false,
        readback_source: "chrome.scripting.executeScript",
        error_code: "CHROME_SCRIPTING_EMPTY_RESULT",
        error_detail: "chrome.scripting.executeScript returned no active-element result"
      };
    }
    return {
      available: true,
      readback_source: "chrome.scripting.executeScript",
      frame_id: selected.frame_id,
      frame_document_id: selected.document_id,
      frame_result_count: frames.length,
      frame_results: frames.map(summarizeFrameExecutionResult),
      ...selected.result
    };
  } catch (error) {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: errorMessage(error)
    };
  }
}

async function tabPageTextState(tabId) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      text: null,
      text_len: 0,
      text_truncated: false,
      max_chars: MAX_PAGE_TEXT_CHARS,
      error_code: "CHROME_SCRIPTING_UNAVAILABLE",
      error_detail: "Chrome scripting API is unavailable; extension is missing scripting permission"
    };
  }
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: readPageTextInPage,
      args: [MAX_PAGE_TEXT_CHARS]
    });
    const frames = frameExecutionResults(results);
    const aggregated = aggregatePageTextFrames(frames, MAX_PAGE_TEXT_CHARS);
    if (!aggregated) {
      return {
        available: false,
        readback_source: "chrome.scripting.executeScript",
        text: null,
        text_len: 0,
        text_truncated: false,
        max_chars: MAX_PAGE_TEXT_CHARS,
        error_code: "CHROME_SCRIPTING_EMPTY_RESULT",
        error_detail: "chrome.scripting.executeScript returned no page-text result"
      };
    }
    return {
      available: true,
      readback_source: "chrome.scripting.executeScript",
      ...aggregated
    };
  } catch (error) {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      text: null,
      text_len: 0,
      text_truncated: false,
      max_chars: MAX_PAGE_TEXT_CHARS,
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: errorMessage(error)
    };
  }
}

async function tabPageContentState(tabId, maxBytes) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      html: null,
      html_len: 0,
      truncated: false,
      max_bytes: maxBytes,
      error_code: "CHROME_SCRIPTING_UNAVAILABLE",
      error_detail: "Chrome scripting API is unavailable; extension is missing scripting permission"
    };
  }
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId },
      func: readPageContentInPage,
      args: [maxBytes]
    });
    const frames = frameExecutionResults(results);
    const selected = frames.find((frame) => frame.result) || null;
    if (!selected || typeof selected.result !== "object" || selected.result === null) {
      return {
        available: false,
        readback_source: "chrome.scripting.executeScript",
        html: null,
        html_len: 0,
        truncated: false,
        max_bytes: maxBytes,
        error_code: "CHROME_SCRIPTING_EMPTY_RESULT",
        error_detail: "chrome.scripting.executeScript returned no page-content result"
      };
    }
    return {
      available: true,
      readback_source: "chrome.scripting.executeScript",
      frame_id: selected.frame_id,
      frame_document_id: selected.document_id,
      ...selected.result
    };
  } catch (error) {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      html: null,
      html_len: 0,
      truncated: false,
      max_bytes: maxBytes,
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: errorMessage(error)
    };
  }
}

function trimForReadback(value, maxChars) {
  const limit = Number.isSafeInteger(maxChars) && maxChars >= 0 ? maxChars : 240;
  return Array.from(String(value || "").replace(/\s+/g, " ").trim()).slice(0, limit).join("");
}

function frameExecutionResults(results) {
  if (!Array.isArray(results)) {
    return [];
  }
  return results.map((entry, index) => ({
    index,
    frame_id: Number.isSafeInteger(entry?.frameId) ? entry.frameId : null,
    document_id: entry?.documentId == null ? null : String(entry.documentId),
    result: entry?.result && typeof entry.result === "object" ? entry.result : null
  }));
}

function summarizeFrameExecutionResult(frame) {
  const result = frame?.result && typeof frame.result === "object" ? frame.result : {};
  const text = typeof result.text === "string" ? result.text : null;
  return {
    index: Number.isSafeInteger(frame?.index) ? frame.index : null,
    frame_id: Number.isSafeInteger(frame?.frame_id) ? frame.frame_id : null,
    document_id: frame?.document_id || null,
    ok: result.ok == null ? null : Boolean(result.ok),
    error_code: result.error_code == null ? null : String(result.error_code),
    error_detail: result.error_detail == null ? null : trimForReadback(result.error_detail, 240),
    matched_count: Number.isSafeInteger(result.matched_count) ? result.matched_count : null,
    resolved_by: result.resolved_by == null ? null : String(result.resolved_by),
    url: result.url == null ? (result.in_page_before_url == null ? null : String(result.in_page_before_url)) : String(result.url),
    title: result.title == null ? (result.in_page_title == null ? null : String(result.in_page_title)) : String(result.title),
    ready_state: result.ready_state == null ? (result.in_page_ready_state == null ? null : String(result.in_page_ready_state)) : String(result.ready_state),
    has_active_element: result.has_active_element == null ? null : Boolean(result.has_active_element),
    is_editable: result.is_editable == null ? null : Boolean(result.is_editable),
    tag_name: result.tag_name == null ? null : String(result.tag_name),
    text_len: Number.isSafeInteger(result.text_len) ? result.text_len : null,
    text_truncated: result.text_truncated == null ? null : Boolean(result.text_truncated),
    text_sample: text == null ? null : trimForReadback(text, 240)
  };
}

function frameResolvedMatchCount(frame) {
  const result = frame?.result && typeof frame.result === "object" ? frame.result : {};
  for (const key of ["matched_count", "match_count", "editable_match_count"]) {
    const value = result[key];
    if (Number.isSafeInteger(value) && value > 0) {
      return value;
    }
  }
  return result.ok ? 1 : 0;
}

function aggregatePageTextFrames(frames, maxChars) {
  const limit = Number.isSafeInteger(maxChars) && maxChars >= 0 ? Math.min(maxChars, 65536) : 4096;
  const availableFrames = frames.filter((frame) => frame.result && typeof frame.result === "object");
  if (availableFrames.length === 0) {
    return null;
  }
  const frameTexts = availableFrames.map((frame) => String(frame.result.text || ""));
  const combined = frameTexts.filter((text) => text.length > 0).join("\n\n");
  const combinedChars = Array.from(combined);
  return {
    text: combinedChars.slice(0, limit).join(""),
    text_len: combinedChars.length,
    text_truncated: combinedChars.length > limit || availableFrames.some((frame) => Boolean(frame.result.text_truncated)),
    max_chars: limit,
    readback_scope: "all_frames",
    frame_count: frames.length,
    frame_text_available_count: availableFrames.length,
    frame_text_nonempty_count: frameTexts.filter((text) => text.length > 0).length,
    frames: availableFrames.map(summarizeFrameExecutionResult)
  };
}

function waitForTextResult(
  selected,
  pageState,
  pageText,
  state,
  expectedText,
  conditionMet,
  timedOut,
  elapsedMs,
  timeoutMs,
  pollingIntervalMs,
  pollCount,
  observedTextLen
) {
  return {
    extension_id: chrome.runtime.id,
    target_id: pageState.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: pageState.chrome_window_id,
    url: pageState.url || pageText?.url || "",
    title: pageState.title || pageText?.title || "",
    ready_state: pageText?.ready_state || pageState.ready_state || "",
    state,
    text: expectedText || null,
    condition_met: Boolean(conditionMet),
    timed_out: Boolean(timedOut),
    elapsed_ms: elapsedMs,
    timeout_ms: timeoutMs,
    polling_interval_ms: pollingIntervalMs,
    poll_count: pollCount,
    observed_text_len: observedTextLen,
    readback_backend: "chrome.scripting.executeScript(page text polling)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function waitForFunctionProbe(selected, expression, args) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId },
      world: "MAIN",
      func: runWaitForFunctionProbeInPage,
      args: [{ expression, args }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript waitForFunction(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(injected);
  const first = frames.find((frame) => frame.result && typeof frame.result === "object");
  if (!first) {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript waitForFunction returned no structured result");
  }
  if (first.result.ok === false) {
    throw bridgeError(
      String(first.result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `waitForFunction failed: ${String(first.result.error_detail || "")}`
    );
  }
  return first.result;
}

function waitForFunctionResult(
  selected,
  pageState,
  poll,
  conditionMet,
  timedOut,
  elapsedMs,
  timeoutMs,
  pollingIntervalMs,
  pollCount,
  expression,
  args
) {
  const current = poll && typeof poll === "object" ? poll : {};
  return {
    extension_id: chrome.runtime.id,
    target_id: pageState.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: pageState.chrome_window_id,
    url: current.url || pageState.url || "",
    title: current.title || pageState.title || "",
    ready_state: current.ready_state || pageState.ready_state || "",
    condition_met: Boolean(conditionMet),
    timed_out: Boolean(timedOut),
    elapsed_ms: elapsedMs,
    timeout_ms: timeoutMs,
    polling_interval_ms: pollingIntervalMs,
    poll_count: pollCount,
    expression_len: expression.length,
    arg_count: args.length,
    value: Object.prototype.hasOwnProperty.call(current, "value") ? current.value : null,
    value_type: String(current.value_type || "undefined"),
    value_description: current.value_description ?? null,
    unserializable_value: current.unserializable_value ?? null,
    readback_backend: "chrome.scripting.executeScript(MAIN waitForFunction predicate polling)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function waitForSelectorPoll(selected, locator, limit, root, state) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: root.frameId === null ? { tabId: selected.tabId, allFrames: true } : { tabId: selected.tabId, frameIds: [root.frameId] },
      func: runAssertPollInPage,
      args: [{ locator, limit, rootPath: root.path }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript waitForSelector(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const frames = frameExecutionResults(injected);
  const resultFrames = frames.filter((frame) => frame.result && typeof frame.result === "object");
  if (resultFrames.length === 0) {
    throw bridgeError(ERROR_CHROME_SCRIPTING_EXECUTE_FAILED, "chrome.scripting.executeScript waitForSelector returned no structured result");
  }
  const failingFrame = resultFrames.find((frame) => frame.result && frame.result.ok === false);
  if (failingFrame) {
    throw bridgeError(
      String(failingFrame.result.error_code || ERROR_CHROME_SCRIPTING_EXECUTE_FAILED),
      `waitForSelector failed: ${String(failingFrame.result.error_detail || "")}`
    );
  }
  const normalized = selectorPollSummary(selected, resultFrames, limit);
  const condition = waitForSelectorCondition(state, normalized);
  return {
    ...normalized,
    condition_met: condition.conditionMet,
    element_id: condition.elementId,
    frame_result_count: frames.length,
    frame_results: frames.map(summarizeFrameExecutionResult)
  };
}

function selectorPollSummary(selected, resultFrames, limit) {
  let matchCount = 0;
  let truncated = false;
  let url = "";
  let title = "";
  let readyState = "";
  const returned = [];
  for (const frame of resultFrames) {
    const result = frame.result || {};
    const frameMatchCount = Number.isSafeInteger(result.match_count) && result.match_count > 0
      ? result.match_count
      : 0;
    matchCount += frameMatchCount;
    truncated = truncated || Boolean(result.truncated);
    url = url || String(result.url || "");
    title = title || String(result.title || "");
    readyState = readyState || String(result.ready_state || "");
    const localIds = Array.isArray(result.local_element_ids)
      ? result.local_element_ids
      : (result.local_element_id ? [result.local_element_id] : []);
    const states = Array.isArray(result.element_states)
      ? result.element_states
      : (result.element_state ? [result.element_state] : []);
    for (let index = 0; index < localIds.length && returned.length < limit; index += 1) {
      if (!Number.isSafeInteger(frame.frame_id)) {
        continue;
      }
      returned.push({
        elementId: chromeBridgeElementId(selected.tabId, frame.frame_id, String(localIds[index])),
        state: states[index] && typeof states[index] === "object" ? states[index] : null
      });
    }
  }
  const visible = returned.filter((item) => item.state?.is_visible === true);
  return {
    match_count: matchCount,
    returned_count: returned.length,
    visible_count: visible.length,
    truncated,
    returned,
    visible,
    url,
    title,
    ready_state: readyState
  };
}

function waitForSelectorCondition(state, poll) {
  if (state === "attached") {
    return {
      conditionMet: poll.match_count > 0,
      elementId: poll.returned[0]?.elementId || null
    };
  }
  if (state === "visible") {
    return {
      conditionMet: poll.visible.length > 0,
      elementId: poll.visible[0]?.elementId || null
    };
  }
  if (state === "hidden") {
    const absent = poll.match_count === 0;
    const hidden = !absent && poll.visible_count === 0 && !poll.truncated;
    return {
      conditionMet: absent || hidden,
      elementId: hidden ? (poll.returned[0]?.elementId || null) : null
    };
  }
  return {
    conditionMet: poll.match_count === 0,
    elementId: null
  };
}

function waitForSelectorResult(
  selected,
  pageState,
  locator,
  state,
  conditionMet,
  timedOut,
  elapsedMs,
  timeoutMs,
  pollingIntervalMs,
  pollCount,
  poll
) {
  const current = poll || {
    match_count: 0,
    returned_count: 0,
    visible_count: 0,
    truncated: false,
    element_id: null,
    url: "",
    title: "",
    ready_state: "",
    frame_result_count: 0,
    frame_results: []
  };
  return {
    extension_id: chrome.runtime.id,
    target_id: pageState.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: pageState.chrome_window_id,
    url: current.url || pageState.url || "",
    title: current.title || pageState.title || "",
    ready_state: current.ready_state || pageState.ready_state || "",
    engine: String(locator.engine || "css").toLowerCase(),
    query: String(locator.query || ""),
    state,
    condition_met: Boolean(conditionMet),
    timed_out: Boolean(timedOut),
    elapsed_ms: elapsedMs,
    timeout_ms: timeoutMs,
    polling_interval_ms: pollingIntervalMs,
    poll_count: pollCount,
    match_count: current.match_count || 0,
    returned_count: current.returned_count || 0,
    visible_count: current.visible_count || 0,
    truncated: Boolean(current.truncated),
    element_id: current.element_id || null,
    readback_backend: "chrome.scripting.executeScript(locator polling)",
    backend_tier_used: "chrome_tabs_extension",
    required_foreground: false,
    frame_result_count: current.frame_result_count || 0,
    frame_results: current.frame_results || [],
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

function optionalInteger(value) {
  if (value === undefined || value === null || value === "") {
    return null;
  }
  const numberValue = Number(value);
  return Number.isSafeInteger(numberValue) ? numberValue : null;
}

async function withTimeout(promise, timeoutMs, description) {
  let timeoutId;
  const timeout = new Promise((_, reject) => {
    timeoutId = setTimeout(() => {
      reject(new Error(`${description} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timeoutId);
  }
}

function delay(timeoutMs) {
  return new Promise((resolve) => setTimeout(resolve, timeoutMs));
}

function elapsedSince(startedAt) {
  return Math.max(0, Date.now() - startedAt);
}

async function runWaitForFunctionProbeInPage(request) {
  const expression = String(request?.expression ?? "");
  const args = Array.isArray(request?.args) ? request.args : [];
  const serializeValue = (value) => {
    const valueType = typeof value;
    if (value === undefined) {
      return {
        value: null,
        value_type: "undefined",
        value_description: "undefined",
        unserializable_value: null
      };
    }
    if (valueType === "bigint") {
      return {
        value: String(value),
        value_type: valueType,
        value_description: `${String(value)}n`,
        unserializable_value: `${String(value)}n`
      };
    }
    if (valueType === "number" && !Number.isFinite(value)) {
      return {
        value: null,
        value_type: valueType,
        value_description: String(value),
        unserializable_value: String(value)
      };
    }
    if (valueType === "function" || valueType === "symbol") {
      return {
        value: null,
        value_type: valueType,
        value_description: String(value),
        unserializable_value: null
      };
    }
    try {
      return {
        value: JSON.parse(JSON.stringify(value)),
        value_type: valueType,
        value_description: null,
        unserializable_value: null
      };
    } catch (_error) {
      return {
        value: null,
        value_type: valueType,
        value_description: String(value),
        unserializable_value: null
      };
    }
  };
  try {
    const candidate = (0, eval)(`(${expression})`);
    const value = typeof candidate === "function" ? candidate(...args) : candidate;
    const resolved = await Promise.resolve(value);
    const serialized = serializeValue(resolved);
    return {
      ok: true,
      condition_met: Boolean(resolved),
      value: serialized.value,
      value_type: serialized.value_type,
      value_description: serialized.value_description,
      unserializable_value: serialized.unserializable_value,
      url: String(location.href || ""),
      title: document && typeof document.title === "string" ? document.title : "",
      ready_state: document && typeof document.readyState === "string" ? document.readyState : ""
    };
  } catch (error) {
    return {
      ok: false,
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: error && error.stack ? String(error.stack) : String(error && error.message ? error.message : error),
      url: String(location.href || ""),
      title: document && typeof document.title === "string" ? document.title : "",
      ready_state: document && typeof document.readyState === "string" ? document.readyState : ""
    };
  }
}

async function tabPageVitalsState(tabId) {
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      visibility_state: null,
      document_hidden: null,
      ready_state: null,
      lcp_supported: null,
      lcp_entry_count: 0,
      lcp: null,
      error_code: "CHROME_SCRIPTING_UNAVAILABLE",
      error_detail: "Chrome scripting API is unavailable; extension is missing scripting permission"
    };
  }
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId, allFrames: true },
      func: readPageVitalsInPage
    });
    const frames = frameExecutionResults(results);
    const selected =
      frames.find((frame) => frame.result?.available) ||
      frames.find((frame) => frame.result) ||
      null;
    if (!selected || typeof selected.result !== "object" || selected.result === null) {
      return {
        available: false,
        readback_source: "chrome.scripting.executeScript",
        visibility_state: null,
        document_hidden: null,
        ready_state: null,
        lcp_supported: null,
        lcp_entry_count: 0,
        lcp: null,
        error_code: "CHROME_SCRIPTING_EMPTY_RESULT",
        error_detail: "chrome.scripting.executeScript returned no page-vitals result"
      };
    }
    return {
      available: true,
      readback_source: "chrome.scripting.executeScript",
      frame_id: selected.frame_id,
      frame_document_id: selected.document_id,
      frame_result_count: frames.length,
      frame_results: frames.map(summarizeFrameExecutionResult),
      ...selected.result
    };
  } catch (error) {
    return {
      available: false,
      readback_source: "chrome.scripting.executeScript",
      visibility_state: null,
      document_hidden: null,
      ready_state: null,
      lcp_supported: null,
      lcp_entry_count: 0,
      lcp: null,
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: errorMessage(error)
    };
  }
}

function readPageTextInPage(maxChars) {
  const limit = Number.isSafeInteger(maxChars) && maxChars >= 0 ? Math.min(maxChars, 65536) : 4096;
  const source =
    document.body && typeof document.body.innerText === "string"
      ? document.body.innerText
      : document.documentElement && typeof document.documentElement.innerText === "string"
        ? document.documentElement.innerText
        : document.body && typeof document.body.textContent === "string"
          ? document.body.textContent
          : document.documentElement && typeof document.documentElement.textContent === "string"
            ? document.documentElement.textContent
            : "";
  let text = "";
  let textLen = 0;
  for (const ch of String(source || "")) {
    if (textLen < limit) {
      text += ch;
    }
    textLen += 1;
  }
  return {
    text,
    text_len: textLen,
    text_truncated: textLen > limit,
    max_chars: limit,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || "")
  };
}

function readPageContentInPage(maxBytes) {
  const max = Number.isSafeInteger(maxBytes) && maxBytes >= 0 ? Math.min(maxBytes, 2 * 1024 * 1024) : 2 * 1024 * 1024;
  const html = String(document.documentElement?.outerHTML || "");
  return {
    ok: true,
    html: Array.from(html).slice(0, max).join(""),
    html_len: html.length,
    truncated: html.length > max,
    max_bytes: max,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || ""),
    history_current_index: -1,
    history_entry_count: Number.isSafeInteger(history.length) ? history.length : 0
  };
}

function runAssertPollInPage(request) {
  const ERROR_SELECTOR_INVALID = "CHROME_DOM_SELECTOR_INVALID";
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const loc = request && typeof request.locator === "object" && !Array.isArray(request.locator)
    ? request.locator
    : {};
  const limit = Number.isSafeInteger(request?.limit) ? Math.max(1, Math.min(request.limit, 500)) : 2;
  const root = elementByPath(request?.rootPath) || document.documentElement;
  if (!root) {
    return fail(ERROR_ELEMENT_NOT_FOUND, `root element path ${JSON.stringify(request?.rootPath)} was not found`);
  }
  const resolved = resolveLocator(root, loc);
  if (!resolved.ok) {
    return resolved;
  }
  const candidates = resolved.elements;
  const returned = candidates.slice(0, limit);
  const selected = returned[0] || null;
  const elementStates = returned.map(elementState);
  return {
    ok: true,
    match_count: candidates.length,
    returned_count: returned.length,
    local_element_id: selected ? domPath(selected) : null,
    local_element_ids: returned.map(domPath),
    element_states: elementStates,
    truncated: candidates.length > returned.length && !Number.isSafeInteger(loc.nth),
    element_state: selected ? elementState(selected) : null,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || "")
  };

  function resolveLocator(rootElement, locator) {
    const engine = String(locator.engine || "css").toLowerCase();
    let candidates;
    try {
      if (engine === "css") {
        candidates = cssCandidates(rootElement, String(locator.query || ""));
      } else if (engine === "xpath") {
        candidates = xpathCandidates(rootElement, String(locator.query || ""));
      } else if (engine === "text") {
        candidates = allElements(rootElement).filter((element) =>
          matchText(elementVisibleText(element), String(locator.query || ""), locator.exact, locator.regex)
        );
      } else if (engine === "role") {
        candidates = roleCandidates(rootElement, locator);
      } else if (engine === "label") {
        candidates = allElements(rootElement).filter((element) =>
          isFormLike(element) && matchText(accessibleName(element), String(locator.query || ""), locator.exact, locator.regex)
        );
      } else if (engine === "placeholder") {
        candidates = allElements(rootElement).filter((element) =>
          matchText(element.getAttribute("placeholder") || "", String(locator.query || ""), locator.exact, locator.regex)
        );
      } else if (engine === "alttext") {
        candidates = allElements(rootElement).filter((element) =>
          matchText(element.getAttribute("alt") || "", String(locator.query || ""), locator.exact, locator.regex)
        );
      } else if (engine === "title") {
        candidates = allElements(rootElement).filter((element) =>
          matchText(element.getAttribute("title") || "", String(locator.query || ""), locator.exact, locator.regex)
        );
      } else if (engine === "testid") {
        const attr = String(locator.testid_attribute || "data-testid");
        candidates = allElements(rootElement).filter((element) =>
          matchText(element.getAttribute(attr) || "", String(locator.query || ""), locator.exact ?? true, locator.regex)
        );
      } else if (engine === "layout") {
        candidates = layoutCandidates(rootElement, locator);
      } else {
        return fail(ERROR_SELECTOR_INVALID, `unsupported locator engine ${JSON.stringify(engine)}`);
      }
    } catch (error) {
      return fail(ERROR_SELECTOR_INVALID, errorMessageLocal(error));
    }
    if (locator.has_text != null && String(locator.has_text).trim()) {
      const expected = normalizeText(String(locator.has_text));
      candidates = candidates.filter((element) => normalizeText(elementVisibleText(element)).includes(expected));
    }
    candidates = applyNth(candidates, locator.nth);
    return { ok: true, elements: candidates };
  }

  function cssCandidates(rootElement, query) {
    if (!query.trim()) {
      return [];
    }
    const found = Array.from(rootElement.querySelectorAll(query));
    if (rootElement.matches?.(query)) {
      found.unshift(rootElement);
    }
    return uniqueElements(found);
  }

  function xpathCandidates(rootElement, query) {
    if (!query.trim()) {
      return [];
    }
    const snapshot = document.evaluate(query, rootElement, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
    const out = [];
    for (let index = 0; index < snapshot.snapshotLength; index += 1) {
      const node = snapshot.snapshotItem(index);
      if (node instanceof Element) {
        out.push(node);
      }
    }
    return out;
  }

  function roleCandidates(rootElement, locator) {
    const role = normalizeText(locator.query);
    const name = locator.name == null ? null : String(locator.name);
    return allElements(rootElement).filter((element) => {
      if (!locator.include_hidden && !isElementVisible(element)) {
        return false;
      }
      if (normalizeText(inferRole(element)) !== role) {
        return false;
      }
      if (name !== null && !matchText(accessibleName(element), name, locator.name_exact, locator.name_regex)) {
        return false;
      }
      if (locator.checked !== null && locator.checked !== undefined && checkedState(element) !== Boolean(locator.checked)) {
        return false;
      }
      if (locator.disabled !== null && locator.disabled !== undefined && isElementEnabled(element) === Boolean(locator.disabled)) {
        return false;
      }
      if (locator.selected !== null && locator.selected !== undefined && boolAttr(element, "aria-selected") !== Boolean(locator.selected)) {
        return false;
      }
      if (locator.pressed !== null && locator.pressed !== undefined && boolAttr(element, "aria-pressed") !== Boolean(locator.pressed)) {
        return false;
      }
      if (locator.expanded !== null && locator.expanded !== undefined && boolAttr(element, "aria-expanded") !== Boolean(locator.expanded)) {
        return false;
      }
      if (locator.level !== null && locator.level !== undefined) {
        const level = Number(element.getAttribute("aria-level") || headingLevel(element) || NaN);
        if (level !== Number(locator.level)) {
          return false;
        }
      }
      return true;
    });
  }

  function layoutCandidates(rootElement, locator) {
    const base = cssCandidates(rootElement, String(locator.query || ""));
    const anchors = cssCandidates(rootElement, String(locator.anchor || ""));
    if (anchors.length === 0) {
      return [];
    }
    const relation = String(locator.relation || "near").toLowerCase();
    const maxDistance = Number.isFinite(Number(locator.max_distance)) ? Number(locator.max_distance) : 50;
    const anchorRects = anchors.map((anchor) => anchor.getBoundingClientRect());
    return base
      .map((element) => ({ element, score: layoutScore(element.getBoundingClientRect(), anchorRects, relation) }))
      .filter((entry) => Number.isFinite(entry.score) && (relation !== "near" || entry.score <= maxDistance))
      .sort((a, b) => a.score - b.score)
      .map((entry) => entry.element);
  }

  function layoutScore(rect, anchorRects, relation) {
    let best = Infinity;
    for (const anchor of anchorRects) {
      let ok = true;
      let dx = 0;
      let dy = 0;
      if (relation === "right-of") {
        ok = rect.left >= anchor.right;
        dx = rect.left - anchor.right;
      } else if (relation === "left-of") {
        ok = rect.right <= anchor.left;
        dx = anchor.left - rect.right;
      } else if (relation === "above") {
        ok = rect.bottom <= anchor.top;
        dy = anchor.top - rect.bottom;
      } else if (relation === "below") {
        ok = rect.top >= anchor.bottom;
        dy = rect.top - anchor.bottom;
      } else {
        dx = Math.max(anchor.left - rect.right, rect.left - anchor.right, 0);
        dy = Math.max(anchor.top - rect.bottom, rect.top - anchor.bottom, 0);
      }
      if (!ok) {
        continue;
      }
      best = Math.min(best, Math.hypot(dx, dy));
    }
    return best;
  }

  function applyNth(elements, nth) {
    if (!Number.isSafeInteger(nth)) {
      return elements;
    }
    const index = nth < 0 ? elements.length + nth : nth;
    return index >= 0 && index < elements.length ? [elements[index]] : [];
  }

  function elementState(element) {
    const attributes = {};
    for (const attr of Array.from(element.attributes || [])) {
      attributes[attr.name] = String(attr.value ?? "");
    }
    return {
      tag_name: tag(element),
      text: String(element.innerText || element.textContent || ""),
      value: "value" in element ? String(element.value ?? "") : null,
      attributes,
      is_visible: isElementVisible(element),
      is_enabled: isElementEnabled(element),
      is_checked: checkedState(element)
    };
  }

  function allElements(rootElement) {
    const list = Array.from(rootElement.querySelectorAll("*"));
    if (rootElement instanceof Element) {
      list.unshift(rootElement);
    }
    return uniqueElements(list);
  }

  function uniqueElements(elements) {
    const seen = new Set();
    const out = [];
    for (const element of elements) {
      if (!(element instanceof Element) || seen.has(element)) {
        continue;
      }
      seen.add(element);
      out.push(element);
    }
    return out;
  }

  function matchText(actual, expected, exact, regex) {
    const source = normalizeText(actual);
    const needle = String(expected || "");
    if (regex) {
      try {
        return new RegExp(needle).test(String(actual || ""));
      } catch (_) {
        return false;
      }
    }
    const normalizedNeedle = normalizeText(needle);
    return exact ? source === normalizedNeedle : source.includes(normalizedNeedle);
  }

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function elementVisibleText(element) {
    return String(element.innerText || element.textContent || "");
  }

  function isFormLike(element) {
    return ["input", "textarea", "select", "button", "output"].includes(tag(element)) || element.hasAttribute("contenteditable");
  }

  function checkedState(element) {
    if ("checked" in element) {
      return Boolean(element.checked);
    }
    const aria = String(element.getAttribute("aria-checked") || "").toLowerCase();
    if (aria === "true") {
      return true;
    }
    if (aria === "false") {
      return false;
    }
    return null;
  }

  function boolAttr(element, name) {
    const value = String(element.getAttribute(name) || "").toLowerCase();
    if (value === "true") {
      return true;
    }
    if (value === "false") {
      return false;
    }
    return null;
  }

  function headingLevel(element) {
    const lower = tag(element);
    return /^h[1-6]$/.test(lower) ? Number(lower.slice(1)) : null;
  }

  function accessibleName(element) {
    const labelledBy = String(element.getAttribute("aria-labelledby") || "").trim();
    if (labelledBy) {
      const text = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id)?.textContent || "")
        .join(" ")
        .trim();
      if (text) {
        return text;
      }
    }
    for (const attr of ["aria-label", "alt", "title", "placeholder", "value"]) {
      const value = String(element.getAttribute(attr) || "").trim();
      if (value) {
        return value;
      }
    }
    if (element.id) {
      const label = document.querySelector(`label[for="${cssEscapeLocal(element.id)}"]`);
      if (label?.textContent?.trim()) {
        return label.textContent;
      }
    }
    const wrappingLabel = element.closest?.("label");
    if (wrappingLabel?.textContent?.trim()) {
      return wrappingLabel.textContent;
    }
    return element.textContent || "";
  }

  function inferRole(element) {
    const explicit = String(element.getAttribute("role") || "").trim().toLowerCase();
    if (explicit) {
      return explicit;
    }
    const lower = tag(element);
    if (lower === "a" && element.hasAttribute("href")) return "link";
    if (lower === "button") return "button";
    if (lower === "select") return "combobox";
    if (lower === "textarea") return "textbox";
    if (lower === "form") return "form";
    if (/^h[1-6]$/.test(lower)) return "heading";
    if (lower === "main") return "main";
    if (lower === "nav") return "navigation";
    if (lower === "section") return "region";
    if (lower === "img") return "img";
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "text").toLowerCase();
      if (["button", "submit", "reset", "image"].includes(type)) return "button";
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      return "textbox";
    }
    return lower === "body" ? "document" : "generic";
  }

  function isElementEnabled(element) {
    if (Boolean(element.disabled)) {
      return false;
    }
    return String(element.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
  }

  function isElementVisible(element) {
    if (!(element instanceof Element) || !element.isConnected) {
      return false;
    }
    const style = window.getComputedStyle(element);
    if (!style || style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") {
      return false;
    }
    if (Number(style.opacity) === 0) {
      return false;
    }
    const rects = element.getClientRects();
    return rects && rects.length > 0 && Array.from(rects).some((rect) => rect.width > 0 && rect.height > 0);
  }

  function elementByPath(path) {
    if (!path) {
      return null;
    }
    const parts = String(path).split(".").map((part) => Number(part));
    let current = document.documentElement;
    if (parts[0] !== 0) {
      return null;
    }
    for (const index of parts.slice(1)) {
      if (!current || !Number.isSafeInteger(index) || index < 0) {
        return null;
      }
      current = current.children[index] || null;
    }
    return current instanceof Element ? current : null;
  }

  function domPath(element) {
    const parts = [];
    let current = element;
    while (current && current instanceof Element && current !== document.documentElement) {
      const parent = current.parentElement;
      if (!parent) {
        break;
      }
      parts.unshift(Array.prototype.indexOf.call(parent.children, current));
      current = parent;
    }
    parts.unshift(0);
    return parts.join(".");
  }

  function tag(element) {
    return String(element?.tagName || "").toLowerCase();
  }

  function cssEscapeLocal(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return String(value).replace(/["\\]/g, "\\$&");
  }

  function errorMessageLocal(error) {
    return error && error.message ? String(error.message) : String(error);
  }

  function fail(errorCode, errorDetail, extra = {}) {
    return {
      ok: false,
      error_code: errorCode,
      error_detail: errorDetail,
      ...extra
    };
  }
}

async function runBridgeElementCommandInPage(request) {
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const operation = String(request?.operation || "inspect");
  const element = elementByPath(request?.elementPath);
  if (!element) {
    return fail(ERROR_ELEMENT_NOT_FOUND, `element path ${JSON.stringify(request?.elementPath)} was not found`);
  }
  if (operation === "scroll") {
    return await scrollCommand(element);
  }
  return await inspectCommand(element);

  async function inspectCommand(target) {
    const maxHtmlBytes = Number.isSafeInteger(request?.maxHtmlBytes)
      ? Math.max(0, Math.min(request.maxHtmlBytes, 2 * 1024 * 1024))
      : 256 * 1024;
    const inspection = elementInspection(target, maxHtmlBytes);
    inspection.actionability = await actionability(target);
    return {
      ok: true,
      element: inspection,
      url: String(location.href || ""),
      title: String(document.title || ""),
      ready_state: String(document.readyState || "")
    };
  }

  async function scrollCommand(target) {
    const beforeContainer = nearestScrollContainer(target);
    const before = scrollSnapshot(target, beforeContainer);
    let scrollPerformed = false;
    if (!rectFullyInViewport(before.viewport_rect) || !rectFullyInScrollContainer(before.viewport_rect, before.scroll_container)) {
      try {
        target.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "instant" });
        scrollPerformed = true;
      } catch (error) {
        try {
          target.scrollIntoView();
          scrollPerformed = true;
        } catch (_) {
          return fail("CHROME_DOM_ELEMENT_NOT_ACTIONABLE", `scrollIntoView failed: ${errorMessage(error)}`);
        }
      }
    }
    if (scrollPerformed) {
      await twoAnimationFrames();
    }
    const afterContainer = nearestScrollContainer(target);
    const after = scrollSnapshot(target, afterContainer);
    return {
      ok: true,
      scroll: {
        before_viewport: before.viewport,
        after_viewport: after.viewport,
        before_box_model: before.box_model,
        after_box_model: after.box_model,
        before_scroll_container: before.scroll_container,
        after_scroll_container: after.scroll_container,
        window_scroll_changed:
          before.viewport.scroll_x !== after.viewport.scroll_x ||
          before.viewport.scroll_y !== after.viewport.scroll_y,
        container_scroll_changed: scrollContainerChanged(before.scroll_container, after.scroll_container),
        scroll_performed: scrollPerformed,
        node_fully_in_viewport_after: rectFullyInViewport(after.viewport_rect),
        readback_backend: "Element.scrollIntoView({block:nearest,inline:nearest}) when needed + getBoundingClientRect + scroll container readback"
      },
      url: String(location.href || ""),
      title: String(document.title || ""),
      ready_state: String(document.readyState || "")
    };
  }

  function elementInspection(target, maxHtmlBytes) {
    const max = Number.isSafeInteger(maxHtmlBytes) && maxHtmlBytes >= 0 ? maxHtmlBytes : 0;
    const outer = stringValue(target.outerHTML);
    const inner = stringValue(target.innerHTML);
    const innerText = stringValue(target.innerText);
    const textContent = stringValue(target.textContent);
    const attributes = {};
    for (const attr of Array.from(target.attributes || [])) {
      attributes[attr.name] = String(attr.value ?? "");
    }
    const rect = viewportRect(target);
    const scrollX = Number(window.scrollX || 0);
    const scrollY = Number(window.scrollY || 0);
    return {
      tag_name: stringValue(target.tagName),
      outer_html: sliceChars(outer, max),
      inner_html: sliceChars(inner, max),
      inner_text: sliceChars(innerText, max),
      text_content: sliceChars(textContent, max),
      html_truncated:
        outer.length > max ||
        inner.length > max ||
        innerText.length > max ||
        textContent.length > max,
      max_html_bytes: max,
      attributes,
      input_value: "value" in target ? stringValue(target.value) : null,
      is_visible: isVisible(target),
      is_enabled: isEnabled(target),
      is_checked: isChecked(target),
      is_editable: isEditable(target),
      bounding_box: {
        x: rect.left + scrollX,
        y: rect.top + scrollY,
        viewport_x: rect.left,
        viewport_y: rect.top,
        width: rect.width,
        height: rect.height
      },
      device_pixel_ratio: Number(window.devicePixelRatio || 1)
    };
  }

  async function actionability(target) {
    const firstBox = boxModel(target);
    await twoAnimationFrames();
    const secondBox = boxModel(target);
    const dom = domState(target);
    const visible = dom.attached && isVisible(target) && Boolean(firstBox?.content?.width > 0 && firstBox?.content?.height > 0);
    const runningAnimation = hasRunningAnimation(target);
    const stable = !runningAnimation && boxesStable(firstBox, secondBox);
    const hit = hitTest(target, firstBox);
    const failures = [];
    if (!dom.attached) {
      failures.push(failure("attached", "detached", "target element is not connected to the document"));
    }
    if (!visible) {
      failures.push(failure("visible", "not_visible", "target element has no visible rendered box"));
    }
    if (!stable) {
      failures.push(failure(
        "stable",
        runningAnimation ? "animation_running" : "box_changed",
        runningAnimation
          ? "target element has a running CSS/Web Animation"
          : "target element box changed between animation-frame samples"
      ));
    }
    if (!dom.enabled) {
      failures.push(failure("enabled", "disabled", "target element is disabled or aria-disabled"));
    }
    if (!hit.receives_events) {
      failures.push(failure(
        "receives_events",
        "obscured",
        hit.top_node_description
          ? `action point is covered by ${hit.top_node_description}`
          : "document.elementFromPoint did not resolve to the target or one of its descendants"
      ));
    }
    const actionReady = dom.attached && visible && stable && dom.enabled && hit.receives_events;
    return {
      target_id: "",
      backend_node_id: 0,
      attached: dom.attached,
      visible,
      stable,
      enabled: dom.enabled,
      editable: dom.editable,
      receives_events: hit.receives_events,
      action_ready: actionReady,
      editable_action_ready: actionReady && dom.editable,
      failure_reasons: failures,
      box_model: firstBox,
      second_box_model: secondBox,
      box_model_error: firstBox ? null : "getBoundingClientRect returned no box",
      second_box_model_error: secondBox ? null : "second getBoundingClientRect returned no box",
      dom_state: dom,
      hit_test: hit
    };
  }

  function domState(target) {
    const style = target instanceof Element ? window.getComputedStyle(target) : null;
    const rect = viewportRect(target);
    const disabled = Boolean(target.disabled);
    const ariaDisabled = String(target.getAttribute?.("aria-disabled") || "").toLowerCase() === "true";
    const readonly = Boolean(target.readOnly);
    const enabled = !disabled && !ariaDisabled;
    return {
      attached: Boolean(target && target.isConnected),
      tag_name: tag(target),
      id: stringValue(target.id),
      class_name: stringValue(target.className),
      name_attr: stringValue(target.getAttribute?.("name")),
      type_attr: stringValue(target.getAttribute?.("type")),
      display: stringValue(style?.display),
      visibility: stringValue(style?.visibility),
      pointer_events: stringValue(style?.pointerEvents),
      disabled,
      aria_disabled: ariaDisabled,
      readonly,
      enabled,
      editable: isEditable(target),
      viewport_rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height
      },
      node_description: nodeDescription(target)
    };
  }

  function boxModel(target) {
    if (!(target instanceof Element) || typeof target.getBoundingClientRect !== "function") {
      return null;
    }
    const rect = target.getBoundingClientRect();
    if (!rect) {
      return null;
    }
    const pageRect = {
      x: Number(rect.left || 0) + Number(window.scrollX || 0),
      y: Number(rect.top || 0) + Number(window.scrollY || 0),
      width: Number(rect.width || 0),
      height: Number(rect.height || 0)
    };
    return {
      content: pageRect,
      border: { ...pageRect },
      width: Math.round(pageRect.width),
      height: Math.round(pageRect.height)
    };
  }

  function hitTest(target, model) {
    if (!model || !model.content || model.content.width <= 0 || model.content.height <= 0) {
      return {
        point: null,
        receives_events: false,
        top_backend_node_id: null,
        top_frame_id: null,
        target_or_descendant_from_point: false,
        top_node_description: null,
        error: "no action point"
      };
    }
    const rect = viewportRect(target);
    const point = {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2
    };
    let top = null;
    let error = null;
    try {
      top = document.elementFromPoint(point.x, point.y);
    } catch (caught) {
      error = errorMessage(caught);
    }
    const targetOrDescendant = top instanceof Element && (top === target || target.contains(top));
    return {
      point,
      receives_events: Boolean(targetOrDescendant),
      top_backend_node_id: null,
      top_frame_id: null,
      target_or_descendant_from_point: Boolean(targetOrDescendant),
      top_node_description: top instanceof Element ? nodeDescription(top) : null,
      error
    };
  }

  function scrollSnapshot(target, scrollContainer) {
    const rect = viewportRect(target);
    return {
      viewport: {
        inner_width: Math.trunc(Number(window.innerWidth || 0)),
        inner_height: Math.trunc(Number(window.innerHeight || 0)),
        scroll_x: Number(window.scrollX || 0),
        scroll_y: Number(window.scrollY || 0),
        device_pixel_ratio: Number(window.devicePixelRatio || 1)
      },
      viewport_rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height
      },
      box_model: boxModel(target),
      scroll_container: scrollContainerSnapshot(scrollContainer)
    };
  }

  function nearestScrollContainer(target) {
    let current = target instanceof Element ? target.parentElement : null;
    while (current && current !== document.documentElement) {
      const style = window.getComputedStyle(current);
      const overflow = `${style.overflow || ""} ${style.overflowX || ""} ${style.overflowY || ""}`;
      const scrollableStyle = /(auto|scroll|overlay)/i.test(overflow);
      const scrollableGeometry =
        current.scrollHeight > current.clientHeight ||
        current.scrollWidth > current.clientWidth;
      if (scrollableStyle && scrollableGeometry) {
        return current;
      }
      current = current.parentElement;
    }
    return null;
  }

  function scrollContainerSnapshot(target) {
    if (!(target instanceof Element)) {
      return null;
    }
    const rect = viewportRect(target);
    return {
      element_id: domPath(target),
      tag_name: tag(target),
      id: stringValue(target.id),
      class_name: stringValue(target.className),
      scroll_top: Number(target.scrollTop || 0),
      scroll_left: Number(target.scrollLeft || 0),
      scroll_height: Number(target.scrollHeight || 0),
      scroll_width: Number(target.scrollWidth || 0),
      client_height: Number(target.clientHeight || 0),
      client_width: Number(target.clientWidth || 0),
      viewport_rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height
      }
    };
  }

  function scrollContainerChanged(before, after) {
    if (!before && !after) {
      return false;
    }
    if (!before || !after) {
      return true;
    }
    return before.scroll_top !== after.scroll_top || before.scroll_left !== after.scroll_left;
  }

  function rectFullyInViewport(rect) {
    if (!rect) {
      return false;
    }
    const epsilon = 0.5;
    return (
      rect.x >= -epsilon &&
      rect.y >= -epsilon &&
      rect.x + rect.width <= Number(window.innerWidth || 0) + epsilon &&
      rect.y + rect.height <= Number(window.innerHeight || 0) + epsilon
    );
  }

  function rectFullyInScrollContainer(rect, scrollContainer) {
    if (!rect || !scrollContainer || !scrollContainer.viewport_rect) {
      return true;
    }
    const container = scrollContainer.viewport_rect;
    const epsilon = 0.5;
    return (
      rect.x >= container.x - epsilon &&
      rect.y >= container.y - epsilon &&
      rect.x + rect.width <= container.x + container.width + epsilon &&
      rect.y + rect.height <= container.y + container.height + epsilon
    );
  }

  function boxesStable(a, b) {
    if (!a || !b || !a.content || !b.content) {
      return false;
    }
    const epsilon = 0.25;
    return (
      Math.abs(a.content.x - b.content.x) <= epsilon &&
      Math.abs(a.content.y - b.content.y) <= epsilon &&
      Math.abs(a.content.width - b.content.width) <= epsilon &&
      Math.abs(a.content.height - b.content.height) <= epsilon
    );
  }

  function hasRunningAnimation(target) {
    if (!(target instanceof Element) || typeof target.getAnimations !== "function") {
      return false;
    }
    try {
      return target.getAnimations({ subtree: false }).some((animation) => animation.playState === "running");
    } catch (_) {
      return false;
    }
  }

  function elementByPath(path) {
    if (!path) {
      return null;
    }
    const parts = String(path).split(".").map((part) => Number(part));
    let current = document.documentElement;
    if (parts[0] !== 0) {
      return null;
    }
    for (const index of parts.slice(1)) {
      if (!current || !Number.isSafeInteger(index) || index < 0) {
        return null;
      }
      current = current.children[index] || null;
    }
    return current instanceof Element ? current : null;
  }

  function domPath(target) {
    const parts = [];
    let current = target;
    while (current && current instanceof Element && current !== document.documentElement) {
      const parent = current.parentElement;
      if (!parent) {
        break;
      }
      parts.unshift(Array.prototype.indexOf.call(parent.children, current));
      current = parent;
    }
    parts.unshift(0);
    return parts.join(".");
  }

  function viewportRect(target) {
    if (!(target instanceof Element) || typeof target.getBoundingClientRect !== "function") {
      return { left: 0, top: 0, width: 0, height: 0 };
    }
    const rect = target.getBoundingClientRect();
    return {
      left: Number(rect.left || 0),
      top: Number(rect.top || 0),
      width: Number(rect.width || 0),
      height: Number(rect.height || 0)
    };
  }

  function isVisible(target) {
    if (!(target instanceof Element) || !target.isConnected) {
      return false;
    }
    const style = window.getComputedStyle(target);
    if (!style || style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") {
      return false;
    }
    if (Number(style.opacity) === 0) {
      return false;
    }
    const rects = target.getClientRects();
    return rects && rects.length > 0 && Array.from(rects).some((rect) => rect.width > 0 && rect.height > 0);
  }

  function isEnabled(target) {
    if (!(target instanceof Element)) {
      return false;
    }
    if (Boolean(target.disabled)) {
      return false;
    }
    return String(target.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
  }

  function isEditable(target) {
    if (!(target instanceof Element) || !isEnabled(target)) {
      return false;
    }
    const lower = tag(target);
    if (lower === "textarea") {
      return !Boolean(target.readOnly);
    }
    if (lower === "input") {
      const type = String(target.getAttribute("type") || "text").toLowerCase();
      const editableTypes = new Set(["text", "search", "url", "tel", "email", "password", "number", "date", "datetime-local", "month", "time", "week", "color"]);
      return editableTypes.has(type) && !Boolean(target.readOnly);
    }
    return Boolean(
      target.isContentEditable ||
        String(target.getAttribute("contenteditable") || "").toLowerCase() === "true" ||
        String(target.getAttribute("role") || "").toLowerCase() === "textbox"
    );
  }

  function isChecked(target) {
    if (target && "checked" in target) {
      return Boolean(target.checked);
    }
    return String(target?.getAttribute?.("aria-checked") || "").toLowerCase() === "true";
  }

  function nodeDescription(target) {
    if (!(target instanceof Element)) {
      return "";
    }
    const id = target.id ? `#${target.id}` : "";
    const cls = typeof target.className === "string" && target.className.trim()
      ? `.${target.className.trim().split(/\s+/).slice(0, 3).join(".")}`
      : "";
    const name = target.getAttribute("name") ? `[name="${target.getAttribute("name")}"]` : "";
    return `${tag(target)}${id}${cls}${name}`;
  }

  function failure(predicate, reason, detail) {
    return { predicate, reason, detail };
  }

  function twoAnimationFrames() {
    return new Promise((resolve) => {
      let done = false;
      const finish = () => {
        if (done) {
          return;
        }
        done = true;
        resolve();
      };
      setTimeout(finish, 100);
      if (typeof requestAnimationFrame === "function") {
        requestAnimationFrame(() => requestAnimationFrame(finish));
      }
    });
  }

  function sliceChars(value, max) {
    return Array.from(String(value || "")).slice(0, Math.max(0, max)).join("");
  }

  function stringValue(value) {
    return value === null || value === undefined ? "" : String(value);
  }

  function tag(target) {
    return stringValue(target?.tagName).toLowerCase();
  }

  function errorMessage(error) {
    return error && error.message ? String(error.message) : String(error);
  }

  function fail(errorCode, errorDetail, extra = {}) {
    return {
      ok: false,
      error_code: errorCode,
      error_detail: errorDetail,
      ...extra
    };
  }
}

async function inspectElementInPage(request) {
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const element = bridgeElementByPath(request?.elementPath);
  if (!element) {
    return bridgeFail(ERROR_ELEMENT_NOT_FOUND, `element path ${JSON.stringify(request?.elementPath)} was not found`);
  }
  const maxHtmlBytes = Number.isSafeInteger(request?.maxHtmlBytes)
    ? Math.max(0, Math.min(request.maxHtmlBytes, 2 * 1024 * 1024))
    : 256 * 1024;
  const inspection = bridgeElementInspection(element, maxHtmlBytes);
  inspection.actionability = await bridgeActionability(element);
  return {
    ok: true,
    element: inspection,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || "")
  };
}

async function scrollElementIntoViewInPage(request) {
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const element = bridgeElementByPath(request?.elementPath);
  if (!element) {
    return bridgeFail(ERROR_ELEMENT_NOT_FOUND, `element path ${JSON.stringify(request?.elementPath)} was not found`);
  }
  const beforeContainer = bridgeNearestScrollContainer(element);
  const before = bridgeScrollSnapshot(element, beforeContainer);
  let scrollPerformed = false;
  if (!bridgeRectFullyInViewport(before.viewport_rect) || !bridgeRectFullyInScrollContainer(before.viewport_rect, before.scroll_container)) {
    try {
      element.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "instant" });
      scrollPerformed = true;
    } catch (error) {
      try {
        element.scrollIntoView();
        scrollPerformed = true;
      } catch (_) {
        return bridgeFail("CHROME_DOM_ELEMENT_NOT_ACTIONABLE", `scrollIntoView failed: ${bridgeErrorMessage(error)}`);
      }
    }
  }
  if (scrollPerformed) {
    await bridgeTwoAnimationFrames();
  }
  const afterContainer = bridgeNearestScrollContainer(element);
  const after = bridgeScrollSnapshot(element, afterContainer);
  return {
    ok: true,
    scroll: {
      before_viewport: before.viewport,
      after_viewport: after.viewport,
      before_box_model: before.box_model,
      after_box_model: after.box_model,
      before_scroll_container: before.scroll_container,
      after_scroll_container: after.scroll_container,
      window_scroll_changed:
        before.viewport.scroll_x !== after.viewport.scroll_x ||
        before.viewport.scroll_y !== after.viewport.scroll_y,
      container_scroll_changed: bridgeScrollContainerChanged(before.scroll_container, after.scroll_container),
      scroll_performed: scrollPerformed,
      node_fully_in_viewport_after: bridgeRectFullyInViewport(after.viewport_rect),
      readback_backend: "Element.scrollIntoView({block:nearest,inline:nearest}) when needed + getBoundingClientRect + scroll container readback"
    },
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || "")
  };
}

function bridgeElementInspection(element, maxHtmlBytes) {
  const max = Number.isSafeInteger(maxHtmlBytes) && maxHtmlBytes >= 0 ? maxHtmlBytes : 0;
  const outer = bridgeString(element.outerHTML);
  const inner = bridgeString(element.innerHTML);
  const innerText = bridgeString(element.innerText);
  const textContent = bridgeString(element.textContent);
  const attributes = {};
  for (const attr of Array.from(element.attributes || [])) {
    attributes[attr.name] = String(attr.value ?? "");
  }
  const rect = bridgeViewportRect(element);
  const scrollX = Number(window.scrollX || 0);
  const scrollY = Number(window.scrollY || 0);
  return {
    tag_name: bridgeString(element.tagName),
    outer_html: bridgeSliceChars(outer, max),
    inner_html: bridgeSliceChars(inner, max),
    inner_text: bridgeSliceChars(innerText, max),
    text_content: bridgeSliceChars(textContent, max),
    html_truncated:
      outer.length > max ||
      inner.length > max ||
      innerText.length > max ||
      textContent.length > max,
    max_html_bytes: max,
    attributes,
    input_value: "value" in element ? bridgeString(element.value) : null,
    is_visible: bridgeIsVisible(element),
    is_enabled: bridgeIsEnabled(element),
    is_checked: bridgeIsChecked(element),
    is_editable: bridgeIsEditable(element),
    bounding_box: {
      x: rect.left + scrollX,
      y: rect.top + scrollY,
      viewport_x: rect.left,
      viewport_y: rect.top,
      width: rect.width,
      height: rect.height
    },
    device_pixel_ratio: Number(window.devicePixelRatio || 1)
  };
}

async function bridgeActionability(element) {
  const firstBox = bridgeBoxModel(element);
  await bridgeTwoAnimationFrames();
  const secondBox = bridgeBoxModel(element);
  const domState = bridgeDomState(element);
  const visible = domState.attached && bridgeIsVisible(element) && Boolean(firstBox?.content?.width > 0 && firstBox?.content?.height > 0);
  const runningAnimation = bridgeHasRunningAnimation(element);
  const stable = !runningAnimation && bridgeBoxesStable(firstBox, secondBox);
  const hitTest = bridgeHitTest(element, firstBox);
  const enabled = domState.enabled;
  const editable = domState.editable;
  const receivesEvents = hitTest.receives_events;
  const failureReasons = [];
  if (!domState.attached) {
    failureReasons.push(bridgeFailure("attached", "detached", "target element is not connected to the document"));
  }
  if (!visible) {
    failureReasons.push(bridgeFailure("visible", "not_visible", "target element has no visible rendered box"));
  }
  if (!stable) {
    failureReasons.push(bridgeFailure(
      "stable",
      runningAnimation ? "animation_running" : "box_changed",
      runningAnimation
        ? "target element has a running CSS/Web Animation"
        : "target element box changed between animation-frame samples"
    ));
  }
  if (!enabled) {
    failureReasons.push(bridgeFailure("enabled", "disabled", "target element is disabled or aria-disabled"));
  }
  if (!receivesEvents) {
    failureReasons.push(bridgeFailure(
      "receives_events",
      "obscured",
      hitTest.top_node_description
        ? `action point is covered by ${hitTest.top_node_description}`
        : "document.elementFromPoint did not resolve to the target or one of its descendants"
    ));
  }
  const actionReady = domState.attached && visible && stable && enabled && receivesEvents;
  return {
    target_id: "",
    backend_node_id: 0,
    attached: domState.attached,
    visible,
    stable,
    enabled,
    editable,
    receives_events: receivesEvents,
    action_ready: actionReady,
    editable_action_ready: actionReady && editable,
    failure_reasons: failureReasons,
    box_model: firstBox,
    second_box_model: secondBox,
    box_model_error: firstBox ? null : "getBoundingClientRect returned no box",
    second_box_model_error: secondBox ? null : "second getBoundingClientRect returned no box",
    dom_state: domState,
    hit_test: hitTest
  };
}

function bridgeDomState(element) {
  const style = element instanceof Element ? window.getComputedStyle(element) : null;
  const rect = bridgeViewportRect(element);
  const disabled = Boolean(element.disabled);
  const ariaDisabled = String(element.getAttribute?.("aria-disabled") || "").toLowerCase() === "true";
  const readonly = Boolean(element.readOnly);
  const enabled = !disabled && !ariaDisabled;
  return {
    attached: Boolean(element && element.isConnected),
    tag_name: bridgeTag(element),
    id: bridgeString(element.id),
    class_name: bridgeString(element.className),
    name_attr: bridgeString(element.getAttribute?.("name")),
    type_attr: bridgeString(element.getAttribute?.("type")),
    display: bridgeString(style?.display),
    visibility: bridgeString(style?.visibility),
    pointer_events: bridgeString(style?.pointerEvents),
    disabled,
    aria_disabled: ariaDisabled,
    readonly,
    enabled,
    editable: bridgeIsEditable(element),
    viewport_rect: {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height
    },
    node_description: bridgeNodeDescription(element)
  };
}

function bridgeBoxModel(element) {
  if (!(element instanceof Element) || typeof element.getBoundingClientRect !== "function") {
    return null;
  }
  const rect = element.getBoundingClientRect();
  if (!rect) {
    return null;
  }
  const scrollX = Number(window.scrollX || 0);
  const scrollY = Number(window.scrollY || 0);
  const pageRect = {
    x: Number(rect.left || 0) + scrollX,
    y: Number(rect.top || 0) + scrollY,
    width: Number(rect.width || 0),
    height: Number(rect.height || 0)
  };
  return {
    content: pageRect,
    border: { ...pageRect },
    width: Math.round(pageRect.width),
    height: Math.round(pageRect.height)
  };
}

function bridgeHitTest(element, boxModel) {
  if (!boxModel || !boxModel.content || boxModel.content.width <= 0 || boxModel.content.height <= 0) {
    return {
      point: null,
      receives_events: false,
      top_backend_node_id: null,
      top_frame_id: null,
      target_or_descendant_from_point: false,
      top_node_description: null,
      error: "no action point"
    };
  }
  const viewportRect = bridgeViewportRect(element);
  const point = {
    x: viewportRect.left + viewportRect.width / 2,
    y: viewportRect.top + viewportRect.height / 2
  };
  let top = null;
  let error = null;
  try {
    top = document.elementFromPoint(point.x, point.y);
  } catch (caught) {
    error = bridgeErrorMessage(caught);
  }
  const targetOrDescendant = top instanceof Element && (top === element || element.contains(top));
  return {
    point,
    receives_events: Boolean(targetOrDescendant),
    top_backend_node_id: null,
    top_frame_id: null,
    target_or_descendant_from_point: Boolean(targetOrDescendant),
    top_node_description: top instanceof Element ? bridgeNodeDescription(top) : null,
    error
  };
}

function bridgeScrollSnapshot(element, scrollContainer) {
  const viewportRect = bridgeViewportRect(element);
  return {
    viewport: {
      inner_width: Math.trunc(Number(window.innerWidth || 0)),
      inner_height: Math.trunc(Number(window.innerHeight || 0)),
      scroll_x: Number(window.scrollX || 0),
      scroll_y: Number(window.scrollY || 0),
      device_pixel_ratio: Number(window.devicePixelRatio || 1)
    },
    viewport_rect: {
      x: viewportRect.left,
      y: viewportRect.top,
      width: viewportRect.width,
      height: viewportRect.height
    },
    box_model: bridgeBoxModel(element),
    scroll_container: bridgeScrollContainerSnapshot(scrollContainer)
  };
}

function bridgeNearestScrollContainer(element) {
  let current = element instanceof Element ? element.parentElement : null;
  while (current && current !== document.documentElement) {
    const style = window.getComputedStyle(current);
    const overflow = `${style.overflow || ""} ${style.overflowX || ""} ${style.overflowY || ""}`;
    const scrollableStyle = /(auto|scroll|overlay)/i.test(overflow);
    const scrollableGeometry =
      current.scrollHeight > current.clientHeight ||
      current.scrollWidth > current.clientWidth;
    if (scrollableStyle && scrollableGeometry) {
      return current;
    }
    current = current.parentElement;
  }
  return null;
}

function bridgeScrollContainerSnapshot(element) {
  if (!(element instanceof Element)) {
    return null;
  }
  const rect = bridgeViewportRect(element);
  return {
    element_id: bridgeDomPath(element),
    tag_name: bridgeTag(element),
    id: bridgeString(element.id),
    class_name: bridgeString(element.className),
    scroll_top: Number(element.scrollTop || 0),
    scroll_left: Number(element.scrollLeft || 0),
    scroll_height: Number(element.scrollHeight || 0),
    scroll_width: Number(element.scrollWidth || 0),
    client_height: Number(element.clientHeight || 0),
    client_width: Number(element.clientWidth || 0),
    viewport_rect: {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height
    }
  };
}

function bridgeScrollContainerChanged(before, after) {
  if (!before && !after) {
    return false;
  }
  if (!before || !after) {
    return true;
  }
  return before.scroll_top !== after.scroll_top || before.scroll_left !== after.scroll_left;
}

function bridgeRectFullyInViewport(rect) {
  if (!rect) {
    return false;
  }
  const epsilon = 0.5;
  return (
    rect.x >= -epsilon &&
    rect.y >= -epsilon &&
    rect.x + rect.width <= Number(window.innerWidth || 0) + epsilon &&
    rect.y + rect.height <= Number(window.innerHeight || 0) + epsilon
  );
}

function bridgeRectFullyInScrollContainer(rect, scrollContainer) {
  if (!rect || !scrollContainer || !scrollContainer.viewport_rect) {
    return true;
  }
  const container = scrollContainer.viewport_rect;
  const epsilon = 0.5;
  return (
    rect.x >= container.x - epsilon &&
    rect.y >= container.y - epsilon &&
    rect.x + rect.width <= container.x + container.width + epsilon &&
    rect.y + rect.height <= container.y + container.height + epsilon
  );
}

function bridgeBoxesStable(a, b) {
  if (!a || !b || !a.content || !b.content) {
    return false;
  }
  const epsilon = 0.25;
  return (
    Math.abs(a.content.x - b.content.x) <= epsilon &&
    Math.abs(a.content.y - b.content.y) <= epsilon &&
    Math.abs(a.content.width - b.content.width) <= epsilon &&
    Math.abs(a.content.height - b.content.height) <= epsilon
  );
}

function bridgeHasRunningAnimation(element) {
  if (!(element instanceof Element) || typeof element.getAnimations !== "function") {
    return false;
  }
  try {
    return element.getAnimations({ subtree: false }).some((animation) => animation.playState === "running");
  } catch (_) {
    return false;
  }
}

function bridgeElementByPath(path) {
  if (!path) {
    return null;
  }
  const parts = String(path).split(".").map((part) => Number(part));
  let current = document.documentElement;
  if (parts[0] !== 0) {
    return null;
  }
  for (const index of parts.slice(1)) {
    if (!current || !Number.isSafeInteger(index) || index < 0) {
      return null;
    }
    current = current.children[index] || null;
  }
  return current instanceof Element ? current : null;
}

function bridgeDomPath(element) {
  const parts = [];
  let current = element;
  while (current && current instanceof Element && current !== document.documentElement) {
    const parent = current.parentElement;
    if (!parent) {
      break;
    }
    parts.unshift(Array.prototype.indexOf.call(parent.children, current));
    current = parent;
  }
  parts.unshift(0);
  return parts.join(".");
}

function bridgeViewportRect(element) {
  if (!(element instanceof Element) || typeof element.getBoundingClientRect !== "function") {
    return { left: 0, top: 0, width: 0, height: 0 };
  }
  const rect = element.getBoundingClientRect();
  return {
    left: Number(rect.left || 0),
    top: Number(rect.top || 0),
    width: Number(rect.width || 0),
    height: Number(rect.height || 0)
  };
}

function bridgeIsVisible(element) {
  if (!(element instanceof Element) || !element.isConnected) {
    return false;
  }
  const style = window.getComputedStyle(element);
  if (!style || style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") {
    return false;
  }
  if (Number(style.opacity) === 0) {
    return false;
  }
  const rects = element.getClientRects();
  return rects && rects.length > 0 && Array.from(rects).some((rect) => rect.width > 0 && rect.height > 0);
}

function bridgeIsEnabled(element) {
  if (!(element instanceof Element)) {
    return false;
  }
  if (Boolean(element.disabled)) {
    return false;
  }
  return String(element.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
}

function bridgeIsEditable(element) {
  if (!(element instanceof Element) || !bridgeIsEnabled(element)) {
    return false;
  }
  const lower = bridgeTag(element);
  if (lower === "textarea") {
    return !Boolean(element.readOnly);
  }
  if (lower === "input") {
    const type = String(element.getAttribute("type") || "text").toLowerCase();
    const editableTypes = new Set(["text", "search", "url", "tel", "email", "password", "number", "date", "datetime-local", "month", "time", "week", "color"]);
    return editableTypes.has(type) && !Boolean(element.readOnly);
  }
  return Boolean(
    element.isContentEditable ||
      String(element.getAttribute("contenteditable") || "").toLowerCase() === "true" ||
      String(element.getAttribute("role") || "").toLowerCase() === "textbox"
  );
}

function bridgeIsChecked(element) {
  if (element && "checked" in element) {
    return Boolean(element.checked);
  }
  return String(element?.getAttribute?.("aria-checked") || "").toLowerCase() === "true";
}

function bridgeNodeDescription(element) {
  if (!(element instanceof Element)) {
    return "";
  }
  const id = element.id ? `#${element.id}` : "";
  const cls = typeof element.className === "string" && element.className.trim()
    ? `.${element.className.trim().split(/\s+/).slice(0, 3).join(".")}`
    : "";
  const name = element.getAttribute("name") ? `[name="${element.getAttribute("name")}"]` : "";
  return `${bridgeTag(element)}${id}${cls}${name}`;
}

function bridgeFailure(predicate, reason, detail) {
  return { predicate, reason, detail };
}

function bridgeTwoAnimationFrames() {
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) {
        return;
      }
      done = true;
      resolve();
    };
    setTimeout(finish, 100);
    if (typeof requestAnimationFrame === "function") {
      requestAnimationFrame(() => requestAnimationFrame(finish));
    }
  });
}

function bridgeSliceChars(value, max) {
  return Array.from(String(value || "")).slice(0, Math.max(0, max)).join("");
}

function bridgeString(value) {
  return value === null || value === undefined ? "" : String(value);
}

function bridgeTag(element) {
  return bridgeString(element?.tagName).toLowerCase();
}

function bridgeErrorMessage(error) {
  return error && error.message ? String(error.message) : String(error);
}

function bridgeFail(errorCode, errorDetail, extra = {}) {
  return {
    ok: false,
    error_code: errorCode,
    error_detail: errorDetail,
    ...extra
  };
}

function runAriaSnapshotInPage(request) {
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const maxNodes = Number.isSafeInteger(request?.maxNodes) ? Math.max(1, Math.min(request.maxNodes, 5000)) : 500;
  const maxDepth = Number.isSafeInteger(request?.maxDepth) ? Math.max(1, Math.min(request.maxDepth, 128)) : 32;
  const root = elementByPath(request?.rootPath) || document.body || document.documentElement;
  if (!root) {
    return fail(ERROR_ELEMENT_NOT_FOUND, `root element path ${JSON.stringify(request?.rootPath)} was not found`);
  }
  const nodes = [];
  let truncatedByMaxNodes = false;
  let truncatedByDepth = false;
  walk(root, null, 0);
  return {
    ok: true,
    nodes,
    total_ax_nodes: nodes.length,
    truncated_by_max_nodes: truncatedByMaxNodes,
    truncated_by_depth: truncatedByDepth,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || "")
  };

  function walk(element, parentPath, depth) {
    if (!(element instanceof Element)) {
      return;
    }
    if (depth > maxDepth) {
      truncatedByDepth = true;
      return;
    }
    if (nodes.length >= maxNodes) {
      truncatedByMaxNodes = true;
      return;
    }
    const path = domPath(element);
    const children = Array.from(element.children || []);
    nodes.push({
      element_id: path,
      parent_element_id: parentPath,
      depth,
      role: inferRole(element),
      name: accessibleName(element).replace(/\s+/g, " ").trim(),
      value: elementValue(element),
      enabled: isElementEnabled(element),
      focused: document.activeElement === element,
      children_count: children.length
    });
    for (const child of children) {
      walk(child, path, depth + 1);
      if (nodes.length >= maxNodes) {
        truncatedByMaxNodes = true;
        break;
      }
    }
  }

  function accessibleName(element) {
    const labelledBy = String(element.getAttribute("aria-labelledby") || "").trim();
    if (labelledBy) {
      const text = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id)?.textContent || "")
        .join(" ")
        .trim();
      if (text) {
        return text;
      }
    }
    for (const attr of ["aria-label", "alt", "title", "placeholder", "value"]) {
      const value = String(element.getAttribute(attr) || "").trim();
      if (value) {
        return value;
      }
    }
    if (element.id) {
      const label = document.querySelector(`label[for="${cssEscapeLocal(element.id)}"]`);
      if (label?.textContent?.trim()) {
        return label.textContent;
      }
    }
    const wrappingLabel = element.closest?.("label");
    if (wrappingLabel?.textContent?.trim()) {
      return wrappingLabel.textContent;
    }
    const role = inferRole(element);
    if (["button", "link", "heading", "checkbox", "radio", "textbox"].includes(role)) {
      return element.textContent || "";
    }
    return "";
  }

  function inferRole(element) {
    const explicit = String(element.getAttribute("role") || "").trim().toLowerCase();
    if (explicit) {
      return explicit;
    }
    const lower = tag(element);
    if (lower === "html" || lower === "body") return "document";
    if (lower === "a" && element.hasAttribute("href")) return "link";
    if (lower === "button") return "button";
    if (lower === "select") return "combobox";
    if (lower === "textarea") return "textbox";
    if (lower === "form") return "form";
    if (/^h[1-6]$/.test(lower)) return "heading";
    if (lower === "main") return "main";
    if (lower === "nav") return "navigation";
    if (lower === "section") return "region";
    if (lower === "img") return "img";
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "text").toLowerCase();
      if (["button", "submit", "reset", "image"].includes(type)) return "button";
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      return "textbox";
    }
    return "generic";
  }

  function elementValue(element) {
    if ("value" in element) {
      return String(element.value ?? "");
    }
    for (const attr of ["aria-valuetext", "aria-valuenow"]) {
      const value = String(element.getAttribute(attr) || "").trim();
      if (value) {
        return value;
      }
    }
    return null;
  }

  function isElementEnabled(element) {
    if (Boolean(element.disabled)) {
      return false;
    }
    return String(element.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
  }

  function elementByPath(path) {
    if (!path) {
      return null;
    }
    const parts = String(path).split(".").map((part) => Number(part));
    let current = document.documentElement;
    if (parts[0] !== 0) {
      return null;
    }
    for (const index of parts.slice(1)) {
      if (!current || !Number.isSafeInteger(index) || index < 0) {
        return null;
      }
      current = current.children[index] || null;
    }
    return current instanceof Element ? current : null;
  }

  function domPath(element) {
    const parts = [];
    let current = element;
    while (current && current instanceof Element && current !== document.documentElement) {
      const parent = current.parentElement;
      if (!parent) {
        break;
      }
      parts.unshift(Array.prototype.indexOf.call(parent.children, current));
      current = parent;
    }
    parts.unshift(0);
    return parts.join(".");
  }

  function tag(element) {
    return String(element?.tagName || "").toLowerCase();
  }

  function cssEscapeLocal(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return String(value).replace(/["\\]/g, "\\$&");
  }

  function fail(errorCode, errorDetail, extra = {}) {
    return {
      ok: false,
      error_code: errorCode,
      error_detail: errorDetail,
      ...extra
    };
  }
}

async function setContentInPage(request) {
  const html = String((request && request.html) ?? "");
  const waitTimeoutMs = Number.isSafeInteger(request?.waitTimeoutMs)
    ? Math.max(1, Math.min(request.waitTimeoutMs, 30000))
    : 10000;
  if (html.length === 0) {
    return {
      ok: false,
      error_code: "CHROME_DOM_ACTION_UNSUPPORTED",
      error_detail: "setContent requires non-empty html"
    };
  }
  try {
    const waitForLoad = new Promise((resolve) => {
      let done = false;
      const finish = () => {
        if (done) {
          return;
        }
        done = true;
        setTimeout(resolve, 0);
      };
      window.addEventListener("load", finish, { once: true });
      setTimeout(finish, waitTimeoutMs);
    });
    document.open();
    document.write(html);
    document.close();
    if (document.readyState !== "complete") {
      await waitForLoad;
    } else {
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    const historyLength = Number.isSafeInteger(history.length) ? history.length : 0;
    return {
      ok: true,
      html_len: html.length,
      url: String(location.href || ""),
      title: String(document.title || ""),
      ready_state: String(document.readyState || ""),
      history_current_index: -1,
      history_entry_count: historyLength,
      inline_script_marker: document.body?.getAttribute("data-inline-script") || null,
      load_event_marker: document.body?.getAttribute("data-load-event") || null,
      script_proof_present: Boolean(document.getElementById("issue1159-script-proof"))
    };
  } catch (error) {
    return {
      ok: false,
      error_code: "CHROME_SCRIPTING_EXECUTE_FAILED",
      error_detail: error && error.message ? String(error.message) : String(error)
    };
  }
}

function runClockInPage(request) {
  const VERSION = "synapse-clock-2026-06-21-v1";
  const operation = String(request?.operation || "status");
  const timeMs = request?.timeMs;
  const deltaMs = request?.deltaMs;
  const loopLimit = Number.isSafeInteger(request?.loopLimit) ? request.loopLimit : 10000;

  function errorDetail(error) {
    return error && error.stack ? String(error.stack) : String(error);
  }

  try {
    const clock = ensureSynapseClock();
    let readback;
    if (operation === "install") {
      readback = clock.call("install", { nowMs: timeMs, loopLimit });
    } else if (operation === "status") {
      readback = clock.call("status", {});
    } else if (operation === "setFixedTime") {
      readback = clock.call("setFixedTime", { timeMs });
    } else if (operation === "fastForward") {
      readback = clock.call("fastForward", { deltaMs });
    } else if (operation === "pauseAt") {
      readback = clock.call("pauseAt", { timeMs });
    } else {
      throw new Error("Unknown Synapse browser clock method: " + operation);
    }
    return {
      ok: true,
      init_script_identifier: "chrome.scripting.executeScript:MAIN:synapse-clock",
      init_script_newly_added: Boolean(clock.lastInstallNewlyAdded),
      installed_at_unix_ms: Number.isSafeInteger(clock.installedAtUnixMs) ? clock.installedAtUnixMs : 0,
      readback,
      url: String(location.href || ""),
      title: String(document.title || ""),
      ready_state: String(document.readyState || "")
    };
  } catch (error) {
    return {
      ok: false,
      error_code: "CHROME_CLOCK_FAILED",
      error_detail: errorDetail(error)
    };
  }

  function ensureSynapseClock() {
    if (globalThis.__synapseClock && globalThis.__synapseClock.version === VERSION) {
      return globalThis.__synapseClock;
    }

    const NativeDate = globalThis.Date;
    const nativePerformanceNow = globalThis.performance && globalThis.performance.now
      ? globalThis.performance.now.bind(globalThis.performance)
      : () => 0;
    const state = {
      installed: false,
      nowMs: NativeDate.now(),
      timers: new Map(),
      nextTimerId: 1,
      firedTimerCount: 0,
      errorCount: 0,
      lastError: null,
      loopLimit: 10000,
      performanceOriginMs: NativeDate.now() - nativePerformanceNow(),
      installedAtUnixMs: 0
    };

    function toFiniteMs(value, fallback) {
      const n = Number(value);
      return Number.isFinite(n) ? Math.max(0, n) : fallback;
    }

    function timerDelay(value) {
      const n = Number(value);
      if (!Number.isFinite(n)) {
        return 0;
      }
      return Math.max(0, n);
    }

    function status() {
      let next = null;
      for (const timer of state.timers.values()) {
        if (next === null || timer.due < next) {
          next = timer.due;
        }
      }
      return {
        installed: state.installed,
        version: VERSION,
        now_ms: Math.floor(state.nowMs),
        pending_timer_count: state.timers.size,
        fired_timer_count: state.firedTimerCount,
        last_timer_id: state.nextTimerId - 1,
        next_timer_ms: next === null ? null : Math.floor(next),
        error_count: state.errorCount,
        last_error: state.lastError
      };
    }

    function recordError(error) {
      state.errorCount += 1;
      state.lastError = error && error.stack ? String(error.stack) : String(error);
    }

    function runHandler(timer) {
      try {
        if (typeof timer.handler === "function") {
          timer.handler.apply(globalThis, timer.args);
        } else {
          (0, eval)(String(timer.handler));
        }
      } catch (error) {
        recordError(error);
      }
    }

    function drainUntil(targetMs) {
      const target = toFiniteMs(targetMs, state.nowMs);
      let guard = 0;
      while (true) {
        let next = null;
        for (const timer of state.timers.values()) {
          if (
            timer.due <= target &&
            (next === null || timer.due < next.due || (timer.due === next.due && timer.id < next.id))
          ) {
            next = timer;
          }
        }
        if (!next) {
          break;
        }
        if (++guard > state.loopLimit) {
          throw new Error("Synapse browser clock loop limit exceeded while draining timers");
        }
        state.nowMs = next.due;
        state.timers.delete(next.id);
        state.firedTimerCount += 1;
        runHandler(next);
        if (next.kind === "interval" && !next.cancelled) {
          const step = Math.max(1, next.delay);
          next.due = state.nowMs + step;
          state.timers.set(next.id, next);
        }
      }
      state.nowMs = target;
    }

    function schedule(kind, handler, delay, args) {
      const id = state.nextTimerId++;
      const normalizedDelay = timerDelay(delay);
      state.timers.set(id, {
        id,
        kind,
        handler,
        args,
        delay: normalizedDelay,
        due: state.nowMs + normalizedDelay,
        cancelled: false
      });
      return id;
    }

    function clear(id) {
      const timer = state.timers.get(Number(id));
      if (timer) {
        timer.cancelled = true;
      }
      state.timers.delete(Number(id));
    }

    function SynapseDate(...args) {
      if (this instanceof SynapseDate) {
        if (args.length === 0) {
          return new NativeDate(state.nowMs);
        }
        return new NativeDate(...args);
      }
      return new NativeDate(state.nowMs).toString();
    }

    Object.setPrototypeOf(SynapseDate, NativeDate);
    SynapseDate.prototype = NativeDate.prototype;
    SynapseDate.now = () => Math.floor(state.nowMs);
    SynapseDate.parse = NativeDate.parse.bind(NativeDate);
    SynapseDate.UTC = NativeDate.UTC.bind(NativeDate);

    function install(args) {
      if (args && Object.prototype.hasOwnProperty.call(args, "nowMs") && args.nowMs !== null) {
        state.nowMs = toFiniteMs(args.nowMs, state.nowMs);
      }
      if (args && Object.prototype.hasOwnProperty.call(args, "loopLimit")) {
        state.loopLimit = Math.max(1, Math.floor(Number(args.loopLimit) || state.loopLimit));
      }
      api.lastInstallNewlyAdded = !state.installed;
      if (!state.installed) {
        globalThis.Date = SynapseDate;
        globalThis.setTimeout = (handler, delay, ...rest) => schedule("timeout", handler, delay, rest);
        globalThis.clearTimeout = clear;
        globalThis.setInterval = (handler, delay, ...rest) => schedule("interval", handler, delay, rest);
        globalThis.clearInterval = clear;
        globalThis.requestAnimationFrame = (handler) =>
          schedule("raf", (ts) => handler(ts), 16, [Math.max(0, state.nowMs - state.performanceOriginMs)]);
        globalThis.cancelAnimationFrame = clear;
        if (globalThis.performance) {
          try {
            Object.defineProperty(globalThis.performance, "now", {
              configurable: true,
              value: () => Math.max(0, state.nowMs - state.performanceOriginMs)
            });
          } catch (_) {}
        }
        state.installed = true;
        state.installedAtUnixMs = NativeDate.now();
        api.installedAtUnixMs = state.installedAtUnixMs;
      }
      return status();
    }

    function setFixedTime(args) {
      if (!state.installed) {
        throw new Error("Synapse browser clock is not installed");
      }
      state.nowMs = toFiniteMs(args && args.timeMs, state.nowMs);
      return status();
    }

    function fastForward(args) {
      if (!state.installed) {
        throw new Error("Synapse browser clock is not installed");
      }
      const delta = toFiniteMs(args && args.deltaMs, 0);
      drainUntil(state.nowMs + delta);
      return status();
    }

    function pauseAt(args) {
      if (!state.installed) {
        throw new Error("Synapse browser clock is not installed");
      }
      const target = toFiniteMs(args && args.timeMs, state.nowMs);
      if (target < state.nowMs) {
        throw new Error("Synapse browser clock pauseAt cannot move backwards");
      }
      drainUntil(target);
      return status();
    }

    const api = {
      version: VERSION,
      installedAtUnixMs: 0,
      lastInstallNewlyAdded: false,
      call(method, args) {
        if (method === "install") {
          return install(args || {});
        }
        if (method === "status") {
          api.lastInstallNewlyAdded = false;
          return status();
        }
        if (method === "setFixedTime") {
          api.lastInstallNewlyAdded = false;
          return setFixedTime(args || {});
        }
        if (method === "fastForward") {
          api.lastInstallNewlyAdded = false;
          return fastForward(args || {});
        }
        if (method === "pauseAt") {
          api.lastInstallNewlyAdded = false;
          return pauseAt(args || {});
        }
        throw new Error("Unknown Synapse browser clock method: " + method);
      }
    };

    globalThis.__synapseClock = api;
    return api;
  }
}

function runPageEventsWorkerProbe() {
  const VERSION = "synapse-page-events-2026-06-22-v1";

  function workerUrl(scriptURL) {
    try {
      return new URL(String(scriptURL || ""), location.href).href;
    } catch (_) {
      return String(scriptURL || "");
    }
  }

  function nowMs() {
    return Date.now();
  }

  function ensureProbe() {
    if (globalThis.__synapsePageEvents && globalThis.__synapsePageEvents.version === VERSION) {
      return globalThis.__synapsePageEvents;
    }
    const state = {
      version: VERSION,
      nextWorkerId: 1,
      events: [],
      workers: new Map(),
      installedAtUnixMs: nowMs(),
      originalWorker: globalThis.Worker,
      originalSharedWorker: globalThis.SharedWorker,
      serviceWorkerContainer: navigator.serviceWorker || null,
      originalServiceWorkerRegister: navigator.serviceWorker?.register || null
    };

    function record(event) {
      const workerId = String(event.worker_id || "");
      const entry = {
        event_kind: String(event.event_kind || ""),
        worker_id: workerId || null,
        worker_type: event.worker_type == null ? null : String(event.worker_type),
        worker_url: event.worker_url == null ? null : String(event.worker_url),
        title: String(document.title || ""),
        url: String(location.href || ""),
        observed_at_unix_ms: nowMs()
      };
      if (!entry.event_kind || !entry.worker_id) {
        return;
      }
      state.events.push(entry);
      if (entry.event_kind === "worker_destroyed") {
        const existing = state.workers.get(workerId) || {};
        state.workers.set(workerId, {
          worker_id: workerId,
          worker_type: entry.worker_type || existing.worker_type || "",
          url: entry.worker_url || existing.url || "",
          title: entry.title || existing.title || "",
          attached: false,
          destroyed: true
        });
      } else {
        state.workers.set(workerId, {
          worker_id: workerId,
          worker_type: entry.worker_type || "",
          url: entry.worker_url || "",
          title: entry.title || "",
          attached: true,
          destroyed: false
        });
      }
    }

    function nextId(prefix) {
      const id = `${prefix}-${state.nextWorkerId}`;
      state.nextWorkerId += 1;
      return id;
    }

    function installWorkerShim() {
      const NativeWorker = state.originalWorker;
      if (typeof NativeWorker !== "function" || NativeWorker.__synapsePageEventsWrapped) {
        return;
      }
      function SynapseWorker(scriptURL, options) {
        const worker = Reflect.construct(
          NativeWorker,
          options === undefined ? [scriptURL] : [scriptURL, options],
          new.target || NativeWorker
        );
        const id = nextId("worker");
        const url = workerUrl(scriptURL);
        try {
          Object.defineProperty(worker, "__synapseWorkerId", {
            configurable: false,
            enumerable: false,
            value: id
          });
        } catch (_) {}
        record({
          event_kind: "worker_created",
          worker_id: id,
          worker_type: "worker",
          worker_url: url
        });
        const nativeTerminate = worker.terminate;
        if (typeof nativeTerminate === "function") {
          worker.terminate = function terminateWithSynapseReadback(...args) {
            record({
              event_kind: "worker_destroyed",
              worker_id: id,
              worker_type: "worker",
              worker_url: url
            });
            return nativeTerminate.apply(this, args);
          };
        }
        return worker;
      }
      Object.defineProperty(SynapseWorker, "__synapsePageEventsWrapped", {
        configurable: false,
        enumerable: false,
        value: true
      });
      Object.setPrototypeOf(SynapseWorker, NativeWorker);
      SynapseWorker.prototype = NativeWorker.prototype;
      globalThis.Worker = SynapseWorker;
    }

    function installSharedWorkerShim() {
      const NativeSharedWorker = state.originalSharedWorker;
      if (typeof NativeSharedWorker !== "function" || NativeSharedWorker.__synapsePageEventsWrapped) {
        return;
      }
      function SynapseSharedWorker(scriptURL, options) {
        const worker = Reflect.construct(
          NativeSharedWorker,
          options === undefined ? [scriptURL] : [scriptURL, options],
          new.target || NativeSharedWorker
        );
        record({
          event_kind: "worker_created",
          worker_id: nextId("shared-worker"),
          worker_type: "shared_worker",
          worker_url: workerUrl(scriptURL)
        });
        return worker;
      }
      Object.defineProperty(SynapseSharedWorker, "__synapsePageEventsWrapped", {
        configurable: false,
        enumerable: false,
        value: true
      });
      Object.setPrototypeOf(SynapseSharedWorker, NativeSharedWorker);
      SynapseSharedWorker.prototype = NativeSharedWorker.prototype;
      globalThis.SharedWorker = SynapseSharedWorker;
    }

    function installServiceWorkerShim() {
      const container = state.serviceWorkerContainer;
      const nativeRegister = state.originalServiceWorkerRegister;
      if (!container || typeof nativeRegister !== "function" || nativeRegister.__synapsePageEventsWrapped) {
        return;
      }
      const wrapped = async function registerWithSynapseReadback(scriptURL, options) {
        const registration = await nativeRegister.call(this, scriptURL, options);
        const script = registration?.active?.scriptURL ||
          registration?.installing?.scriptURL ||
          registration?.waiting?.scriptURL ||
          workerUrl(scriptURL);
        record({
          event_kind: "worker_created",
          worker_id: `service-worker:${script}`,
          worker_type: "service_worker",
          worker_url: script
        });
        return registration;
      };
      Object.defineProperty(wrapped, "__synapsePageEventsWrapped", {
        configurable: false,
        enumerable: false,
        value: true
      });
      try {
        Object.defineProperty(container, "register", {
          configurable: true,
          value: wrapped
        });
      } catch (_) {
        try {
          container.register = wrapped;
        } catch (_) {}
      }
    }

    state.drain = () => {
      const drained = state.events.splice(0, state.events.length);
      return {
        events: drained,
        workers: Array.from(state.workers.values())
      };
    };

    try {
      installWorkerShim();
    } catch (error) {
      state.events.push({
        event_kind: "worker_info_changed",
        worker_id: "worker-shim-install-error",
        worker_type: "worker",
        worker_url: String(error && error.message ? error.message : error),
        title: String(document.title || ""),
        url: String(location.href || ""),
        observed_at_unix_ms: nowMs()
      });
    }
    try {
      installSharedWorkerShim();
    } catch (_) {}
    try {
      installServiceWorkerShim();
    } catch (_) {}
    globalThis.__synapsePageEvents = state;
    return state;
  }

  const probe = ensureProbe();
  const drained = probe.drain();
  return {
    ok: true,
    version: VERSION,
    installed_at_unix_ms: probe.installedAtUnixMs,
    url: String(location.href || ""),
    title: String(document.title || ""),
    ready_state: String(document.readyState || ""),
    events: drained.events,
    workers: drained.workers
  };
}

function readPageVitalsInPage() {
  function stringValue(value) {
    return value === null || value === undefined ? "" : String(value);
  }
  function numberValue(value) {
    return typeof value === "number" && Number.isFinite(value) ? value : 0;
  }
  function trimText(value, maxChars) {
    const normalized = stringValue(value).replace(/\s+/g, " ").trim();
    return Array.from(normalized).slice(0, maxChars).join("");
  }
  function cssEscape(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return stringValue(value).replace(/["\\#.:,[\]>+~*'()=]/g, "\\$&");
  }
  function elementSelector(element) {
    if (!element || element.nodeType !== 1) {
      return null;
    }
    const parts = [];
    let current = element;
    while (current && current.nodeType === 1 && parts.length < 6) {
      const local = stringValue(current.localName || current.tagName).toLowerCase();
      let part = local || "element";
      if (current.id) {
        part += "#" + cssEscape(current.id);
        parts.unshift(part);
        break;
      }
      const classes = Array.from(current.classList || []).slice(0, 2);
      for (const className of classes) {
        part += "." + cssEscape(className);
      }
      const parent = current.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children || []).filter((node) => node.localName === current.localName);
        if (siblings.length > 1) {
          part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
        }
      }
      parts.unshift(part);
      current = parent;
    }
    return parts.join(" > ");
  }
  function lcpSummary(entry) {
    const element = entry && entry.element && entry.element.nodeType === 1 ? entry.element : null;
    return {
      name: stringValue(entry && entry.name),
      entry_type: stringValue(entry && entry.entryType) || "largest-contentful-paint",
      start_time: numberValue(entry && entry.startTime),
      render_time: numberValue(entry && entry.renderTime),
      load_time: numberValue(entry && entry.loadTime),
      size: numberValue(entry && entry.size),
      element_tag_name: element ? stringValue(element.tagName).toLowerCase() : null,
      element_id: element ? stringValue(element.id) : null,
      element_class_name: element ? stringValue(element.className) : null,
      element_selector: element ? elementSelector(element) : null,
      element_text: element ? trimText(element.innerText || element.textContent || "", 2048) : null,
      element_current_src: element && "currentSrc" in element ? stringValue(element.currentSrc) : null,
      element_url: element && "url" in entry ? stringValue(entry.url) : null
    };
  }
  try {
    const supported = Boolean(
      typeof PerformanceObserver !== "undefined" &&
        Array.isArray(PerformanceObserver.supportedEntryTypes) &&
        PerformanceObserver.supportedEntryTypes.includes("largest-contentful-paint")
    );
    const entries = supported && performance && typeof performance.getEntriesByType === "function"
      ? performance.getEntriesByType("largest-contentful-paint")
      : [];
    const last = entries.length > 0 ? entries[entries.length - 1] : null;
    return {
      available: true,
      readback_source: "PerformanceObserver.supportedEntryTypes+performance.getEntriesByType",
      visibility_state: stringValue(document.visibilityState),
      document_hidden: Boolean(document.hidden),
      ready_state: stringValue(document.readyState),
      lcp_supported: supported,
      lcp_entry_count: entries.length,
      lcp: last ? lcpSummary(last) : null,
      error_code: null,
      error_detail: null
    };
  } catch (error) {
    return {
      available: false,
      readback_source: "PerformanceObserver.supportedEntryTypes+performance.getEntriesByType",
      visibility_state: typeof document !== "undefined" ? stringValue(document.visibilityState) : null,
      document_hidden: typeof document !== "undefined" ? Boolean(document.hidden) : null,
      ready_state: typeof document !== "undefined" ? stringValue(document.readyState) : null,
      lcp_supported: false,
      lcp_entry_count: 0,
      lcp: null,
      error_code: "PAGE_VITALS_READ_FAILED",
      error_detail: error && error.message ? String(error.message) : String(error)
    };
  }
}

function readActiveElementInPage() {
  const element = document.activeElement;
  if (!element) {
    return {
      has_active_element: false,
      is_editable: false,
      tag_name: "",
      id: "",
      name: "",
      value: null,
      selected_text: null
    };
  }
  const tagName = String(element.tagName || "");
  const lowerTag = tagName.toLowerCase();
  const contentEditable =
    element.isContentEditable ||
    String(element.getAttribute?.("contenteditable") || "").toLowerCase() === "true";
  const valueCapable = "value" in element;
  const isEditable =
    contentEditable ||
    lowerTag === "textarea" ||
    lowerTag === "select" ||
    (lowerTag === "input" && !["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"].includes(String(element.type || "").toLowerCase()));
  let value = null;
  if (valueCapable) {
    value = String(element.value ?? "");
  } else if (contentEditable) {
    value = String(element.innerText || element.textContent || "");
  }
  let selectedText = null;
  try {
    if (typeof element.selectionStart === "number" && typeof element.selectionEnd === "number" && value !== null) {
      selectedText = String(value).slice(element.selectionStart, element.selectionEnd);
    } else {
      const selection = document.getSelection?.();
      if (selection && selection.rangeCount > 0) {
        selectedText = String(selection.toString() || "");
      }
    }
  } catch (_) {
    selectedText = null;
  }
  return {
    has_active_element: true,
    is_editable: Boolean(isEditable),
    tag_name: tagName,
    id: String(element.id || ""),
    name: String(element.getAttribute?.("name") || ""),
    value,
    selected_text: selectedText
  };
}

function typeActiveElementInPage(text) {
  const element = document.activeElement;
  const before = readActiveElementLocal();
  if (!element || !before.has_active_element) {
    return {
      ok: false,
      error_code: "CHROME_ACTIVE_ELEMENT_MISSING",
      error_detail: "document.activeElement is absent",
      before_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...before }
    };
  }
  if (!before.is_editable) {
    return {
      ok: false,
      error_code: "CHROME_ACTIVE_ELEMENT_NOT_EDITABLE",
      error_detail: `active element ${String(before.tag_name || "")} is not editable`,
      before_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...before }
    };
  }

  const inputText = String(text ?? "");
  const eventsDispatched = [];
  const beforeInput = dispatchSyntheticInputEventLocal(element, "beforeinput", inputText, true);
  eventsDispatched.push("beforeinput");
  if (!beforeInput) {
    return {
      ok: false,
      error_code: "CHROME_BEFOREINPUT_CANCELLED",
      error_detail: "active element beforeinput listener cancelled background typing",
      before_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...before },
      events_dispatched: eventsDispatched
    };
  }

  const expectedValue = applyTextToEditableLocal(element, inputText);
  dispatchSyntheticInputEventLocal(element, "input", inputText, false);
  eventsDispatched.push("input");
  element.dispatchEvent(new Event("change", { bubbles: true }));
  eventsDispatched.push("change");

  const after = readActiveElementLocal();
  const afterValue = after.value === null || after.value === undefined ? "" : String(after.value);
  if (afterValue !== expectedValue) {
    return {
      ok: false,
      error_code: "CHROME_ACTIVE_ELEMENT_VALUE_MISMATCH",
      error_detail: `after value did not match expected value; expected_len=${expectedValue.length} after_len=${afterValue.length}`,
      before_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...before },
      after_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...after },
      expected_value: expectedValue,
      events_dispatched: eventsDispatched
    };
  }

  return {
    ok: true,
    before_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...before },
    after_active_element: { available: true, readback_source: "chrome.scripting.executeScript", ...after },
    expected_value: expectedValue,
    events_dispatched: eventsDispatched
  };

  function readActiveElementLocal() {
    const active = document.activeElement;
    if (!active) {
      return {
        has_active_element: false,
        is_editable: false,
        tag_name: "",
        id: "",
        name: "",
        value: null,
        selected_text: null
      };
    }
    const tagName = String(active.tagName || "");
    const lowerTag = tagName.toLowerCase();
    const contentEditable =
      active.isContentEditable ||
      String(active.getAttribute?.("contenteditable") || "").toLowerCase() === "true";
    const valueCapable = "value" in active;
    const isEditable =
      contentEditable ||
      lowerTag === "textarea" ||
      lowerTag === "select" ||
      (lowerTag === "input" && !["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"].includes(String(active.type || "").toLowerCase()));
    let value = null;
    if (valueCapable) {
      value = String(active.value ?? "");
    } else if (contentEditable) {
      value = String(active.innerText || active.textContent || "");
    }
    let selectedText = null;
    try {
      if (typeof active.selectionStart === "number" && typeof active.selectionEnd === "number" && value !== null) {
        selectedText = String(value).slice(active.selectionStart, active.selectionEnd);
      } else {
        const selection = document.getSelection?.();
        if (selection && selection.rangeCount > 0) {
          selectedText = String(selection.toString() || "");
        }
      }
    } catch (_) {
      selectedText = null;
    }
    return {
      has_active_element: true,
      is_editable: Boolean(isEditable),
      tag_name: tagName,
      id: String(active.id || ""),
      name: String(active.getAttribute?.("name") || ""),
      value,
      selected_text: selectedText
    };
  }

  function applyTextToEditableLocal(active, inputText) {
    const tagName = String(active.tagName || "").toLowerCase();
    const contentEditable =
      active.isContentEditable ||
      String(active.getAttribute?.("contenteditable") || "").toLowerCase() === "true";
    if ((tagName === "input" || tagName === "textarea") && "value" in active) {
      const beforeValue = String(active.value ?? "");
      const start = typeof active.selectionStart === "number" ? active.selectionStart : beforeValue.length;
      const end = typeof active.selectionEnd === "number" ? active.selectionEnd : start;
      const expected = beforeValue.slice(0, start) + inputText + beforeValue.slice(end);
      active.value = expected;
      if (typeof active.setSelectionRange === "function") {
        const caret = start + inputText.length;
        active.setSelectionRange(caret, caret);
      }
      return expected;
    }
    if (contentEditable) {
      const selection = document.getSelection?.();
      if (selection && selection.rangeCount > 0 && active.contains(selection.anchorNode)) {
        const range = selection.getRangeAt(0);
        range.deleteContents();
        const textNode = document.createTextNode(inputText);
        range.insertNode(textNode);
        range.setStartAfter(textNode);
        range.setEndAfter(textNode);
        selection.removeAllRanges();
        selection.addRange(range);
      } else {
        active.appendChild(document.createTextNode(inputText));
      }
      return String(active.innerText || active.textContent || "");
    }
    throw new Error(`active element ${tagName || "unknown"} is not text editable`);
  }

  function dispatchSyntheticInputEventLocal(active, type, inputText, cancelable) {
    let event;
    try {
      event = new InputEvent(type, {
        bubbles: true,
        cancelable,
        data: inputText,
        inputType: "insertText"
      });
    } catch (_) {
      event = new Event(type, { bubbles: true, cancelable });
    }
    return active.dispatchEvent(event);
  }
}

function applyTextToEditable(element, text) {
  const tagName = String(element.tagName || "").toLowerCase();
  const contentEditable =
    element.isContentEditable ||
    String(element.getAttribute?.("contenteditable") || "").toLowerCase() === "true";
  if ((tagName === "input" || tagName === "textarea") && "value" in element) {
    const beforeValue = String(element.value ?? "");
    const start = typeof element.selectionStart === "number" ? element.selectionStart : beforeValue.length;
    const end = typeof element.selectionEnd === "number" ? element.selectionEnd : start;
    const expected = beforeValue.slice(0, start) + text + beforeValue.slice(end);
    element.value = expected;
    if (typeof element.setSelectionRange === "function") {
      const caret = start + text.length;
      element.setSelectionRange(caret, caret);
    }
    return expected;
  }
  if (contentEditable) {
    const selection = document.getSelection?.();
    if (selection && selection.rangeCount > 0 && element.contains(selection.anchorNode)) {
      const range = selection.getRangeAt(0);
      range.deleteContents();
      const textNode = document.createTextNode(text);
      range.insertNode(textNode);
      range.setStartAfter(textNode);
      range.setEndAfter(textNode);
      selection.removeAllRanges();
      selection.addRange(range);
    } else {
      element.appendChild(document.createTextNode(text));
    }
    return String(element.innerText || element.textContent || "");
  }
  throw new Error(`active element ${tagName || "unknown"} is not text editable`);
}

function dispatchSyntheticInputEvent(element, type, text, cancelable) {
  let event;
  try {
    event = new InputEvent(type, {
      bubbles: true,
      cancelable,
      data: text,
      inputType: "insertText"
    });
  } catch (_) {
    event = new Event(type, { bubbles: true, cancelable });
  }
  return element.dispatchEvent(event);
}

// #1000/#717: background-safe field REPLACE for the user's normal Chrome. Runs
// entirely in-page via chrome.scripting (no debugger attach, no OS foreground,
// works on occluded/inactive tabs that UIA cannot see). Resolves the target by
// a strict CSS selector (exactly one editable, visible match) or the current
// active element, then replaces the value with the NATIVE prototype setter so
// React/Vue/Angular controlled inputs do not silently revert the change.
async function handleSetFieldValue(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const text = String(params.text ?? "");
  const selector = params.selector == null ? null : String(params.selector);
  const activeElement = Boolean(params.activeElement);
  if (!chrome.scripting || typeof chrome.scripting.executeScript !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript unavailable; extension is missing scripting permission"
    );
  }
  let results;
  try {
    results = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, allFrames: true },
      func: setFieldValueInPage,
      args: [{ selector, activeElement, text, resolveOnly: true }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript setFieldValue(${selected.tabId}) resolve failed: ${errorMessage(error)}`
    );
  }
  const frameResults = frameExecutionResults(results);
  const okFrames = frameResults.filter((frame) => frame.result && frame.result.ok);
  const totalMatches = frameResults.reduce((sum, frame) => sum + frameResolvedMatchCount(frame), 0);
  if (okFrames.length > 1 || totalMatches > 1) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `setFieldValue matched ${totalMatches} editable element(s) across ${frameResults.length} frame(s); ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }
  const first = okFrames[0] || null;
  if (!first || !first.result || typeof first.result !== "object") {
    const failed = frameResults.find((frame) => frame.result)?.result;
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `setFieldValue failed across ${frameResults.length} frame(s): code=${String(failed?.error_code || "UNKNOWN")} ` +
        `detail=${String(failed?.error_detail || "no matching frame result")}; frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }
  if (!Number.isSafeInteger(first.frame_id)) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `setFieldValue resolved a frame without a targetable frame_id; ` +
        `frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }

  let actionResults;
  try {
    actionResults = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId, frameIds: [first.frame_id] },
      func: setFieldValueInPage,
      args: [{ selector, activeElement, text }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript setFieldValue(${selected.tabId}, frame=${first.frame_id}) failed: ${errorMessage(error)}`
    );
  }
  const actionFrameResults = frameExecutionResults(actionResults);
  const actionFrame = actionFrameResults.find((frame) => frame.result) || null;
  const result = actionFrame?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript setFieldValue returned no structured action result"
    );
  }
  if (!result.ok) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `setFieldValue failed in resolved frame ${first.frame_id}: code=${String(result.error_code || "UNKNOWN")} ` +
        `detail=${String(result.error_detail || "")}; frame_results=${JSON.stringify(frameResults.map(summarizeFrameExecutionResult).slice(0, 8))}`
    );
  }
  return {
    extension_id: chrome.runtime.id,
    target_id: selected.target.id,
    tab_id: selected.tabId,
    chars_requested: [...text].length,
    readback_backend: "chrome.scripting.executeScript",
    frame_id: first.frame_id,
    frame_document_id: first.document_id,
    frame_result_count: frameResults.length,
    frame_results: frameResults.map(summarizeFrameExecutionResult),
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...result
  };
}

function setFieldValueInPage(request) {
  const selector = request && request.selector != null ? String(request.selector) : null;
  const wantActive = Boolean(request && request.activeElement);
  const inputText = String((request && request.text) ?? "");
  const resolveOnly = Boolean(request && request.resolveOnly);

  function isEditable(el) {
    if (!el || el.nodeType !== 1) {
      return false;
    }
    if (el.disabled || el.readOnly) {
      return false;
    }
    const tag = String(el.tagName || "").toLowerCase();
    if (tag === "textarea") {
      return true;
    }
    if (tag === "input") {
      const type = String(el.getAttribute("type") || "text").toLowerCase();
      const nonText = ["button", "submit", "reset", "checkbox", "radio", "range", "color", "file", "image", "hidden"];
      return !nonText.includes(type);
    }
    return Boolean(
      el.isContentEditable ||
        String(el.getAttribute && el.getAttribute("contenteditable") || "").toLowerCase() === "true"
    );
  }
  function isVisible(el) {
    if (!el || typeof el.getClientRects !== "function") {
      return false;
    }
    if (el.getClientRects().length > 0) {
      return true;
    }
    // Occluded windows still lay out the DOM; fall back to box model.
    return Boolean(el.offsetWidth || el.offsetHeight);
  }
  function readValue(el) {
    const tag = String(el.tagName || "").toLowerCase();
    if ((tag === "input" || tag === "textarea") && "value" in el) {
      return String(el.value ?? "");
    }
    return String(el.innerText || el.textContent || "");
  }

  let element = null;
  let resolvedBy = "";
  let matchCount = 0;
  if (selector) {
    let nodes;
    try {
      nodes = Array.from(document.querySelectorAll(selector));
    } catch (error) {
      return {
        ok: false,
        error_code: "CHROME_SET_FIELD_SELECTOR_INVALID",
        error_detail: `querySelectorAll(${selector}) threw: ${String(error && error.message || error)}`
      };
    }
    const editable = nodes.filter((node) => isEditable(node) && isVisible(node));
    matchCount = editable.length;
    if (editable.length === 0) {
      return {
        ok: false,
        error_code: "CHROME_SET_FIELD_NOT_FOUND",
        error_detail: `selector matched ${nodes.length} node(s), 0 editable+visible`,
        selector_match_count: nodes.length
      };
    }
    if (editable.length > 1) {
      return {
        ok: false,
        error_code: "CHROME_SET_FIELD_NOT_UNIQUE",
        error_detail: `selector matched ${editable.length} editable+visible nodes; refine the selector for a unique target`,
        editable_match_count: editable.length
      };
    }
    element = editable[0];
    resolvedBy = "selector";
  } else if (wantActive) {
    element = document.activeElement;
    resolvedBy = "active_element";
    matchCount = element ? 1 : 0;
  } else {
    return {
      ok: false,
      error_code: "CHROME_SET_FIELD_BAD_LOCATOR",
      error_detail: "setFieldValue requires a selector or active_element=true"
    };
  }

  if (!element) {
    return { ok: false, error_code: "CHROME_ACTIVE_ELEMENT_MISSING", error_detail: "no element resolved" };
  }
  if (!isEditable(element)) {
    return {
      ok: false,
      error_code: "CHROME_ACTIVE_ELEMENT_NOT_EDITABLE",
      error_detail: `resolved element ${String(element.tagName || "")} is not an editable field`
    };
  }

  const beforeValue = readValue(element);
  const tag = String(element.tagName || "").toLowerCase();
  if (resolveOnly) {
    return {
      ok: true,
      resolved_by: resolvedBy,
      match_count: matchCount,
      tag_name: tag,
      is_editable: true,
      before_value: beforeValue
    };
  }
  try {
    element.focus();
  } catch (_) {
    // focus failures are not fatal; value mutation + readback is the source of truth
  }

  // beforeinput is cancelable; a page handler may legitimately block input.
  let beforeInputEvent;
  try {
    beforeInputEvent = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      data: inputText,
      inputType: "insertReplacementText"
    });
  } catch (_) {
    beforeInputEvent = new Event("beforeinput", { bubbles: true, cancelable: true });
  }
  const beforeInputOk = element.dispatchEvent(beforeInputEvent);
  if (!beforeInputOk) {
    return {
      ok: false,
      error_code: "CHROME_BEFOREINPUT_CANCELLED",
      error_detail: "a page beforeinput listener cancelled the background replace",
      before_value: beforeValue
    };
  }

  let expected;
  if ((tag === "input" || tag === "textarea") && "value" in element) {
    const proto = tag === "input" ? window.HTMLInputElement.prototype : window.HTMLTextAreaElement.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
    if (descriptor && typeof descriptor.set === "function") {
      descriptor.set.call(element, inputText);
    } else {
      element.value = inputText;
    }
    if (typeof element.setSelectionRange === "function") {
      try {
        element.setSelectionRange(inputText.length, inputText.length);
      } catch (_) {
        // selection is best-effort; ignore on inputs that disallow it
      }
    }
    expected = String(element.value ?? "");
  } else {
    element.textContent = inputText;
    expected = String(element.innerText || element.textContent || "");
  }

  let inputEvent;
  try {
    inputEvent = new InputEvent("input", { bubbles: true, cancelable: false, data: inputText, inputType: "insertReplacementText" });
  } catch (_) {
    inputEvent = new Event("input", { bubbles: true });
  }
  element.dispatchEvent(inputEvent);
  element.dispatchEvent(new Event("change", { bubbles: true }));

  const afterValue = readValue(element);
  if (afterValue !== expected) {
    return {
      ok: false,
      error_code: "CHROME_SET_FIELD_VALUE_MISMATCH",
      error_detail: `post-set readback diverged from the applied value (expected_len=${expected.length} after_len=${afterValue.length}); a page framework may have reverted it`,
      resolved_by: resolvedBy,
      match_count: matchCount,
      tag_name: tag,
      before_value: beforeValue,
      after_value: afterValue,
      expected_value: expected
    };
  }

  return {
    ok: true,
    resolved_by: resolvedBy,
    match_count: matchCount,
    tag_name: tag,
    is_editable: true,
    before_value: beforeValue,
    after_value: afterValue,
    expected_value: expected
  };
}

function normalizeDomAction(action) {
  const normalized = String(action || "").trim().toLowerCase();
  if (normalized === "dispatchevent" || normalized === "dispatch-event") {
    return "dispatch_event";
  }
  if (normalized === "selecttext" || normalized === "select-text") {
    return "select_text";
  }
  if (["click", "dblclick", "press", "select", "submit", "dispatch_event", "clear", "focus", "blur", "select_text", "check", "uncheck"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(
    ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
    `domAction action must be one of click, dblclick, press, select, submit, dispatch_event, clear, focus, blur, select_text, check, uncheck; got ${JSON.stringify(action)}`
  );
}

function stringOrNull(value) {
  if (value === null || value === undefined) {
    return null;
  }
  const text = String(value).trim();
  return text ? text : null;
}

function plainObjectOrEmpty(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  return value;
}

function normalizeAriaMaxNodes(value) {
  if (value === undefined || value === null) {
    return 500;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 1 || number > 5000) {
    throw bridgeError(ERROR_ATTACH_FAILED, "ariaSnapshot maxNodes must be an integer from 1 through 5000");
  }
  return number;
}

function normalizeAriaMaxDepth(value) {
  if (value === undefined || value === null) {
    return 32;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 1 || number > 128) {
    throw bridgeError(ERROR_ATTACH_FAILED, "ariaSnapshot maxDepth must be an integer from 1 through 128");
  }
  return number;
}

function normalizeAssertLimit(value) {
  if (value === undefined || value === null) {
    return 2;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 1 || number > 500) {
    throw bridgeError(ERROR_ATTACH_FAILED, "assertPoll limit must be an integer from 1 through 500");
  }
  return number;
}

function normalizeInspectMaxHtmlBytes(value) {
  if (value === undefined || value === null) {
    return 256 * 1024;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0 || number > 2 * 1024 * 1024) {
    throw bridgeError(ERROR_ATTACH_FAILED, "inspectElement maxHtmlBytes must be an integer from 0 through 2097152");
  }
  return number;
}

function parseChromeBridgeElementId(value, expectedTabId, commandName) {
  const raw = stringOrNull(value);
  if (!raw) {
    return { raw: null, frameId: null, path: null };
  }
  const match = /^chrome-tab:(\d+):frame:(\d+):path:([0-9.]+)$/.exec(raw);
  if (!match) {
    throw bridgeError(
      ERROR_ATTACH_FAILED,
      `${commandName} root element id must be a normal bridge element id like chrome-tab:<tabId>:frame:<frameId>:path:<domPath>; got ${JSON.stringify(raw)}`
    );
  }
  const tabId = Number(match[1]);
  const frameId = Number(match[2]);
  if (tabId !== expectedTabId) {
    throw bridgeError(
      ERROR_ATTACH_FAILED,
      `${commandName} root element id tab ${tabId} does not match selected tab ${expectedTabId}`
    );
  }
  return { raw, frameId, path: match[3] };
}

function chromeBridgeElementId(tabId, frameId, localPath) {
  return `chrome-tab:${tabId}:frame:${frameId}:path:${String(localPath || "0")}`;
}

function prefixAriaSnapshotNode(node, tabId, frameId) {
  const localId = String(node?.element_id || "0");
  const localParent = node?.parent_element_id == null ? null : String(node.parent_element_id);
  return {
    element_id: chromeBridgeElementId(tabId, frameId, localId),
    parent_element_id: localParent ? chromeBridgeElementId(tabId, frameId, localParent) : null,
    depth: Number.isSafeInteger(node?.depth) ? node.depth : 0,
    role: String(node?.role || "generic"),
    name: String(node?.name || ""),
    value: node?.value == null ? null : String(node.value),
    enabled: node?.enabled !== false,
    focused: Boolean(node?.focused),
    children_count: Number.isSafeInteger(node?.children_count) ? node.children_count : 0
  };
}

function renderPrefixedAriaSnapshot(nodes) {
  return nodes.map((node) => {
    let line = `${"  ".repeat(Math.max(0, node.depth))}- ${node.role || "generic"}`;
    if (node.name) {
      line += ` "${escapeSnapshotScalar(node.name)}"`;
    }
    if (node.value) {
      line += `: "${escapeSnapshotScalar(node.value)}"`;
    }
    if (!node.enabled) {
      line += " [disabled]";
    }
    if (node.focused) {
      line += " [focused]";
    }
    return line;
  }).join("\n");
}

function escapeSnapshotScalar(value) {
  return String(value || "").replace(/\\/g, "\\\\").replace(/"/g, "\\\"");
}

function normalizeClickCount(value, action = "click") {
  if (value === null || value === undefined) {
    return action === "dblclick" ? 2 : 1;
  }
  const clicks = Number(value);
  if (!Number.isSafeInteger(clicks) || clicks < 1 || clicks > 3) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction clicks must be an integer in 1..=3; got ${JSON.stringify(value)}`
    );
  }
  if (action === "dblclick" && clicks !== 2) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction dblclick requires clicks/clickCount to be omitted or 2; got ${JSON.stringify(value)}`
    );
  }
  return clicks;
}

function normalizeMouseButton(value, owner = "domAction") {
  if (value === null || value === undefined || String(value).trim() === "") {
    return "left";
  }
  const normalized = String(value).trim().toLowerCase();
  if (["left", "right", "middle"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(
    ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
    `${owner} button must be one of left, right, middle; got ${JSON.stringify(value)}`
  );
}

function normalizeClickModifiers(value, owner = "domAction") {
  if (value === null || value === undefined) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `${owner} modifiers must be an array of ctrl/shift/alt/meta strings; got ${JSON.stringify(value)}`
    );
  }
  const seen = new Set();
  for (const item of value) {
    const normalized = String(item || "").trim().toLowerCase();
    let canonical = null;
    if (normalized === "ctrl" || normalized === "control") {
      canonical = "ctrl";
    } else if (normalized === "shift") {
      canonical = "shift";
    } else if (normalized === "alt" || normalized === "option") {
      canonical = "alt";
    } else if (["meta", "super", "cmd", "command"].includes(normalized)) {
      canonical = "meta";
    }
    if (!canonical) {
      throw bridgeError(
        ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
        `${owner} modifier must be ctrl, shift, alt, or meta; got ${JSON.stringify(item)}`
      );
    }
    seen.add(canonical);
  }
  return ["alt", "ctrl", "meta", "shift"].filter((modifier) => seen.has(modifier));
}

function normalizeClickPosition(params, owner = "domAction") {
  const nested = params?.position && typeof params.position === "object" && !Array.isArray(params.position)
    ? params.position
    : null;
  const rawX = params?.positionX ?? params?.offsetX ?? nested?.x;
  const rawY = params?.positionY ?? params?.offsetY ?? nested?.y;
  if (rawX === null || rawX === undefined || rawY === null || rawY === undefined) {
    if (rawX === null || rawX === undefined) {
      if (rawY === null || rawY === undefined) {
        return null;
      }
    }
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `${owner} position requires both x and y; got x=${JSON.stringify(rawX)} y=${JSON.stringify(rawY)}`
    );
  }
  const x = Number(rawX);
  const y = Number(rawY);
  if (!Number.isSafeInteger(x) || !Number.isSafeInteger(y) || x < 0 || y < 0) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `${owner} position must use non-negative safe-integer CSS pixels; got x=${JSON.stringify(rawX)} y=${JSON.stringify(rawY)}`
    );
  }
  return { x, y };
}

function normalizeEventType(value) {
  const eventType = String(value || "").trim();
  if (!eventType) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction dispatch_event requires non-empty eventType; got ${JSON.stringify(value)}`
    );
  }
  if ([...eventType].some((ch) => ch < " " || ch === "\u007f")) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction dispatch_event eventType contains control characters: ${JSON.stringify(value)}`
    );
  }
  return eventType;
}

function normalizeEventInit(value) {
  if (value === null || value === undefined) {
    return {};
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction dispatch_event eventInit must be a JSON object; got ${JSON.stringify(value)}`
    );
  }
  return value;
}

function normalizeInteger(value, fieldName) {
  const number = Number(value);
  if (!Number.isSafeInteger(number)) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction ${fieldName} must be a safe integer; got ${JSON.stringify(value)}`
    );
  }
  return number;
}

function normalizeSelectOptions(value) {
  if (value === null || value === undefined) {
    return [];
  }
  if (!Array.isArray(value)) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction select options must be an array of {value,label,index}; got ${JSON.stringify(value)}`
    );
  }
  return value.map((item, index) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      throw bridgeError(
        ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
        `domAction select options[${index}] must be an object; got ${JSON.stringify(item)}`
      );
    }
    const spec = {};
    if (item.value !== null && item.value !== undefined) {
      spec.value = String(item.value);
    }
    if (item.label !== null && item.label !== undefined) {
      spec.label = String(item.label);
    }
    if (item.index !== null && item.index !== undefined) {
      spec.index = normalizeInteger(item.index, `options[${index}].index`);
    }
    return spec;
  });
}

function normalizeCoordinateValue(value, fieldName) {
  const coordinate = Number(value);
  if (!Number.isSafeInteger(coordinate)) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `coordinateClick ${fieldName} must be a safe integer; got ${JSON.stringify(value)}`
    );
  }
  return coordinate;
}

function normalizeCoordinateSpace(value) {
  const normalized = String(value || "screen").trim().toLowerCase();
  if (["screen", "window", "viewport"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(
    ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
    `coordinateClick coordinateSpace must be one of screen, window, viewport; got ${JSON.stringify(value)}`
  );
}

async function performDomActionInPage(request) {
  const ERROR_SELECTOR_INVALID = "CHROME_DOM_SELECTOR_INVALID";
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const ERROR_ELEMENT_AMBIGUOUS = "CHROME_DOM_ELEMENT_AMBIGUOUS";
  const ERROR_ELEMENT_NOT_ACTIONABLE = "CHROME_DOM_ELEMENT_NOT_ACTIONABLE";
  const ERROR_ACTION_UNSUPPORTED = "CHROME_DOM_ACTION_UNSUPPORTED";
  const ERROR_POSTCONDITION_FAILED = "CHROME_DOM_ACTION_POSTCONDITION_FAILED";
  const ERROR_BROWSER_WAIT_TIMEOUT = "BROWSER_WAIT_TIMEOUT";

  const action = String(request?.action || "").trim().toLowerCase();
  const locator = {
    selector: stringOrEmpty(request?.selector),
    elementId: stringOrEmpty(request?.elementId),
    role: normalizeText(request?.role),
    name: normalizeText(request?.name),
    value: stringOrEmpty(request?.value),
    option: stringOrEmpty(request?.option),
    optionLabel: stringOrEmpty(request?.optionLabel),
    optionIndex: Number.isSafeInteger(request?.optionIndex) ? request.optionIndex : null,
    options: Array.isArray(request?.options) ? request.options : []
  };
  const clicks = Number.isSafeInteger(request?.clicks) ? request.clicks : (action === "dblclick" ? 2 : 1);
  const clickOptions = {
    button: normalizeMouseButtonLocal(request?.button),
    modifiers: normalizeClickModifiersLocal(request?.modifiers),
    position: normalizeClickPositionLocal(request?.position)
  };
  const eventType = stringOrEmpty(request?.eventType);
  const eventInit = plainObjectOrEmpty(request?.eventInit);
  const resolveOnly = Boolean(request?.resolveOnly);
  const autoWait = Boolean(request?.autoWait);
  const autoWaitTimeoutMs = Number.isSafeInteger(request?.autoWaitTimeoutMs)
    ? Math.max(50, Math.min(request.autoWaitTimeoutMs, 30000))
    : 2000;
  const maxPageTextChars = Number.isSafeInteger(request?.maxPageTextChars)
    ? Math.min(Math.max(request.maxPageTextChars, 0), 65536)
    : 4096;

  if (!["click", "dblclick", "press", "select", "submit", "dispatch_event", "clear", "focus", "blur", "select_text", "check", "uncheck"].includes(action)) {
    return fail(ERROR_ACTION_UNSUPPORTED, `unsupported DOM action ${JSON.stringify(action)}`);
  }
  if (action === "dispatch_event" && !eventType) {
    return fail(ERROR_ACTION_UNSUPPORTED, "dispatch_event requires a non-empty eventType");
  }

  const beforeUrl = String(location.href || "");
  const beforePageText = readPageText(maxPageTextChars);
  const resolved = resolveDomActionElement(action, locator);
  if (!resolved.ok) {
    return resolved;
  }
  const element = resolved.element;
  const beforeElement = elementSummary(element);

  const bypassActionability = ["dispatch_event", "focus", "blur", "select_text"].includes(action);
  const deferActionability = autoWait && !bypassActionability;
  if (!bypassActionability && !deferActionability && !isElementEnabled(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is disabled or aria-disabled", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement
    });
  }
  if (!bypassActionability && !deferActionability && !isElementVisible(element) && action !== "submit") {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is not visible/actionable", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement
    });
  }
  if (resolveOnly) {
    return {
      ok: true,
      action,
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement,
      resolve_only: true,
      auto_wait: autoWait,
      in_page_before_url: beforeUrl,
      in_page_after_url: String(location.href || ""),
      in_page_title: String(document.title || ""),
      in_page_ready_state: String(document.readyState || ""),
      in_page_before_text: beforePageText,
      in_page_after_text: beforePageText
    };
  }

  const eventsDispatched = [];
  let actionReadback = {};
  let autoWaitReadback = null;
  if (autoWait && !bypassActionability) {
    autoWaitReadback = await waitForDomActionability(element, action, autoWaitTimeoutMs);
    if (!autoWaitReadback.ok) {
      return fail(
        ERROR_BROWSER_WAIT_TIMEOUT,
        `domAction ${action} auto_wait timed out after ${autoWaitTimeoutMs} ms waiting for ${autoWaitReadback.requirement}; unmet predicates: ${autoWaitReadback.predicate_detail}`,
        {
          matched_count: resolved.matchedCount,
          resolved_by: resolved.resolvedBy,
          before_element: beforeElement,
          auto_wait: true,
          auto_wait_readback: autoWaitReadback,
          source_of_truth: "Element.scrollIntoView({block:nearest,inline:nearest}) + getBoundingClientRect + elementFromPoint"
        }
      );
    }
  }
  if (!bypassActionability && !isElementEnabled(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is disabled or aria-disabled", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement,
      auto_wait: autoWait,
      auto_wait_readback: autoWaitReadback
    });
  }
  if (!bypassActionability && !isElementVisible(element) && action !== "submit") {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is not visible/actionable", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement,
      auto_wait: autoWait,
      auto_wait_readback: autoWaitReadback
    });
  }
  try {
    if (action === "click" || action === "press" || action === "dblclick") {
      actionReadback = performClick(element, action, clicks, clickOptions, eventsDispatched);
    } else if (action === "select") {
      actionReadback = performNativeSelect(element, locator, eventsDispatched);
    } else if (action === "submit") {
      actionReadback = performSubmit(element, eventsDispatched);
    } else if (action === "dispatch_event") {
      actionReadback = performDispatchEvent(element, eventType, eventInit, eventsDispatched);
    } else if (action === "clear") {
      actionReadback = performClear(element, eventsDispatched);
    } else if (action === "focus") {
      actionReadback = performFocus(element, eventsDispatched);
    } else if (action === "blur") {
      actionReadback = performBlur(element, eventsDispatched);
    } else if (action === "select_text") {
      actionReadback = performSelectText(element, eventsDispatched);
    } else if (action === "check") {
      actionReadback = performCheckState(element, true, eventsDispatched);
    } else if (action === "uncheck") {
      actionReadback = performCheckState(element, false, eventsDispatched);
    }
  } catch (error) {
    return fail(error?.code || ERROR_ACTION_UNSUPPORTED, errorMessageLocal(error), {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement,
      auto_wait: autoWait,
      auto_wait_readback: autoWaitReadback,
      events_dispatched: eventsDispatched
    });
  }

  const afterElement = element.isConnected ? elementSummary(element) : null;
  const afterPageText = readPageText(maxPageTextChars);
  return {
    ok: true,
    action,
    matched_count: resolved.matchedCount,
    resolved_by: resolved.resolvedBy,
    before_element: beforeElement,
    after_element: afterElement,
    auto_wait: autoWait,
    auto_wait_readback: autoWaitReadback,
    events_dispatched: eventsDispatched,
    action_readback: actionReadback,
    in_page_before_url: beforeUrl,
    in_page_after_url: String(location.href || ""),
    in_page_title: String(document.title || ""),
    in_page_ready_state: String(document.readyState || ""),
    in_page_before_text: beforePageText,
    in_page_after_text: afterPageText
  };

  async function waitForDomActionability(element, actionName, timeoutMs) {
    const started = Date.now();
    const deadline = started + timeoutMs;
    const requirement = actionName === "clear" ? "editable_action_ready" : "action_ready";
    let pollCount = 0;
    let lastActionability = null;
    let lastScroll = null;
    while (true) {
      lastScroll = await scrollIntoViewIfNeededSnapshot(element);
      lastActionability = await domActionabilitySnapshot(element);
      pollCount += 1;
      const ready = requirement === "editable_action_ready"
        ? lastActionability.editable_action_ready
        : lastActionability.action_ready;
      const unmetPredicates = actionabilityRelevantFailures(lastActionability, requirement);
      const predicateDetail = unmetPredicates.length
        ? unmetPredicates.map((failure) => failure.predicate).join(",")
        : "unknown";
      const readback = {
        ok: Boolean(ready),
        requirement,
        timeout_ms: timeoutMs,
        poll_count: pollCount,
        elapsed_ms: Math.max(0, Date.now() - started),
        unmet_predicates: unmetPredicates,
        predicate_detail: predicateDetail,
        last_actionability: lastActionability,
        scroll: lastScroll
      };
      if (ready) {
        return readback;
      }
      const now = Date.now();
      if (now >= deadline) {
        return readback;
      }
      await delay(Math.min(50, Math.max(0, deadline - now)));
    }
  }

  async function domActionabilitySnapshot(element) {
    const firstBox = domBoxModel(element);
    await twoAnimationFramesLocal();
    const secondBox = domBoxModel(element);
    const attached = Boolean(element && element.isConnected);
    const visible = attached && isElementVisible(element) && Boolean(firstBox?.content?.width > 0 && firstBox?.content?.height > 0);
    const enabled = isElementEnabled(element);
    const editable = Boolean(editableKind(element)) && !Boolean(element.disabled) && !Boolean(element.readOnly);
    const runningAnimation = hasRunningAnimationLocal(element);
    const stable = !runningAnimation && domBoxesStable(firstBox, secondBox);
    const hitTest = domHitTest(element, firstBox);
    const receivesEvents = hitTest.receives_events;
    const failureReasons = [];
    if (!attached) {
      failureReasons.push(domFailure("attached", "detached", "target element is not connected to the document"));
    }
    if (!visible) {
      failureReasons.push(domFailure("visible", "not_visible", "target element has no visible rendered box"));
    }
    if (!stable) {
      failureReasons.push(domFailure(
        "stable",
        runningAnimation ? "animation_running" : "box_changed",
        runningAnimation
          ? "target element has a running CSS/Web Animation"
          : "target element box changed between animation-frame samples"
      ));
    }
    if (!enabled) {
      failureReasons.push(domFailure("enabled", "disabled", "target element is disabled or aria-disabled"));
    }
    if (!editable && editableKind(element) && Boolean(element.readOnly)) {
      failureReasons.push(domFailure("editable", "readonly", "target editable element is readonly"));
    }
    if (!receivesEvents) {
      failureReasons.push(domFailure(
        "receives_events",
        "obscured",
        hitTest.top_node_description
          ? `action point is covered by ${hitTest.top_node_description}`
          : "document.elementFromPoint did not resolve to the target or one of its descendants"
      ));
    }
    const actionReady = attached && visible && stable && enabled && receivesEvents;
    return {
      attached,
      visible,
      stable,
      enabled,
      editable,
      receives_events: receivesEvents,
      action_ready: actionReady,
      editable_action_ready: actionReady && editable,
      failure_reasons: failureReasons,
      box_model: firstBox,
      second_box_model: secondBox,
      hit_test: hitTest
    };
  }

  async function scrollIntoViewIfNeededSnapshot(element) {
    const beforeContainer = nearestScrollContainerLocal(element);
    const before = domScrollSnapshot(element, beforeContainer);
    let scrollPerformed = false;
    if (!rectFullyInViewportLocal(before.viewport_rect) || !rectFullyInScrollContainerLocal(before.viewport_rect, before.scroll_container)) {
      try {
        element.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "instant" });
        scrollPerformed = true;
      } catch (_) {
        try {
          element.scrollIntoView();
          scrollPerformed = true;
        } catch (error) {
          return {
            ok: false,
            error: errorMessageLocal(error),
            before_viewport: before.viewport,
            before_scroll_container: before.scroll_container,
            scroll_performed: false
          };
        }
      }
    }
    if (scrollPerformed) {
      await twoAnimationFramesLocal();
    }
    const afterContainer = nearestScrollContainerLocal(element);
    const after = domScrollSnapshot(element, afterContainer);
    return {
      ok: true,
      before_viewport: before.viewport,
      after_viewport: after.viewport,
      before_box_model: before.box_model,
      after_box_model: after.box_model,
      before_scroll_container: before.scroll_container,
      after_scroll_container: after.scroll_container,
      window_scroll_changed:
        before.viewport.scroll_x !== after.viewport.scroll_x ||
        before.viewport.scroll_y !== after.viewport.scroll_y,
      container_scroll_changed: scrollContainerChangedLocal(before.scroll_container, after.scroll_container),
      node_fully_in_viewport_after: rectFullyInViewportLocal(after.viewport_rect),
      scroll_performed: scrollPerformed
    };
  }

  function actionabilityRelevantFailures(actionability, requirement) {
    const includesEditable = requirement === "editable_action_ready";
    return (Array.isArray(actionability?.failure_reasons) ? actionability.failure_reasons : [])
      .filter((failure) =>
        ["attached", "visible", "stable", "enabled", "receives_events"].includes(failure.predicate) ||
        (includesEditable && failure.predicate === "editable")
      )
      .map((failure) => ({
        predicate: failure.predicate,
        reason: failure.reason,
        detail: failure.detail
      }));
  }

  function domHitTest(element, boxModel) {
    if (!boxModel || !boxModel.content || boxModel.content.width <= 0 || boxModel.content.height <= 0) {
      return {
        point: null,
        receives_events: false,
        target_or_descendant_from_point: false,
        top_node_description: null,
        error: "no action point"
      };
    }
    const rect = viewportRectLocal(element);
    const point = {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2
    };
    let top = null;
    let error = null;
    try {
      top = document.elementFromPoint(point.x, point.y);
    } catch (caught) {
      error = errorMessageLocal(caught);
    }
    const targetOrDescendant = top instanceof Element && (top === element || element.contains(top));
    return {
      point,
      receives_events: Boolean(targetOrDescendant),
      target_or_descendant_from_point: Boolean(targetOrDescendant),
      top_node_description: top instanceof Element ? nodeDescriptionLocal(top) : null,
      error
    };
  }

  function domBoxModel(element) {
    if (!(element instanceof Element) || typeof element.getBoundingClientRect !== "function") {
      return null;
    }
    const rect = element.getBoundingClientRect();
    if (!rect) {
      return null;
    }
    const pageRect = {
      x: Number(rect.left || 0) + Number(window.scrollX || 0),
      y: Number(rect.top || 0) + Number(window.scrollY || 0),
      width: Number(rect.width || 0),
      height: Number(rect.height || 0)
    };
    return {
      content: pageRect,
      border: { ...pageRect },
      width: Math.round(pageRect.width),
      height: Math.round(pageRect.height)
    };
  }

  function domScrollSnapshot(element, scrollContainer) {
    const rect = viewportRectLocal(element);
    return {
      viewport: {
        inner_width: Math.trunc(Number(window.innerWidth || 0)),
        inner_height: Math.trunc(Number(window.innerHeight || 0)),
        scroll_x: Number(window.scrollX || 0),
        scroll_y: Number(window.scrollY || 0),
        device_pixel_ratio: Number(window.devicePixelRatio || 1)
      },
      viewport_rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height
      },
      box_model: domBoxModel(element),
      scroll_container: scrollContainerSnapshotLocal(scrollContainer)
    };
  }

  function nearestScrollContainerLocal(element) {
    let current = element instanceof Element ? element.parentElement : null;
    while (current && current !== document.documentElement) {
      const style = window.getComputedStyle(current);
      const overflow = `${style.overflow || ""} ${style.overflowX || ""} ${style.overflowY || ""}`;
      const scrollableStyle = /(auto|scroll|overlay)/i.test(overflow);
      const scrollableGeometry =
        current.scrollHeight > current.clientHeight ||
        current.scrollWidth > current.clientWidth;
      if (scrollableStyle && scrollableGeometry) {
        return current;
      }
      current = current.parentElement;
    }
    return null;
  }

  function scrollContainerSnapshotLocal(element) {
    if (!(element instanceof Element)) {
      return null;
    }
    const rect = viewportRectLocal(element);
    return {
      tag_name: tag(element),
      id: stringOrEmpty(element.id),
      class_name: stringOrEmpty(element.className),
      scroll_top: Number(element.scrollTop || 0),
      scroll_left: Number(element.scrollLeft || 0),
      scroll_height: Number(element.scrollHeight || 0),
      scroll_width: Number(element.scrollWidth || 0),
      client_height: Number(element.clientHeight || 0),
      client_width: Number(element.clientWidth || 0),
      viewport_rect: {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height
      }
    };
  }

  function scrollContainerChangedLocal(before, after) {
    if (!before && !after) {
      return false;
    }
    if (!before || !after) {
      return true;
    }
    return before.scroll_top !== after.scroll_top || before.scroll_left !== after.scroll_left;
  }

  function rectFullyInViewportLocal(rect) {
    if (!rect) {
      return false;
    }
    const epsilon = 0.5;
    return (
      rect.x >= -epsilon &&
      rect.y >= -epsilon &&
      rect.x + rect.width <= Number(window.innerWidth || 0) + epsilon &&
      rect.y + rect.height <= Number(window.innerHeight || 0) + epsilon
    );
  }

  function rectFullyInScrollContainerLocal(rect, scrollContainer) {
    if (!rect || !scrollContainer || !scrollContainer.viewport_rect) {
      return true;
    }
    const container = scrollContainer.viewport_rect;
    const epsilon = 0.5;
    return (
      rect.x >= container.x - epsilon &&
      rect.y >= container.y - epsilon &&
      rect.x + rect.width <= container.x + container.width + epsilon &&
      rect.y + rect.height <= container.y + container.height + epsilon
    );
  }

  function domBoxesStable(first, second) {
    if (!first || !second || !first.content || !second.content) {
      return false;
    }
    const epsilon = 0.25;
    return (
      Math.abs(first.content.x - second.content.x) <= epsilon &&
      Math.abs(first.content.y - second.content.y) <= epsilon &&
      Math.abs(first.content.width - second.content.width) <= epsilon &&
      Math.abs(first.content.height - second.content.height) <= epsilon
    );
  }

  function hasRunningAnimationLocal(element) {
    if (!(element instanceof Element) || typeof element.getAnimations !== "function") {
      return false;
    }
    try {
      return element.getAnimations({ subtree: false }).some((animation) => animation.playState === "running");
    } catch (_) {
      return false;
    }
  }

  function viewportRectLocal(element) {
    if (!(element instanceof Element) || typeof element.getBoundingClientRect !== "function") {
      return { left: 0, top: 0, width: 0, height: 0 };
    }
    const rect = element.getBoundingClientRect();
    return {
      left: Number(rect.left || 0),
      top: Number(rect.top || 0),
      width: Number(rect.width || 0),
      height: Number(rect.height || 0)
    };
  }

  function nodeDescriptionLocal(element) {
    if (!(element instanceof Element)) {
      return "";
    }
    const id = element.id ? `#${element.id}` : "";
    const cls = typeof element.className === "string" && element.className.trim()
      ? `.${element.className.trim().split(/\s+/).slice(0, 3).join(".")}`
      : "";
    const name = element.getAttribute("name") ? `[name="${element.getAttribute("name")}"]` : "";
    return `${tag(element)}${id}${cls}${name}`;
  }

  function domFailure(predicate, reason, detail) {
    return { predicate, reason, detail };
  }

  function twoAnimationFramesLocal() {
    return new Promise((resolve) => {
      let done = false;
      const finish = () => {
        if (done) {
          return;
        }
        done = true;
        resolve();
      };
      setTimeout(finish, 100);
      if (typeof requestAnimationFrame === "function") {
        requestAnimationFrame(() => requestAnimationFrame(finish));
      }
    });
  }

  function delay(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  function resolveDomActionElement(actionName, loc) {
    let candidates = [];
    let resolvedBy = "semantic";
    if (loc.selector) {
      resolvedBy = "selector";
      try {
        candidates = Array.from(document.querySelectorAll(loc.selector));
      } catch (error) {
        return fail(ERROR_SELECTOR_INVALID, `invalid selector ${JSON.stringify(loc.selector)}: ${errorMessageLocal(error)}`);
      }
    } else if (loc.elementId) {
      resolvedBy = "element_id";
      const id = loc.elementId.startsWith("#") ? loc.elementId.slice(1) : loc.elementId;
      const byId = id ? document.getElementById(id) : null;
      candidates = byId ? [byId] : [];
    } else {
      candidates = semanticCandidates(actionName);
    }

    const filtered = candidates.filter((candidate) => {
      if (!(candidate instanceof Element)) {
        return false;
      }
      if (loc.role && normalizeText(inferRole(candidate)) !== loc.role) {
        return false;
      }
      if (loc.name && !textMatches(accessibleName(candidate), loc.name)) {
        return false;
      }
      if (loc.value && actionName !== "select" && !valueMatches(candidate, loc.value)) {
        return false;
      }
      return true;
    });

    if (filtered.length === 0) {
      return fail(ERROR_ELEMENT_NOT_FOUND, `no DOM element matched ${locatorDebug(loc)}`, {
        matched_count: 0,
        resolved_by: resolvedBy
      });
    }
    if (filtered.length > 1) {
      return fail(ERROR_ELEMENT_AMBIGUOUS, `${filtered.length} DOM elements matched ${locatorDebug(loc)}`, {
        matched_count: filtered.length,
        resolved_by: resolvedBy,
        candidates: filtered.slice(0, 5).map(elementSummary)
      });
    }
    return {
      ok: true,
      element: filtered[0],
      matchedCount: filtered.length,
      resolvedBy
    };
  }

  function semanticCandidates(actionName) {
    if (["dispatch_event", "focus", "blur", "select_text"].includes(actionName)) {
      return Array.from(document.querySelectorAll("*"));
    }
    if (actionName === "clear") {
      return Array.from(document.querySelectorAll("input,textarea,[contenteditable],[role='textbox']"));
    }
    if (actionName === "check" || actionName === "uncheck") {
      return Array.from(document.querySelectorAll("input[type='checkbox'],input[type='radio'],[role='checkbox'],[role='radio']"));
    }
    if (actionName === "select") {
      return Array.from(document.querySelectorAll("select,[role='combobox'],[role='listbox']"));
    }
    if (actionName === "submit") {
      return Array.from(document.querySelectorAll("form,button,input[type='submit'],input[type='image'],input[type='button'],[role='button']"));
    }
    return Array.from(document.querySelectorAll("button,a[href],input[type='button'],input[type='submit'],input[type='reset'],summary,[role='button'],[role='link'],[role='menuitem'],[role='tab'],[tabindex]"));
  }

  function performClick(element, actionName, count, options, events) {
    scrollIntoViewIfPossible(element);
    const bounded = Math.min(Math.max(count, 1), 3);
    const button = options?.button || "left";
    const modifiers = Array.isArray(options?.modifiers) ? options.modifiers : [];
    const needsSyntheticEvents = actionName === "dblclick" || bounded !== 1 || button !== "left" || modifiers.length > 0 || Boolean(options?.position);
    const popupIntents = [];
    const restoreWindowOpen = installWindowOpenCapture(popupIntents);
    const anchorIntent = captureAnchorTargetBlankIntent(element);
    let readback;
    try {
      if (!needsSyntheticEvents && typeof element.click === "function") {
        element.click();
        events.push("click");
        readback = {
          click_count: 1,
          button,
          modifiers,
          position: null,
          activation_event: "click",
          native_click_method: true
        };
      } else {
        const point = elementClickPoint(element, options?.position || null);
        let activationEvent = "click";
        for (let index = 0; index < bounded; index += 1) {
          const detail = index + 1;
          dispatchPointerLikeEvent(element, "pointerdown", point, button, modifiers, detail, true);
          events.push("pointerdown");
          dispatchMouseLikeEvent(element, "mousedown", point, button, modifiers, detail, true);
          events.push("mousedown");
          dispatchPointerLikeEvent(element, "pointerup", point, button, modifiers, detail, false);
          events.push("pointerup");
          dispatchMouseLikeEvent(element, "mouseup", point, button, modifiers, detail, false);
          events.push("mouseup");
          activationEvent = activationEventForButton(button);
          dispatchMouseLikeEvent(element, activationEvent, point, button, modifiers, detail, false);
          events.push(activationEvent);
          if (button === "left" && detail === 2 && (actionName === "dblclick" || bounded >= 2)) {
            dispatchMouseLikeEvent(element, "dblclick", point, button, modifiers, detail, false);
            events.push("dblclick");
          }
        }
        readback = {
          click_count: bounded,
          button,
          modifiers,
          position: options?.position || null,
          viewport_point: {
            client_x: point.client_x,
            client_y: point.client_y
          },
          element_point: {
            x: point.element_x,
            y: point.element_y
          },
          activation_event: activationEvent
        };
      }
    } finally {
      restoreWindowOpen();
    }
    return {
      ...readback,
      popup_intents: popupIntents,
      anchor_target_blank_intent: anchorIntent
    };
  }

  function installWindowOpenCapture(intents) {
    if (typeof window.open !== "function") {
      return () => {};
    }
    const originalOpen = window.open;
    const capturedOpen = function capturedSynapseWindowOpen(url, target, features) {
      let opened = false;
      let thrown = null;
      try {
        const result = originalOpen.apply(window, arguments);
        opened = Boolean(result);
        return result;
      } catch (error) {
        thrown = errorMessageLocal(error);
        throw error;
      } finally {
        intents.push({
          kind: "window_open",
          url: stringifyPopupArg(url),
          resolved_url: resolvePopupUrl(url),
          target: stringifyPopupArg(target),
          features: stringifyPopupArg(features),
          opened,
          error: thrown
        });
      }
    };
    window.open = capturedOpen;
    return () => {
      try {
        if (window.open === capturedOpen) {
          window.open = originalOpen;
        }
      } catch (_) {
        // Best-effort restore; the action result still reports captured intent.
      }
    };
  }

  function captureAnchorTargetBlankIntent(element) {
    const anchor = typeof element.closest === "function" ? element.closest("a[href]") : null;
    if (!(anchor instanceof HTMLAnchorElement)) {
      return null;
    }
    const target = String(anchor.getAttribute("target") || "").trim();
    const normalizedTarget = target.toLowerCase();
    if (!normalizedTarget || normalizedTarget === "_self") {
      return null;
    }
    return {
      kind: "anchor_target_blank",
      url: String(anchor.getAttribute("href") || ""),
      resolved_url: String(anchor.href || ""),
      target,
      rel: String(anchor.getAttribute("rel") || ""),
      opened: false,
      text: trimForReadback(anchor.innerText || anchor.textContent || "", 200)
    };
  }

  function stringifyPopupArg(value) {
    return value === null || value === undefined ? "" : String(value);
  }

  function resolvePopupUrl(value) {
    const raw = stringifyPopupArg(value);
    if (!raw) {
      return "about:blank";
    }
    try {
      return new URL(raw, location.href).href;
    } catch (_) {
      return raw;
    }
  }

  function elementClickPoint(element, position) {
    const rect = element.getBoundingClientRect();
    if (!rect || rect.width <= 0 || rect.height <= 0) {
      throw actionError(ERROR_ELEMENT_NOT_ACTIONABLE, `resolved ${tag(element)} element has no clickable box`);
    }
    const elementX = position ? Number(position.x) : rect.width / 2;
    const elementY = position ? Number(position.y) : rect.height / 2;
    if (!Number.isFinite(elementX) || !Number.isFinite(elementY) || elementX < 0 || elementY < 0 || elementX >= rect.width || elementY >= rect.height) {
      throw actionError(
        ERROR_ELEMENT_NOT_ACTIONABLE,
        `click position (${elementX}, ${elementY}) falls outside ${tag(element)} box ${Math.round(rect.width)}x${Math.round(rect.height)}`
      );
    }
    return {
      client_x: rect.left + elementX,
      client_y: rect.top + elementY,
      element_x: elementX,
      element_y: elementY
    };
  }

  function dispatchPointerLikeEvent(element, eventType, point, button, modifiers, detail, pressed) {
    if (typeof PointerEvent !== "function") {
      dispatchMouseLikeEvent(element, eventType.replace(/^pointer/, "mouse"), point, button, modifiers, detail, pressed);
      return;
    }
    element.dispatchEvent(new PointerEvent(eventType, {
      ...mouseEventInit(point, button, modifiers, detail, pressed),
      pointerId: 1,
      pointerType: "mouse",
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: pressed ? 0.5 : 0
    }));
  }

  function dispatchMouseLikeEvent(element, eventType, point, button, modifiers, detail, pressed) {
    element.dispatchEvent(new MouseEvent(eventType, mouseEventInit(point, button, modifiers, detail, pressed)));
  }

  function mouseEventInit(point, button, modifiers, detail, pressed) {
    return {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      detail,
      button: mouseButtonCode(button),
      buttons: pressed ? mouseButtonBits(button) : 0,
      clientX: point.client_x,
      clientY: point.client_y,
      screenX: Math.round(Number(window.screenX || 0) + point.client_x),
      screenY: Math.round(Number(window.screenY || 0) + point.client_y),
      ...modifierFlags(modifiers)
    };
  }

  function activationEventForButton(button) {
    if (button === "right") {
      return "contextmenu";
    }
    if (button === "middle") {
      return "auxclick";
    }
    return "click";
  }

  function mouseButtonCode(button) {
    if (button === "middle") {
      return 1;
    }
    if (button === "right") {
      return 2;
    }
    return 0;
  }

  function mouseButtonBits(button) {
    if (button === "middle") {
      return 4;
    }
    if (button === "right") {
      return 2;
    }
    return 1;
  }

  function modifierFlags(modifiers) {
    const set = new Set(Array.isArray(modifiers) ? modifiers : []);
    return {
      altKey: set.has("alt"),
      ctrlKey: set.has("ctrl"),
      metaKey: set.has("meta"),
      shiftKey: set.has("shift")
    };
  }

  function normalizeMouseButtonLocal(value) {
    const normalized = String(value || "left").trim().toLowerCase();
    return ["left", "right", "middle"].includes(normalized) ? normalized : "left";
  }

  function normalizeClickModifiersLocal(value) {
    if (!Array.isArray(value)) {
      return [];
    }
    const seen = new Set();
    for (const item of value) {
      const normalized = String(item || "").trim().toLowerCase();
      if (normalized === "ctrl" || normalized === "control") {
        seen.add("ctrl");
      } else if (normalized === "shift") {
        seen.add("shift");
      } else if (normalized === "alt" || normalized === "option") {
        seen.add("alt");
      } else if (["meta", "super", "cmd", "command"].includes(normalized)) {
        seen.add("meta");
      }
    }
    return ["alt", "ctrl", "meta", "shift"].filter((modifier) => seen.has(modifier));
  }

  function normalizeClickPositionLocal(value) {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return null;
    }
    const x = Number(value.x);
    const y = Number(value.y);
    if (!Number.isSafeInteger(x) || !Number.isSafeInteger(y) || x < 0 || y < 0) {
      return null;
    }
    return { x, y };
  }

  function performDispatchEvent(element, requestedType, requestedInit, events) {
    const eventType = stringOrEmpty(requestedType);
    if (!eventType) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "dispatch_event requires a non-empty event type");
    }
    const eventInit = plainObjectOrEmpty(requestedInit);
    const event = createDomEvent(eventType, eventInit);
    const defaultAllowed = element.dispatchEvent(event);
    events.push(eventType);
    return {
      event_type: eventType,
      constructor_used: event.constructor && event.constructor.name ? String(event.constructor.name) : "Event",
      default_allowed: Boolean(defaultAllowed),
      default_prevented: Boolean(event.defaultPrevented),
      bubbles: Boolean(event.bubbles),
      cancelable: Boolean(event.cancelable),
      composed: Boolean(event.composed),
      detail: "detail" in event ? event.detail : null
    };
  }

  function performClear(element, events) {
    const editable = editableKind(element);
    if (!editable) {
      throw actionError(
        ERROR_ACTION_UNSUPPORTED,
        `domAction clear supports editable input/textarea/contenteditable targets only; resolved ${tag(element)} role=${inferRole(element)}`
      );
    }
    if (element.disabled || element.readOnly) {
      throw actionError(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved editable element is disabled or readonly");
    }
    try {
      if (typeof element.focus === "function") {
        element.focus({ preventScroll: true });
        events.push("focus");
      }
    } catch (_) {
      // A failed focus does not prove the DOM value cannot be cleared.
    }

    const beforeValue = elementTextValue(element);
    const beforeInput = dispatchInputLikeEvent(element, "beforeinput", null, "deleteContentBackward", true);
    events.push("beforeinput");
    if (!beforeInput) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "clear was cancelled by a beforeinput listener");
    }

    if (editable === "value") {
      setNativeValue(element, "");
      try {
        if (typeof element.setSelectionRange === "function") {
          element.setSelectionRange(0, 0);
        }
      } catch (_) {
        // Some input types expose value but reject text selection.
      }
    } else {
      element.textContent = "";
    }

    dispatchInputLikeEvent(element, "input", null, "deleteContentBackward", false);
    events.push("input");
    element.dispatchEvent(new Event("change", { bubbles: true }));
    events.push("change");

    const afterValue = elementTextValue(element);
    if (afterValue !== "") {
      throw actionError(
        ERROR_POSTCONDITION_FAILED,
        `clear postcondition failed: after value/text length ${afterValue.length} was not zero`
      );
    }
    return {
      before_value_len: beforeValue.length,
      after_value_len: afterValue.length,
      input_fired: true,
      change_fired: true
    };
  }

  function performFocus(element, events) {
    const beforeActive = activeElementSummary(element.ownerDocument, element);
    if (typeof element.focus !== "function") {
      throw actionError(ERROR_ACTION_UNSUPPORTED, `resolved ${tag(element)} element has no focus() method`);
    }
    try {
      element.focus({ preventScroll: true });
    } catch (_) {
      element.focus();
    }
    events.push("focus");
    const afterActive = activeElementSummary(element.ownerDocument, element);
    if (element.ownerDocument.activeElement !== element) {
      throw actionError(
        ERROR_POSTCONDITION_FAILED,
        `focus postcondition failed: activeElement is ${afterActive.tag_name || "none"}#${afterActive.id || ""}`
      );
    }
    return {
      before_active_element: beforeActive,
      after_active_element: afterActive,
      active_element_is_target: true
    };
  }

  function performBlur(element, events) {
    const beforeActive = activeElementSummary(element.ownerDocument, element);
    if (typeof element.blur !== "function") {
      throw actionError(ERROR_ACTION_UNSUPPORTED, `resolved ${tag(element)} element has no blur() method`);
    }
    element.blur();
    events.push("blur");
    const afterActive = activeElementSummary(element.ownerDocument, element);
    if (element.ownerDocument.activeElement === element) {
      throw actionError(ERROR_POSTCONDITION_FAILED, "blur postcondition failed: target remained document.activeElement");
    }
    return {
      before_active_element: beforeActive,
      after_active_element: afterActive,
      active_element_is_target: false
    };
  }

  function performSelectText(element, events) {
    const beforeSelection = selectionSummary(element);
    let readback;
    if (isSelectableValueControl(element)) {
      try {
        if (typeof element.focus === "function") {
          element.focus({ preventScroll: true });
          events.push("focus");
        }
      } catch (_) {
        // select() may still work for some controls.
      }
      try {
        element.select();
      } catch (error) {
        throw actionError(ERROR_ACTION_UNSUPPORTED, `selectText value-control select() failed: ${errorMessageLocal(error)}`);
      }
      element.dispatchEvent(new Event("select", { bubbles: true }));
      events.push("select");
      element.ownerDocument.dispatchEvent(new Event("selectionchange", { bubbles: true }));
      events.push("selectionchange");
      const value = elementTextValue(element);
      const start = typeof element.selectionStart === "number" ? element.selectionStart : null;
      const end = typeof element.selectionEnd === "number" ? element.selectionEnd : null;
      readback = {
        selection_mode: "value_control_select",
        text_len: value.length,
        selection_start: start,
        selection_end: end,
        selected_text: start === null || end === null ? "" : value.slice(start, end)
      };
      if (readback.selected_text !== value) {
        throw actionError(
          ERROR_POSTCONDITION_FAILED,
          `selectText postcondition failed: selected ${readback.selected_text.length} of ${value.length} chars`
        );
      }
    } else {
      const doc = element.ownerDocument;
      const selection = doc.defaultView?.getSelection?.();
      if (!selection) {
        throw actionError(ERROR_ACTION_UNSUPPORTED, "selectText requires document.getSelection()");
      }
      const range = doc.createRange();
      range.selectNodeContents(element);
      selection.removeAllRanges();
      selection.addRange(range);
      doc.dispatchEvent(new Event("selectionchange", { bubbles: true }));
      events.push("selectionchange");
      const expected = String(element.textContent || "");
      readback = {
        selection_mode: "dom_range_select_node_contents",
        text_len: expected.length,
        selection_start: null,
        selection_end: null,
        selected_text: String(selection.toString() || "")
      };
      if (readback.selected_text !== expected) {
        throw actionError(
          ERROR_POSTCONDITION_FAILED,
          `selectText postcondition failed: selected ${readback.selected_text.length} of ${expected.length} chars`
        );
      }
    }
    return {
      before_selection: beforeSelection,
      after_selection: selectionSummary(element),
      ...readback
    };
  }

  function performCheckState(element, desiredChecked, events) {
    const checkable = checkableKind(element);
    if (!checkable) {
      throw actionError(
        ERROR_ACTION_UNSUPPORTED,
        `domAction ${desiredChecked ? "check" : "uncheck"} supports native checkbox/radio inputs only; resolved ${tag(element)} role=${inferRole(element)}`
      );
    }
    if (checkable === "radio" && !desiredChecked) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "domAction uncheck does not support radio inputs; select another radio in the group");
    }
    const beforeChecked = Boolean(element.checked);
    if (beforeChecked === desiredChecked) {
      return {
        desired_checked: desiredChecked,
        before_checked: beforeChecked,
        after_checked: beforeChecked,
        changed: false,
        no_op: true,
        click_dispatched: false,
        checkable_kind: checkable
      };
    }
    scrollIntoViewIfPossible(element);
    if (typeof element.click !== "function") {
      throw actionError(ERROR_ACTION_UNSUPPORTED, `resolved ${tag(element)} element has no click() method`);
    }
    element.click();
    events.push("click");
    const afterChecked = Boolean(element.checked);
    if (afterChecked !== desiredChecked) {
      throw actionError(
        ERROR_POSTCONDITION_FAILED,
        `check state postcondition failed: desired=${desiredChecked} after=${afterChecked}`
      );
    }
    return {
      desired_checked: desiredChecked,
      before_checked: beforeChecked,
      after_checked: afterChecked,
      changed: true,
      no_op: false,
      click_dispatched: true,
      checkable_kind: checkable,
      name_attr: String(element.getAttribute("name") || ""),
      value_attr: String(element.getAttribute("value") || "")
    };
  }

  function createDomEvent(eventType, eventInit) {
    const lower = eventType.toLowerCase();
    if (Object.prototype.hasOwnProperty.call(eventInit, "detail") && typeof CustomEvent === "function") {
      return new CustomEvent(eventType, eventInit);
    }
    if (isPointerEventType(lower) && typeof PointerEvent === "function") {
      return new PointerEvent(eventType, eventInit);
    }
    if (isMouseEventType(lower) && typeof MouseEvent === "function") {
      return new MouseEvent(eventType, eventInit);
    }
    if (isKeyboardEventType(lower) && typeof KeyboardEvent === "function") {
      return new KeyboardEvent(eventType, eventInit);
    }
    if (isInputEventType(lower) && typeof InputEvent === "function") {
      return new InputEvent(eventType, eventInit);
    }
    if (isFocusEventType(lower) && typeof FocusEvent === "function") {
      return new FocusEvent(eventType, eventInit);
    }
    if (lower === "wheel" && typeof WheelEvent === "function") {
      return new WheelEvent(eventType, eventInit);
    }
    return new Event(eventType, eventInit);
  }

  function isPointerEventType(lower) {
    return ["pointerover", "pointerenter", "pointerdown", "pointermove", "pointerup", "pointercancel", "pointerout", "pointerleave", "gotpointercapture", "lostpointercapture"].includes(lower);
  }

  function isMouseEventType(lower) {
    return ["click", "dblclick", "mousedown", "mouseup", "mouseover", "mousemove", "mouseout", "mouseenter", "mouseleave", "contextmenu", "auxclick"].includes(lower);
  }

  function isKeyboardEventType(lower) {
    return ["keydown", "keypress", "keyup"].includes(lower);
  }

  function isInputEventType(lower) {
    return ["beforeinput", "input"].includes(lower);
  }

  function isFocusEventType(lower) {
    return ["focus", "blur", "focusin", "focusout"].includes(lower);
  }

  function performNativeSelect(element, loc, events) {
    if (tag(element) !== "select") {
      throw actionError(
        ERROR_ACTION_UNSUPPORTED,
        `domAction select supports native <select> only; resolved ${tag(element)} role=${inferRole(element)}`
      );
    }
    const specs = selectOptionSpecs(loc);
    if (specs.length === 0) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "domAction select requires option, value, option_label, option_index, or options[]");
    }
    if (specs.length > 1 && !element.multiple) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "domAction select received multiple option specs for a non-multiple <select>");
    }
    const options = Array.from(element.options || []);
    const resolved = specs.map((spec) => resolveSelectOption(options, spec));
    if (element.multiple) {
      for (const option of options) {
        option.selected = false;
      }
      for (const option of resolved) {
        option.selected = true;
      }
    } else {
      for (const option of options) {
        option.selected = false;
      }
      resolved[0].selected = true;
      element.selectedIndex = resolved[0].index;
    }
    element.dispatchEvent(new Event("input", { bubbles: true }));
    events.push("input");
    element.dispatchEvent(new Event("change", { bubbles: true }));
    events.push("change");
    const selectedOptions = Array.from(element.selectedOptions || []).map(optionReadback);
    const requested = specs.map(selectSpecReadback);
    for (const spec of specs) {
      const found = selectedOptions.some((option) => option.index === spec.resolvedIndex);
      if (!found) {
        throw actionError(
          ERROR_POSTCONDITION_FAILED,
          `select postcondition failed: option index ${spec.resolvedIndex} was not selected`
        );
      }
    }
    return {
      multiple: Boolean(element.multiple),
      requested_options: requested,
      selected_options: selectedOptions,
      selected_values: selectedOptions.map((option) => option.value),
      selected_labels: selectedOptions.map((option) => option.label),
      selected_indices: selectedOptions.map((option) => option.index),
      selected_value: String(element.value ?? ""),
      selected_index: element.selectedIndex
    };
  }

  function selectOptionSpecs(loc) {
    const specs = [];
    if (loc.option) {
      specs.push({ kind: "value_or_label", value: loc.option });
    }
    if (loc.value) {
      specs.push({ kind: "value_or_label", value: loc.value });
    }
    if (loc.optionLabel) {
      specs.push({ kind: "label", label: loc.optionLabel });
    }
    if (loc.optionIndex !== null) {
      specs.push({ kind: "index", index: loc.optionIndex });
    }
    for (const raw of loc.options) {
      const value = raw && raw.value !== null && raw.value !== undefined ? String(raw.value) : "";
      const label = raw && raw.label !== null && raw.label !== undefined ? String(raw.label) : "";
      const hasIndex = raw && Number.isSafeInteger(raw.index);
      const present = [value !== "", label !== "", hasIndex].filter(Boolean).length;
      if (present !== 1) {
        throw actionError(ERROR_ACTION_UNSUPPORTED, `select options[] entries must contain exactly one of value, label, or index; got ${JSON.stringify(raw)}`);
      }
      if (value !== "") {
        specs.push({ kind: "value", value });
      } else if (label !== "") {
        specs.push({ kind: "label", label });
      } else {
        specs.push({ kind: "index", index: raw.index });
      }
    }
    return specs;
  }

  function resolveSelectOption(options, spec) {
    let matches = [];
    if (spec.kind === "index") {
      if (!Number.isSafeInteger(spec.index) || spec.index < 0 || spec.index >= options.length) {
        throw actionError(ERROR_ELEMENT_NOT_FOUND, `no <option> matched index ${JSON.stringify(spec.index)}; option_count=${options.length}`);
      }
      const option = options[spec.index];
      spec.resolvedIndex = option.index;
      return option;
    }
    if (spec.kind === "value") {
      matches = options.filter((option) => String(option.value ?? "") === spec.value);
    } else if (spec.kind === "label") {
      matches = options.filter((option) => normalizeText(option.textContent) === normalizeText(spec.label));
    } else {
      const exactValue = options.filter((option) => String(option.value ?? "") === spec.value);
      const exactText = options.filter((option) => normalizeText(option.textContent) === normalizeText(spec.value));
      matches = exactValue.length > 0 ? exactValue : exactText;
    }
    if (matches.length === 0) {
      throw actionError(ERROR_ELEMENT_NOT_FOUND, `no <option> matched ${JSON.stringify(selectSpecReadback(spec))}`);
    }
    if (matches.length > 1) {
      throw actionError(ERROR_ELEMENT_AMBIGUOUS, `${matches.length} <option> elements matched ${JSON.stringify(selectSpecReadback(spec))}`);
    }
    spec.resolvedIndex = matches[0].index;
    return matches[0];
  }

  function optionReadback(option) {
    return {
      value: String(option.value ?? ""),
      label: trimForReadback(option.textContent, 160),
      index: option.index,
      disabled: Boolean(option.disabled)
    };
  }

  function selectSpecReadback(spec) {
    if (spec.kind === "index") {
      return { index: spec.index };
    }
    if (spec.kind === "label") {
      return { label: spec.label };
    }
    return { value: spec.value };
  }

  function performSubmit(element, events) {
    const form = tag(element) === "form" ? element : (element.form || element.closest?.("form"));
    if (!(form instanceof HTMLFormElement)) {
      throw actionError(ERROR_ELEMENT_NOT_FOUND, `resolved ${tag(element)} element is not a form and has no owning form`);
    }
    if (typeof form.requestSubmit !== "function") {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "HTMLFormElement.requestSubmit() is unavailable");
    }
    let submitter = tag(element) === "form" ? null : element;
    if (submitter && !isSubmitter(submitter)) {
      submitter = null;
    }
    if (submitter) {
      form.requestSubmit(submitter);
    } else {
      form.requestSubmit();
    }
    events.push("submit");
    return {
      form_id: String(form.id || ""),
      submitter_tag: submitter ? tag(submitter) : null,
      submitter_id: submitter ? String(submitter.id || "") : null,
      submitter_name: submitter ? String(submitter.getAttribute("name") || "") : null
    };
  }

  function isSubmitter(element) {
    const lower = tag(element);
    if (lower === "button") {
      const type = String(element.getAttribute("type") || "submit").toLowerCase();
      return type === "submit" || type === "";
    }
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "").toLowerCase();
      return type === "submit" || type === "image";
    }
    return false;
  }

  function editableKind(element) {
    if (!(element instanceof Element)) {
      return null;
    }
    const lower = tag(element);
    if (lower === "textarea") {
      return "value";
    }
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "text").toLowerCase();
      const nonText = ["button", "submit", "reset", "checkbox", "radio", "range", "color", "file", "image", "hidden"];
      return nonText.includes(type) ? null : "value";
    }
    if (
      element.isContentEditable ||
      String(element.getAttribute("contenteditable") || "").toLowerCase() === "true" ||
      String(element.getAttribute("role") || "").toLowerCase() === "textbox"
    ) {
      return "contenteditable";
    }
    return null;
  }

  function isSelectableValueControl(element) {
    if (!(element instanceof Element)) {
      return false;
    }
    const lower = tag(element);
    if (!["input", "textarea"].includes(lower) || typeof element.select !== "function" || !("value" in element)) {
      return false;
    }
    if (lower === "textarea") {
      return true;
    }
    const type = String(element.getAttribute("type") || "text").toLowerCase();
    const nonSelectable = ["button", "submit", "reset", "checkbox", "radio", "range", "color", "file", "image", "hidden"];
    return !nonSelectable.includes(type);
  }

  function checkableKind(element) {
    if (!(element instanceof HTMLInputElement)) {
      return null;
    }
    const type = String(element.getAttribute("type") || "").toLowerCase();
    if (type === "checkbox" || type === "radio") {
      return type;
    }
    return null;
  }

  function elementTextValue(element) {
    if ("value" in element) {
      return String(element.value ?? "");
    }
    return String(element.innerText || element.textContent || "");
  }

  function setNativeValue(element, value) {
    const lower = tag(element);
    const proto =
      lower === "input"
        ? window.HTMLInputElement?.prototype
        : lower === "textarea"
          ? window.HTMLTextAreaElement?.prototype
          : null;
    const descriptor = proto ? Object.getOwnPropertyDescriptor(proto, "value") : null;
    if (descriptor && typeof descriptor.set === "function") {
      descriptor.set.call(element, value);
    } else {
      element.value = value;
    }
  }

  function dispatchInputLikeEvent(element, type, data, inputType, cancelable) {
    let event;
    try {
      event = new InputEvent(type, {
        bubbles: true,
        cancelable,
        data,
        inputType
      });
    } catch (_) {
      event = new Event(type, { bubbles: true, cancelable });
    }
    return element.dispatchEvent(event);
  }

  function activeElementSummary(doc, target) {
    const active = doc?.activeElement || null;
    if (!(active instanceof Element)) {
      return {
        has_active_element: false,
        tag_name: "",
        id: "",
        role: "",
        name_attr: "",
        accessible_name: "",
        is_target: false
      };
    }
    return {
      has_active_element: true,
      tag_name: tag(active),
      id: String(active.id || ""),
      role: inferRole(active),
      name_attr: String(active.getAttribute("name") || ""),
      accessible_name: trimForReadback(accessibleName(active), 160),
      is_target: active === target
    };
  }

  function selectionSummary(element) {
    const value = elementTextValue(element);
    if (typeof element.selectionStart === "number" && typeof element.selectionEnd === "number") {
      return {
        selection_mode: "value_control",
        selection_start: element.selectionStart,
        selection_end: element.selectionEnd,
        selected_text: value.slice(element.selectionStart, element.selectionEnd)
      };
    }
    const selection = element.ownerDocument?.defaultView?.getSelection?.();
    return {
      selection_mode: "dom_selection",
      selection_start: null,
      selection_end: null,
      selected_text: selection ? String(selection.toString() || "") : ""
    };
  }

  function accessibleName(element) {
    const labelledBy = stringOrEmpty(element.getAttribute("aria-labelledby"));
    if (labelledBy) {
      const text = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id)?.textContent || "")
        .join(" ")
        .trim();
      if (text) {
        return text;
      }
    }
    for (const attr of ["aria-label", "alt", "title", "placeholder", "value"]) {
      const value = stringOrEmpty(element.getAttribute(attr));
      if (value) {
        return value;
      }
    }
    if (element.id) {
      const label = document.querySelector(`label[for="${cssEscapeLocal(element.id)}"]`);
      if (label?.textContent?.trim()) {
        return label.textContent;
      }
    }
    const wrappingLabel = element.closest?.("label");
    if (wrappingLabel?.textContent?.trim()) {
      return wrappingLabel.textContent;
    }
    return element.textContent || "";
  }

  function inferRole(element) {
    const explicit = stringOrEmpty(element.getAttribute("role")).toLowerCase();
    if (explicit) {
      return explicit;
    }
    const lower = tag(element);
    if (lower === "a" && element.hasAttribute("href")) {
      return "link";
    }
    if (lower === "button") {
      return "button";
    }
    if (lower === "select") {
      return "combobox";
    }
    if (lower === "form") {
      return "form";
    }
    if (lower === "textarea") {
      return "textbox";
    }
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "text").toLowerCase();
      if (["button", "submit", "reset", "image"].includes(type)) {
        return "button";
      }
      if (type === "checkbox") {
        return "checkbox";
      }
      if (type === "radio") {
        return "radio";
      }
      return "textbox";
    }
    return lower || "element";
  }

  function elementSummary(element) {
    if (!(element instanceof Element)) {
      return null;
    }
    const lower = tag(element);
    const value = "value" in element ? String(element.value ?? "") : "";
    return {
      tag_name: lower,
      role: inferRole(element),
      id: String(element.id || ""),
      name_attr: String(element.getAttribute("name") || ""),
      type_attr: String(element.getAttribute("type") || ""),
      accessible_name: trimForReadback(accessibleName(element), 160),
      text: trimForReadback(element.textContent, 160),
      value_len: value.length,
      checked: "checked" in element ? Boolean(element.checked) : null,
      selected_index: lower === "select" ? element.selectedIndex : null,
      visible: isElementVisible(element),
      enabled: isElementEnabled(element)
    };
  }

  function isElementEnabled(element) {
    if (Boolean(element.disabled)) {
      return false;
    }
    return String(element.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
  }

  function isElementVisible(element) {
    if (!(element instanceof Element) || !element.isConnected) {
      return false;
    }
    const style = window.getComputedStyle(element);
    if (!style || style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") {
      return false;
    }
    if (Number(style.opacity) === 0) {
      return false;
    }
    const rects = element.getClientRects();
    return rects && rects.length > 0 && Array.from(rects).some((rect) => rect.width > 0 && rect.height > 0);
  }

  function scrollIntoViewIfPossible(element) {
    try {
      element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
    } catch (_) {
      try {
        element.scrollIntoView();
      } catch (_) {
        // A failed scroll is not by itself proof the element is unclickable.
      }
    }
  }

  function textMatches(actual, expectedNormalized) {
    const actualNormalized = normalizeText(actual);
    return actualNormalized === expectedNormalized || actualNormalized.includes(expectedNormalized);
  }

  function valueMatches(element, expected) {
    const expectedNormalized = normalizeText(expected);
    const values = [
      "value" in element ? String(element.value ?? "") : "",
      element.getAttribute("value") || "",
      element.getAttribute("data-value") || "",
      element.textContent || ""
    ];
    return values.some((value) => normalizeText(value) === expectedNormalized);
  }

  function readPageText(maxChars) {
    const source =
      document.body && typeof document.body.innerText === "string"
        ? document.body.innerText
        : document.documentElement && typeof document.documentElement.innerText === "string"
          ? document.documentElement.innerText
          : document.body && typeof document.body.textContent === "string"
            ? document.body.textContent
            : document.documentElement && typeof document.documentElement.textContent === "string"
              ? document.documentElement.textContent
              : "";
    let text = "";
    let textLen = 0;
    for (const ch of String(source || "")) {
      if (textLen < maxChars) {
        text += ch;
      }
      textLen += 1;
    }
    return {
      text,
      text_len: textLen,
      text_truncated: textLen > maxChars,
      max_chars: maxChars
    };
  }

  function locatorDebug(loc) {
    return JSON.stringify({
      selector: loc.selector || null,
      elementId: loc.elementId || null,
      role: loc.role || null,
      name: loc.name || null,
      value: loc.value || null,
      option: loc.option || null,
      optionLabel: loc.optionLabel || null,
      optionIndex: loc.optionIndex,
      options_count: Array.isArray(loc.options) ? loc.options.length : 0
    });
  }

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function stringOrEmpty(value) {
    return value === null || value === undefined ? "" : String(value).trim();
  }

  function plainObjectOrEmpty(value) {
    if (value === null || value === undefined) {
      return {};
    }
    if (typeof value !== "object" || Array.isArray(value)) {
      return {};
    }
    const out = {};
    for (const [key, val] of Object.entries(value)) {
      out[key] = val;
    }
    return out;
  }

  function trimForReadback(value, maxChars) {
    const text = String(value || "").replace(/\s+/g, " ").trim();
    return [...text].slice(0, maxChars).join("");
  }

  function tag(element) {
    return String(element?.tagName || "").toLowerCase();
  }

  function cssEscapeLocal(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return String(value).replace(/["\\]/g, "\\$&");
  }

  function actionError(code, detail) {
    const error = new Error(detail);
    error.code = code;
    return error;
  }

  function errorMessageLocal(error) {
    return error && error.message ? String(error.message) : String(error);
  }

  function fail(errorCode, errorDetail, extra = {}) {
    return {
      ok: false,
      error_code: errorCode,
      error_detail: errorDetail,
      ...extra
    };
  }
}

function performCoordinateClickInPage(request) {
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const ERROR_ELEMENT_NOT_ACTIONABLE = "CHROME_DOM_ELEMENT_NOT_ACTIONABLE";
  const ERROR_ACTION_UNSUPPORTED = "CHROME_DOM_ACTION_UNSUPPORTED";

  const x = Number(request?.x);
  const y = Number(request?.y);
  const coordinateSpace = String(request?.coordinateSpace || "screen").toLowerCase();
  const clicks = Number.isSafeInteger(request?.clicks) ? Math.min(Math.max(request.clicks, 1), 3) : 1;
  const button = normalizeMouseButtonLocal(request?.button);
  const modifiers = normalizeClickModifiersLocal(request?.modifiers);
  const maxPageTextChars = Number.isSafeInteger(request?.maxPageTextChars)
    ? Math.min(Math.max(request.maxPageTextChars, 0), 65536)
    : 4096;

  if (!Number.isSafeInteger(x) || !Number.isSafeInteger(y)) {
    return fail(ERROR_ACTION_UNSUPPORTED, `coordinateClick requires integer x/y, got (${JSON.stringify(request?.x)}, ${JSON.stringify(request?.y)})`);
  }
  if (!["screen", "window", "viewport"].includes(coordinateSpace)) {
    return fail(ERROR_ACTION_UNSUPPORTED, `unsupported coordinate space ${JSON.stringify(coordinateSpace)}`);
  }

  const beforeUrl = String(location.href || "");
  const beforePageText = readPageTextLocal(maxPageTextChars);
  const beforeActiveElement = readActiveElementLocal();
  const viewportPoint = toViewportPoint(x, y, coordinateSpace);
  const viewportReadback = {
    inner_width: Math.trunc(window.innerWidth || 0),
    inner_height: Math.trunc(window.innerHeight || 0),
    outer_width: Math.trunc(window.outerWidth || 0),
    outer_height: Math.trunc(window.outerHeight || 0),
    screen_x: Math.trunc(window.screenX || 0),
    screen_y: Math.trunc(window.screenY || 0),
    device_pixel_ratio: Number(window.devicePixelRatio || 1),
    visual_viewport_offset_left: Number(window.visualViewport?.offsetLeft || 0),
    visual_viewport_offset_top: Number(window.visualViewport?.offsetTop || 0),
    visual_viewport_scale: Number(window.visualViewport?.scale || 1)
  };

  if (
    viewportPoint.client_x < 0 ||
    viewportPoint.client_y < 0 ||
    viewportPoint.client_x >= window.innerWidth ||
    viewportPoint.client_y >= window.innerHeight
  ) {
    return fail(
      ERROR_ELEMENT_NOT_FOUND,
      `coordinate (${x}, ${y}) in ${coordinateSpace} space maps outside viewport (${viewportPoint.client_x}, ${viewportPoint.client_y}) for inner size ${window.innerWidth}x${window.innerHeight}`,
      {
        coordinate_space: coordinateSpace,
        input_coordinate: { x, y },
        viewport_coordinate: viewportPoint,
        viewport_readback: viewportReadback,
        before_active_element: beforeActiveElement,
        in_page_before_text: beforePageText
      }
    );
  }

  const element = document.elementFromPoint(viewportPoint.client_x, viewportPoint.client_y);
  if (!(element instanceof Element)) {
    return fail(ERROR_ELEMENT_NOT_FOUND, "document.elementFromPoint returned no element at the requested coordinate", {
      coordinate_space: coordinateSpace,
      input_coordinate: { x, y },
      viewport_coordinate: viewportPoint,
      viewport_readback: viewportReadback,
      before_active_element: beforeActiveElement,
      in_page_before_text: beforePageText
    });
  }

  const beforeElement = elementSummary(element);
  if (!isElementEnabled(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "coordinate resolved to a disabled or aria-disabled element", {
      coordinate_space: coordinateSpace,
      input_coordinate: { x, y },
      viewport_coordinate: viewportPoint,
      viewport_readback: viewportReadback,
      before_element: beforeElement,
      before_active_element: beforeActiveElement,
      in_page_before_text: beforePageText
    });
  }
  if (!isElementVisible(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "coordinate resolved to a non-visible element", {
      coordinate_space: coordinateSpace,
      input_coordinate: { x, y },
      viewport_coordinate: viewportPoint,
      viewport_readback: viewportReadback,
      before_element: beforeElement,
      before_active_element: beforeActiveElement,
      in_page_before_text: beforePageText
    });
  }

  const eventsDispatched = [];
  try {
    if (typeof element.focus === "function") {
      element.focus({ preventScroll: true });
      eventsDispatched.push("focus");
    }
  } catch (_) {
    // Some elements reject focus; the click event may still be actionable.
  }
  for (let index = 0; index < clicks; index += 1) {
    const detail = index + 1;
    dispatchPointerLikeEvent(element, "pointerdown", viewportPoint, button, modifiers, detail, true);
    eventsDispatched.push("pointerdown");
    dispatchMouseLikeEvent(element, "mousedown", viewportPoint, button, modifiers, detail, true);
    eventsDispatched.push("mousedown");
    dispatchPointerLikeEvent(element, "pointerup", viewportPoint, button, modifiers, detail, false);
    eventsDispatched.push("pointerup");
    dispatchMouseLikeEvent(element, "mouseup", viewportPoint, button, modifiers, detail, false);
    eventsDispatched.push("mouseup");
    const activationEvent = activationEventForButton(button);
    dispatchMouseLikeEvent(element, activationEvent, viewportPoint, button, modifiers, detail, false);
    eventsDispatched.push(activationEvent);
    if (button === "left" && detail === 2 && clicks >= 2) {
      dispatchMouseLikeEvent(element, "dblclick", viewportPoint, button, modifiers, detail, false);
      eventsDispatched.push("dblclick");
    }
  }

  const afterElement = element.isConnected ? elementSummary(element) : null;
  const afterActiveElement = readActiveElementLocal();
  const afterPageText = readPageTextLocal(maxPageTextChars);
  return {
    ok: true,
    action: "coordinate_click",
    coordinate_space: coordinateSpace,
    input_coordinate: { x, y },
    viewport_coordinate: viewportPoint,
    viewport_readback: viewportReadback,
    click_count: clicks,
    button,
    modifiers,
    before_element: beforeElement,
    after_element: afterElement,
    before_active_element: beforeActiveElement,
    after_active_element: afterActiveElement,
    events_dispatched: eventsDispatched,
    in_page_before_url: beforeUrl,
    in_page_after_url: String(location.href || ""),
    in_page_title: String(document.title || ""),
    in_page_ready_state: String(document.readyState || ""),
    in_page_before_text: beforePageText,
    in_page_after_text: afterPageText
  };

  function toViewportPoint(inputX, inputY, space) {
    if (space === "viewport") {
      return {
        client_x: inputX,
        client_y: inputY,
        conversion: "identity_viewport"
      };
    }
    const horizontalChrome = Math.max(0, (Number(window.outerWidth || 0) - Number(window.innerWidth || 0)) / 2);
    const verticalChrome = Math.max(0, Number(window.outerHeight || 0) - Number(window.innerHeight || 0) - horizontalChrome);
    if (space === "window") {
      return {
        client_x: Math.round(inputX - horizontalChrome),
        client_y: Math.round(inputY - verticalChrome),
        conversion: "window_outer_to_viewport",
        horizontal_chrome_px: horizontalChrome,
        vertical_chrome_px: verticalChrome
      };
    }
    return {
      client_x: Math.round(inputX - Number(window.screenX || 0) - horizontalChrome),
      client_y: Math.round(inputY - Number(window.screenY || 0) - verticalChrome),
      conversion: "screen_to_viewport",
      horizontal_chrome_px: horizontalChrome,
      vertical_chrome_px: verticalChrome
    };
  }

  function dispatchPointerLikeEvent(element, eventType, point, button, modifiers, detail, pressed) {
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(new PointerEvent(eventType, pointerEventInit(point, button, modifiers, detail, pressed)));
      return;
    }
    dispatchMouseLikeEvent(element, eventType.replace(/^pointer/, "mouse"), point, button, modifiers, detail, pressed);
  }

  function dispatchMouseLikeEvent(element, eventType, point, button, modifiers, detail, pressed) {
    element.dispatchEvent(new MouseEvent(eventType, mouseEventInit(point, button, modifiers, detail, pressed)));
  }

  function pointerEventInit(point, button, modifiers, detail, pressed) {
    return {
      ...mouseEventInit(point, button, modifiers, detail, pressed),
      pointerId: 1,
      pointerType: "mouse",
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: eventPressure(pressed)
    };
  }

  function mouseEventInit(point, button, modifiers, detail, pressed) {
    return {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      detail,
      button: mouseButtonCode(button),
      buttons: pressed ? mouseButtonBits(button) : 0,
      clientX: point.client_x,
      clientY: point.client_y,
      screenX: coordinateSpace === "screen" ? x : Math.round(Number(window.screenX || 0) + point.client_x),
      screenY: coordinateSpace === "screen" ? y : Math.round(Number(window.screenY || 0) + point.client_y),
      ...modifierFlags(modifiers)
    };
  }

  function eventPressure(pressed) {
    return pressed ? 0.5 : 0;
  }

  function activationEventForButton(button) {
    if (button === "right") {
      return "contextmenu";
    }
    if (button === "middle") {
      return "auxclick";
    }
    return "click";
  }

  function mouseButtonCode(button) {
    if (button === "middle") {
      return 1;
    }
    if (button === "right") {
      return 2;
    }
    return 0;
  }

  function mouseButtonBits(button) {
    if (button === "middle") {
      return 4;
    }
    if (button === "right") {
      return 2;
    }
    return 1;
  }

  function modifierFlags(modifiers) {
    const set = new Set(Array.isArray(modifiers) ? modifiers : []);
    return {
      altKey: set.has("alt"),
      ctrlKey: set.has("ctrl"),
      metaKey: set.has("meta"),
      shiftKey: set.has("shift")
    };
  }

  function normalizeMouseButtonLocal(value) {
    const normalized = String(value || "left").trim().toLowerCase();
    return ["left", "right", "middle"].includes(normalized) ? normalized : "left";
  }

  function normalizeClickModifiersLocal(value) {
    if (!Array.isArray(value)) {
      return [];
    }
    const seen = new Set();
    for (const item of value) {
      const normalized = String(item || "").trim().toLowerCase();
      if (normalized === "ctrl" || normalized === "control") {
        seen.add("ctrl");
      } else if (normalized === "shift") {
        seen.add("shift");
      } else if (normalized === "alt" || normalized === "option") {
        seen.add("alt");
      } else if (["meta", "super", "cmd", "command"].includes(normalized)) {
        seen.add("meta");
      }
    }
    return ["alt", "ctrl", "meta", "shift"].filter((modifier) => seen.has(modifier));
  }

  function elementSummary(element) {
    if (!(element instanceof Element)) {
      return null;
    }
    const lower = tag(element);
    const value = "value" in element ? String(element.value ?? "") : "";
    const rect = element.getBoundingClientRect();
    return {
      tag_name: lower,
      role: inferRole(element),
      id: String(element.id || ""),
      name_attr: String(element.getAttribute("name") || ""),
      type_attr: String(element.getAttribute("type") || ""),
      accessible_name: trimForReadback(accessibleName(element), 160),
      text: trimForReadback(element.textContent, 160),
      value_len: value.length,
      checked: "checked" in element ? Boolean(element.checked) : null,
      selected_index: lower === "select" ? element.selectedIndex : null,
      visible: isElementVisible(element),
      enabled: isElementEnabled(element),
      bbox: {
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        w: Math.round(rect.width),
        h: Math.round(rect.height)
      }
    };
  }

  function readActiveElementLocal() {
    return elementSummary(document.activeElement);
  }

  function readPageTextLocal(maxChars) {
    const source = document.body?.innerText || document.documentElement?.innerText || "";
    return trimForReadback(source, maxChars);
  }

  function isElementEnabled(element) {
    if (Boolean(element.disabled)) {
      return false;
    }
    return String(element.getAttribute("aria-disabled") || "").toLowerCase() !== "true";
  }

  function isElementVisible(element) {
    if (!(element instanceof Element) || !element.isConnected) {
      return false;
    }
    const style = window.getComputedStyle(element);
    if (!style || style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") {
      return false;
    }
    if (Number(style.opacity) === 0) {
      return false;
    }
    const rects = element.getClientRects();
    return rects && rects.length > 0 && Array.from(rects).some((rect) => rect.width > 0 && rect.height > 0);
  }

  function accessibleName(element) {
    const labelledBy = stringOrEmpty(element.getAttribute("aria-labelledby"));
    if (labelledBy) {
      const text = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id)?.textContent || "")
        .join(" ")
        .trim();
      if (text) {
        return text;
      }
    }
    for (const attr of ["aria-label", "alt", "title", "placeholder", "value"]) {
      const value = stringOrEmpty(element.getAttribute(attr));
      if (value) {
        return value;
      }
    }
    if (element.id) {
      const label = document.querySelector(`label[for="${cssEscapeLocal(element.id)}"]`);
      if (label?.textContent?.trim()) {
        return label.textContent;
      }
    }
    const wrappingLabel = element.closest?.("label");
    if (wrappingLabel?.textContent?.trim()) {
      return wrappingLabel.textContent;
    }
    return element.textContent || "";
  }

  function inferRole(element) {
    const explicit = stringOrEmpty(element.getAttribute("role")).toLowerCase();
    if (explicit) {
      return explicit;
    }
    const lower = tag(element);
    if (lower === "a" && element.hasAttribute("href")) {
      return "link";
    }
    if (lower === "button") {
      return "button";
    }
    if (lower === "select") {
      return "combobox";
    }
    if (lower === "form") {
      return "form";
    }
    if (lower === "textarea") {
      return "textbox";
    }
    if (lower === "input") {
      const type = String(element.getAttribute("type") || "text").toLowerCase();
      if (["button", "submit", "reset", "image"].includes(type)) {
        return "button";
      }
      if (type === "checkbox") {
        return "checkbox";
      }
      if (type === "radio") {
        return "radio";
      }
      return "textbox";
    }
    return lower || "element";
  }

  function normalizeText(value) {
    return String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
  }

  function stringOrEmpty(value) {
    return value === null || value === undefined ? "" : String(value).trim();
  }

  function trimForReadback(value, maxChars) {
    const text = String(value || "").replace(/\s+/g, " ").trim();
    return [...text].slice(0, maxChars).join("");
  }

  function tag(element) {
    return String(element?.tagName || "").toLowerCase();
  }

  function cssEscapeLocal(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return String(value).replace(/["\\]/g, "\\$&");
  }

  function fail(errorCode, errorDetail, extra = {}) {
    return {
      ok: false,
      error_code: errorCode,
      error_detail: errorDetail,
      ...extra
    };
  }
}

function tabsNavigationMethod(action) {
  if (action === "navigate") {
    return "update";
  }
  if (action === "reload") {
    return "reload";
  }
  if (action === "back") {
    return "goBack";
  }
  if (action === "forward") {
    return "goForward";
  }
  return String(action);
}

function normalizeNavigateAction(action) {
  const normalized = String(action || "").toLowerCase();
  if (["navigate", "reload", "back", "forward"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported navigation action ${String(action)}`);
}

function normalizeOptionalSessionId(value) {
  const sessionId = String(value || "").trim();
  return sessionId || null;
}

function markAgentNavigation(tabId, claim) {
  if (!Number.isInteger(tabId) || !claim?.sessionId) {
    return;
  }
  agentNavigationClaims.set(tabId, {
    action: String(claim.action || ""),
    requestedUrl: claim.requestedUrl === null ? null : String(claim.requestedUrl || ""),
    sessionId: claim.sessionId,
    at: Date.now()
  });
}

function matchingAgentNavigationClaim(tabId, url, status) {
  const claim = agentNavigationClaims.get(tabId);
  if (!claim) {
    return null;
  }
  const ageMs = Date.now() - claim.at;
  if (ageMs > AGENT_NAVIGATION_CLAIM_TTL_MS) {
    agentNavigationClaims.delete(tabId);
    return null;
  }
  const requestedUrl = claim.requestedUrl || "";
  const matchesUrl = !requestedUrl || !url || url === requestedUrl || url.startsWith(requestedUrl) || requestedUrl.startsWith(url);
  if (!matchesUrl) {
    return null;
  }
  if (status === "complete") {
    agentNavigationClaims.delete(tabId);
  }
  return claim;
}

function ensurePageEventBuffer(tabId, state = {}) {
  let buffer = pageEventBuffers.get(tabId);
  if (buffer) {
    updatePageSnapshot(buffer, state);
    return { buffer, newlyArmed: false };
  }
  const now = Date.now();
  const targetId = state.target_id || targetIdForTabId(tabId);
  buffer = {
    tabId,
    targetId,
    capacity: MAX_PAGE_EVENT_BUFFER,
    entries: [],
    pages: new Map(),
    workers: new Map(),
    nextSeq: 0,
    dropped: 0,
    armedAtUnixMs: now
  };
  pageEventBuffers.set(tabId, buffer);
  updatePageSnapshot(buffer, state);
  return { buffer, newlyArmed: true };
}

function updatePageSnapshot(buffer, state = {}) {
  if (!buffer) {
    return;
  }
  const now = Date.now();
  const targetId = buffer.targetId || state.target_id || targetIdForTabId(buffer.tabId);
  const existing = buffer.pages.get(targetId);
  const snapshot = {
    target_id: targetId,
    target_type: "page",
    url: String(state.url || existing?.url || ""),
    title: String(state.title || existing?.title || ""),
    opener_id: existing?.opener_id || null,
    opener_frame_id: existing?.opener_frame_id || null,
    can_access_opener: Boolean(existing?.can_access_opener),
    browser_context_id: existing?.browser_context_id || null,
    subtype: existing?.subtype || null,
    attached: false,
    destroyed: false,
    first_seen_seq: Number.isSafeInteger(existing?.first_seen_seq) ? existing.first_seen_seq : buffer.nextSeq,
    last_seen_seq: buffer.nextSeq,
    first_seen_unix_ms: Number.isSafeInteger(existing?.first_seen_unix_ms) ? existing.first_seen_unix_ms : now,
    last_seen_unix_ms: now
  };
  buffer.pages.set(targetId, snapshot);
}

function pushPageEvent(tabId, entry) {
  const buffer = pageEventBuffers.get(tabId);
  if (!buffer || !entry || typeof entry !== "object") {
    return null;
  }
  const now = Date.now();
  const event = {
    seq: buffer.nextSeq,
    event_kind: String(entry.event_kind || ""),
    target_id: String(entry.target_id || buffer.targetId || targetIdForTabId(tabId)),
    target_type: entry.target_type == null ? null : String(entry.target_type),
    target_attached: entry.target_attached == null ? null : Boolean(entry.target_attached),
    page_target_id: entry.page_target_id == null ? null : String(entry.page_target_id),
    opener_id: entry.opener_id == null ? null : String(entry.opener_id),
    opener_frame_id: entry.opener_frame_id == null ? null : String(entry.opener_frame_id),
    can_access_opener: entry.can_access_opener == null ? null : Boolean(entry.can_access_opener),
    browser_context_id: entry.browser_context_id == null ? null : String(entry.browser_context_id),
    subtype: entry.subtype == null ? null : String(entry.subtype),
    worker_id: entry.worker_id == null ? null : String(entry.worker_id),
    worker_type: entry.worker_type == null ? null : String(entry.worker_type),
    worker_url: entry.worker_url == null ? null : String(entry.worker_url),
    frame_id: entry.frame_id == null ? null : String(entry.frame_id),
    parent_frame_id: entry.parent_frame_id == null ? null : String(entry.parent_frame_id),
    loader_id: entry.loader_id == null ? null : String(entry.loader_id),
    name: entry.name == null ? null : String(entry.name),
    url: entry.url == null ? null : String(entry.url),
    title: entry.title == null ? null : String(entry.title),
    navigation_type: entry.navigation_type == null ? null : String(entry.navigation_type),
    timestamp_s: Number.isFinite(Number(entry.timestamp_s)) ? Number(entry.timestamp_s) : null,
    observed_at_unix_ms: Number.isSafeInteger(entry.observed_at_unix_ms)
      ? entry.observed_at_unix_ms
      : now
  };
  if (!event.event_kind) {
    return null;
  }
  buffer.nextSeq += 1;
  if (event.page_target_id) {
    updateBufferedPageFromEvent(buffer, event);
  } else if (event.event_kind === "framenavigated" || event.event_kind === "load" || event.event_kind === "domcontentloaded") {
    updatePageSnapshot(buffer, {
      target_id: event.target_id,
      url: event.url || "",
      title: event.title || ""
    });
  }
  if (event.worker_id) {
    updateBufferedWorkerFromEvent(buffer, event);
  }
  while (buffer.entries.length >= buffer.capacity) {
    buffer.entries.shift();
    buffer.dropped += 1;
  }
  buffer.entries.push(event);
  return event;
}

function updateBufferedPageFromEvent(buffer, event) {
  const id = event.page_target_id;
  const existing = buffer.pages.get(id);
  const page = {
    target_id: id,
    target_type: event.target_type || existing?.target_type || "page",
    url: event.url || existing?.url || "",
    title: event.title || existing?.title || "",
    opener_id: event.opener_id || existing?.opener_id || null,
    opener_frame_id: event.opener_frame_id || existing?.opener_frame_id || null,
    can_access_opener: event.can_access_opener == null
      ? Boolean(existing?.can_access_opener)
      : Boolean(event.can_access_opener),
    browser_context_id: event.browser_context_id || existing?.browser_context_id || null,
    subtype: event.subtype || existing?.subtype || null,
    attached: event.target_attached == null ? Boolean(existing?.attached) : Boolean(event.target_attached),
    destroyed: event.event_kind === "page_destroyed" || Boolean(existing?.destroyed),
    first_seen_seq: Number.isSafeInteger(existing?.first_seen_seq) ? existing.first_seen_seq : event.seq,
    last_seen_seq: event.seq,
    first_seen_unix_ms: Number.isSafeInteger(existing?.first_seen_unix_ms) ? existing.first_seen_unix_ms : event.observed_at_unix_ms,
    last_seen_unix_ms: event.observed_at_unix_ms
  };
  if (event.event_kind === "page_attached") {
    page.attached = true;
  }
  if (event.event_kind === "page_destroyed") {
    page.attached = false;
  }
  buffer.pages.set(id, page);
}

function updateBufferedWorkerFromEvent(buffer, event) {
  const id = event.worker_id;
  const existing = buffer.workers.get(id);
  const worker = {
    worker_id: id,
    worker_type: event.worker_type || existing?.worker_type || "",
    url: event.worker_url || existing?.url || "",
    title: event.title || existing?.title || "",
    attached: event.event_kind !== "worker_destroyed",
    destroyed: event.event_kind === "worker_destroyed" || Boolean(existing?.destroyed),
    first_seen_seq: Number.isSafeInteger(existing?.first_seen_seq) ? existing.first_seen_seq : event.seq,
    last_seen_seq: event.seq,
    first_seen_unix_ms: Number.isSafeInteger(existing?.first_seen_unix_ms) ? existing.first_seen_unix_ms : event.observed_at_unix_ms,
    last_seen_unix_ms: event.observed_at_unix_ms
  };
  if (event.event_kind === "worker_destroyed") {
    worker.attached = false;
  }
  buffer.workers.set(id, worker);
}

function readPageEventBuffer(buffer, filters) {
  const entries = buffer.entries
    .filter((entry) => filters.sinceSeq == null || entry.seq >= filters.sinceSeq)
    .filter((entry) => !filters.eventKind || String(entry.event_kind).toLowerCase() === filters.eventKind)
    .filter((entry) => !filters.workerType || String(entry.worker_type || "").toLowerCase() === filters.workerType)
    .slice(0, filters.limit);
  const pages = Array.from(buffer.pages.values()).sort((left, right) =>
    String(left.target_id).localeCompare(String(right.target_id))
  );
  const workers = Array.from(buffer.workers.values()).sort((left, right) =>
    String(left.worker_id).localeCompare(String(right.worker_id))
  );
  return {
    entries,
    pages,
    workers,
    next_cursor: buffer.nextSeq,
    returned: entries.length,
    total_buffered: buffer.entries.length,
    dropped: buffer.dropped
  };
}

function updatePageEventTabState(tabId, tab, changeInfo = {}) {
  const buffer = pageEventBuffers.get(tabId);
  if (!tab) {
    return;
  }
  if (buffer) {
    updatePageSnapshot(buffer, tabPageStateFromTab(tab));
    if (changeInfo.title) {
      pushPageEvent(tabId, {
        event_kind: "page_info_changed",
        target_id: targetIdForTabId(tabId),
        page_target_id: targetIdForTabId(tabId),
        target_type: "page",
        target_attached: false,
        url: String(tab.pendingUrl || tab.url || ""),
        title: String(tab.title || ""),
        observed_at_unix_ms: Date.now()
      });
    }
  }
  updateRelatedPageEventTabState(tabId, tab, changeInfo);
}

function updateRelatedPageEventTabState(tabId, tab, changeInfo = {}) {
  const targetId = targetIdForTabId(tabId);
  if (!changeInfo?.url && !changeInfo?.title && changeInfo?.status !== "complete") {
    return;
  }
  for (const [ownerTabId, buffer] of pageEventBuffers.entries()) {
    if (ownerTabId === tabId || !buffer.pages.has(targetId)) {
      continue;
    }
    pushPageEvent(ownerTabId, {
      event_kind: "page_info_changed",
      target_id: buffer.targetId,
      page_target_id: targetId,
      target_type: "page",
      target_attached: false,
      url: String(tab.pendingUrl || tab.url || ""),
      title: String(tab.title || ""),
      observed_at_unix_ms: Date.now()
    });
  }
}

function recordTabCreatedForPageEvents(tab) {
  if (!tab || !Number.isInteger(tab.id) || !Number.isInteger(tab.openerTabId)) {
    return;
  }
  const openerBuffer = pageEventBuffers.get(tab.openerTabId);
  if (!openerBuffer) {
    return;
  }
  pushPageEvent(tab.openerTabId, {
    event_kind: "page_created",
    target_id: openerBuffer.targetId,
    target_type: "page",
    target_attached: false,
    page_target_id: targetIdForTabId(tab.id),
    opener_id: openerBuffer.targetId,
    can_access_opener: true,
    url: String(tab.pendingUrl || tab.url || ""),
    title: String(tab.title || ""),
    observed_at_unix_ms: Date.now()
  });
}

function recordTabRemovedForPageEvents(tabId) {
  for (const [ownerTabId, buffer] of pageEventBuffers.entries()) {
    const removedTargetId = targetIdForTabId(tabId);
    if (!buffer.pages.has(removedTargetId)) {
      continue;
    }
    pushPageEvent(ownerTabId, {
      event_kind: "page_destroyed",
      target_id: buffer.targetId,
      page_target_id: removedTargetId,
      observed_at_unix_ms: Date.now()
    });
  }
  pageEventBuffers.delete(tabId);
}

function recordWebNavigationPageEvent(eventKind, details, extra = {}) {
  if (!details || !Number.isInteger(details.tabId) || details.tabId < 0) {
    return;
  }
  const buffer = pageEventBuffers.get(details.tabId);
  if (!buffer) {
    return;
  }
  const frameId = Number.isSafeInteger(details.frameId) ? String(details.frameId) : null;
  const parentFrameId = Number.isSafeInteger(details.parentFrameId) && details.parentFrameId >= 0
    ? String(details.parentFrameId)
    : null;
  pushPageEvent(details.tabId, {
    event_kind: eventKind,
    target_id: buffer.targetId || targetIdForTabId(details.tabId),
    frame_id: frameId,
    parent_frame_id: parentFrameId,
    loader_id: details.documentId == null ? null : String(details.documentId),
    url: String(details.url || ""),
    navigation_type: extra.navigation_type || null,
    timestamp_s: Number.isFinite(details.timeStamp) ? details.timeStamp / 1000 : null,
    observed_at_unix_ms: Number.isFinite(details.timeStamp) ? Math.floor(details.timeStamp) : Date.now()
  });
}

function webNavigationTransition(details) {
  const transitionType = String(details?.transitionType || "");
  const qualifiers = Array.isArray(details?.transitionQualifiers)
    ? details.transitionQualifiers.map(String).sort()
    : [];
  if (!transitionType && qualifiers.length === 0) {
    return "";
  }
  return [transitionType, ...qualifiers].filter(Boolean).join("+");
}

async function postTabNavigationEvent(source, tab) {
  if (!tab || typeof tab.id !== "number") {
    return;
  }
  const url = String(tab.pendingUrl || tab.url || "");
  if (!url) {
    return;
  }
  const title = String(tab.title || "");
  const status = String(tab.status || "");
  const agentClaim = matchingAgentNavigationClaim(tab.id, url, status);
  if (agentClaim) {
    return;
  }
  const key = `${tab.id}\n${url}\n${title}\n${status}`;
  if (recentNavigationKeys.includes(key)) {
    return;
  }
  recentNavigationKeys.push(key);
  while (recentNavigationKeys.length > MAX_RECENT_NAVIGATION_KEYS) {
    recentNavigationKeys.shift();
  }
  await postDaemonMessage({
    type: "event",
    event: "tabNavigation",
    source,
    target_id: targetIdForTabId(tab.id),
    tab_id: tab.id,
    chrome_window_id: Number.isInteger(tab.windowId) ? tab.windowId : null,
    url,
    title,
    status,
    active: Boolean(tab.active),
    highlighted: Boolean(tab.highlighted),
    pinned: Boolean(tab.pinned),
    observed_at_unix_ms: Date.now()
  });
}

function requiredUrl(url) {
  const value = String(url ?? "");
  if (!value || value.trim() !== value || value.includes("\u0000")) {
    throw bridgeError(ERROR_ATTACH_FAILED, "Page.navigate requires a non-empty URL with no surrounding whitespace or NUL");
  }
  if (Array.from(value).length > 8192) {
    throw bridgeError(ERROR_ATTACH_FAILED, "Page.navigate URL must be at most 8192 Unicode scalar values");
  }
  return value;
}

function normalizeOpenUrl(url) {
  const value = String(url ?? "");
  if (!value) {
    return "";
  }
  if (value.trim() !== value || value.includes("\u0000")) {
    throw bridgeError(ERROR_ATTACH_FAILED, "chrome.tabs.create URL must have no surrounding whitespace or NUL");
  }
  if (Array.from(value).length > 8192) {
    throw bridgeError(ERROR_ATTACH_FAILED, "chrome.tabs.create URL must be at most 8192 Unicode scalar values");
  }
  return value;
}

function normalizeWaitTimeout(value) {
  if (value === undefined || value === null) {
    return 10000;
  }
  const number = Number(value);
  if (!Number.isInteger(number) || number < 1 || number > 30000) {
    throw bridgeError(ERROR_ATTACH_FAILED, "waitTimeoutMs must be an integer from 1 through 30000");
  }
  return number;
}

function normalizePollingInterval(value) {
  if (value === undefined || value === null) {
    return 100;
  }
  const number = Number(value);
  if (!Number.isInteger(number) || number < 10 || number > 5000) {
    throw bridgeError(ERROR_ATTACH_FAILED, "pollingIntervalMs must be an integer from 10 through 5000");
  }
  return number;
}

function normalizeWaitForTextState(value) {
  const state = String(value || "text_appears");
  if (state === "text_appears" || state === "text_gone" || state === "timeout") {
    return state;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported waitForText state ${JSON.stringify(state)}`);
}

function normalizeWaitForSelectorState(value) {
  const state = String(value || "visible");
  if (state === "attached" || state === "visible" || state === "hidden" || state === "detached") {
    return state;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported waitForSelector state ${JSON.stringify(state)}`);
}

function normalizeDomActionAutoWaitTimeout(value, enabled) {
  if (!enabled) {
    return 2000;
  }
  if (value === undefined || value === null) {
    return 2000;
  }
  const number = Number(value);
  if (!Number.isInteger(number) || number < 50 || number > 30000) {
    throw bridgeError(ERROR_ATTACH_FAILED, "autoWaitTimeoutMs must be an integer from 50 through 30000");
  }
  return number;
}

function normalizeClockOperation(value) {
  const operation = String(value || "status");
  if (["status", "install", "setFixedTime", "fastForward", "pauseAt"].includes(operation)) {
    return operation;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported clock operation ${JSON.stringify(operation)}`);
}

function normalizePageEventsFilter(params) {
  const sinceSeq = optionalNonNegativeInteger(params.sinceSeq);
  const limit = normalizePageEventsLimit(params.limit);
  const eventKind = normalizeOptionalPageEventKind(params.eventKind);
  const workerType = normalizeOptionalWorkerType(params.workerType);
  return { sinceSeq, limit, eventKind, workerType };
}

function optionalNonNegativeInteger(value) {
  if (value === undefined || value === null || value === "") {
    return null;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "sinceSeq must be a non-negative safe integer");
  }
  return number;
}

function normalizePageEventsLimit(value) {
  if (value === undefined || value === null) {
    return 100;
  }
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number < 1 || number > MAX_PAGE_EVENT_BUFFER) {
    throw bridgeError(ERROR_ATTACH_FAILED, `limit must be an integer from 1 through ${MAX_PAGE_EVENT_BUFFER}`);
  }
  return number;
}

function normalizeOptionalPageEventKind(value) {
  const kind = String(value || "").trim().toLowerCase();
  if (!kind) {
    return null;
  }
  if ([
    "domcontentloaded",
    "load",
    "lifecycle",
    "framenavigated",
    "framestartednavigating",
    "framestoppedloading",
    "page_created",
    "page_attached",
    "page_destroyed",
    "page_info_changed",
    "worker_created",
    "worker_attached",
    "worker_destroyed",
    "worker_info_changed"
  ].includes(kind)) {
    return kind;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported page event kind ${JSON.stringify(value)}`);
}

function normalizeOptionalWorkerType(value) {
  const workerType = String(value || "").trim().toLowerCase();
  if (!workerType) {
    return null;
  }
  if (["worker", "service_worker", "shared_worker"].includes(workerType)) {
    return workerType;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported worker type ${JSON.stringify(value)}`);
}

function normalizeMaxContentBytes(value) {
  if (value === undefined || value === null) {
    return 2 * 1024 * 1024;
  }
  const number = Number(value);
  if (!Number.isInteger(number) || number < 0 || number > 2 * 1024 * 1024) {
    throw bridgeError(ERROR_ATTACH_FAILED, "maxBytes must be an integer from 0 through 2097152");
  }
  return number;
}

function normalizeReloadDelay(value) {
  if (value === undefined || value === null) {
    return 100;
  }
  const number = Number(value);
  if (!Number.isInteger(number) || number < 0 || number > 5000) {
    throw bridgeError(ERROR_ATTACH_FAILED, "reloadDelayMs must be an integer from 0 through 5000");
  }
  return number;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function postResponse(id, ok, result, error) {
  await postDaemonMessage({
    type: "response",
    id,
    ok,
    result,
    error
  });
}

async function postDaemonMessage(message) {
  if (!hostId) {
    console.warn("Synapse daemon bridge unavailable; message dropped");
    return;
  }
  try {
    await daemonFetchJson("/chrome-debugger/native/message", {
      method: "POST",
      body: {
        host_id: hostId,
        message
      }
    });
  } catch (error) {
    scheduleReconnect(
      `direct daemon message failed: ${errorMessage(error)}`,
      ERROR_DAEMON_UNAVAILABLE
    );
    throw error;
  }
}

async function daemonFetchJson(path, options = {}) {
  const init = {
    method: options.method || "GET",
    cache: "no-store",
    headers: {}
  };
  if (bridgeToken) {
    init.headers[BRIDGE_TOKEN_HEADER] = bridgeToken;
  }
  if (Object.prototype.hasOwnProperty.call(options, "body")) {
    init.headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(options.body);
  }
  const response = await fetch(`${DAEMON_BASE_URL}${path}`, init);
  const text = await response.text();
  let value = null;
  if (text) {
    try {
      value = JSON.parse(text);
    } catch (_) {
      throw new Error(
        `daemon ${init.method} ${path} returned non-JSON status=${response.status} body=${JSON.stringify(text.slice(0, 512))}`
      );
    }
  }
  if (!response.ok) {
    const error = new Error(
      `daemon ${init.method} ${path} failed status=${response.status} body=${JSON.stringify(value ?? text)}`
    );
    if (value?.code) {
      error.code = value.code;
    }
    error.status = response.status;
    error.responseBody = value ?? text;
    throw error;
  }
  return value;
}

function bridgeError(code, detail) {
  const error = new Error(detail);
  error.code = code;
  return error;
}

function errorPayload(error) {
  return {
    code: error?.code || ERROR_ATTACH_FAILED,
    detail: errorMessage(error)
  };
}

function errorMessage(error) {
  if (!error) {
    return "unknown error";
  }
  if (typeof error === "string") {
    return error;
  }
  return error.message || String(error);
}
