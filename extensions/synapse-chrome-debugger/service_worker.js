const PROTOCOL_VERSION = 1;
const BRIDGE_BUILD_ID = "synapse-chrome-bridge-2026-06-21-check-uncheck-v1";
const BRIDGE_BUILD_SHA256 = "a848cc84f03897822f0c37cb428eae220ca62c57ed47a5bf25c7ff850b2f7dd4";
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
  "domAction",
  "coordinateClick",
  "reloadSelf",
  "typeActiveElement",
  "setFieldValue"
]);
const EXPECTED_EXTENSION_ID = "leoocgnkjnplbfdbklajepahofecgfbk";
const DAEMON_BASE_URL = "http://127.0.0.1:7700";
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
    "navigateTab",
    "activateTab",
    "domAction"
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
  postTabNavigationEvent("tabs.onUpdated", tab).catch((error) => {
    console.warn(`Synapse tab navigation event dropped: ${errorMessage(error)}`);
  });
});
chrome.tabs.onActivated.addListener((activeInfo) => {
  chrome.tabs.get(activeInfo.tabId)
    .then((tab) => postTabNavigationEvent("tabs.onActivated", tab))
    .catch((error) => {
      console.warn(`Synapse active tab navigation readback failed: ${errorMessage(error)}`);
    });
});

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
    clicks: normalizeClickCount(params.clicks),
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
  if (["click", "press", "select", "submit", "dispatch_event", "clear", "focus", "blur", "select_text", "check", "uncheck"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(
    ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
    `domAction action must be one of click, press, select, submit, dispatch_event, clear, focus, blur, select_text, check, uncheck; got ${JSON.stringify(action)}`
  );
}

function stringOrNull(value) {
  if (value === null || value === undefined) {
    return null;
  }
  const text = String(value).trim();
  return text ? text : null;
}

function normalizeClickCount(value) {
  if (value === null || value === undefined) {
    return 1;
  }
  const clicks = Number(value);
  if (!Number.isSafeInteger(clicks) || clicks < 1 || clicks > 3) {
    throw bridgeError(
      ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
      `domAction clicks must be an integer in 1..=3; got ${JSON.stringify(value)}`
    );
  }
  return clicks;
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

function performDomActionInPage(request) {
  const ERROR_SELECTOR_INVALID = "CHROME_DOM_SELECTOR_INVALID";
  const ERROR_ELEMENT_NOT_FOUND = "CHROME_DOM_ELEMENT_NOT_FOUND";
  const ERROR_ELEMENT_AMBIGUOUS = "CHROME_DOM_ELEMENT_AMBIGUOUS";
  const ERROR_ELEMENT_NOT_ACTIONABLE = "CHROME_DOM_ELEMENT_NOT_ACTIONABLE";
  const ERROR_ACTION_UNSUPPORTED = "CHROME_DOM_ACTION_UNSUPPORTED";
  const ERROR_POSTCONDITION_FAILED = "CHROME_DOM_ACTION_POSTCONDITION_FAILED";

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
  const clicks = Number.isSafeInteger(request?.clicks) ? request.clicks : 1;
  const eventType = stringOrEmpty(request?.eventType);
  const eventInit = plainObjectOrEmpty(request?.eventInit);
  const resolveOnly = Boolean(request?.resolveOnly);
  const maxPageTextChars = Number.isSafeInteger(request?.maxPageTextChars)
    ? Math.min(Math.max(request.maxPageTextChars, 0), 65536)
    : 4096;

  if (!["click", "press", "select", "submit", "dispatch_event", "clear", "focus", "blur", "select_text", "check", "uncheck"].includes(action)) {
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
  if (!bypassActionability && !isElementEnabled(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is disabled or aria-disabled", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement
    });
  }
  if (!bypassActionability && !isElementVisible(element) && action !== "submit") {
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
  try {
    if (action === "click" || action === "press") {
      actionReadback = performClick(element, clicks, eventsDispatched);
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
    events_dispatched: eventsDispatched,
    action_readback: actionReadback,
    in_page_before_url: beforeUrl,
    in_page_after_url: String(location.href || ""),
    in_page_title: String(document.title || ""),
    in_page_ready_state: String(document.readyState || ""),
    in_page_before_text: beforePageText,
    in_page_after_text: afterPageText
  };

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

  function performClick(element, count, events) {
    if (typeof element.click !== "function") {
      throw actionError(ERROR_ACTION_UNSUPPORTED, `resolved ${tag(element)} element has no click() method`);
    }
    scrollIntoViewIfPossible(element);
    const bounded = Math.min(Math.max(count, 1), 3);
    for (let index = 0; index < bounded; index += 1) {
      element.click();
      events.push("click");
    }
    return {
      click_count: bounded
    };
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
    dispatchPointerLikeEvent(element, "pointerdown", viewportPoint);
    eventsDispatched.push("pointerdown");
    dispatchMouseLikeEvent(element, "mousedown", viewportPoint);
    eventsDispatched.push("mousedown");
    dispatchPointerLikeEvent(element, "pointerup", viewportPoint);
    eventsDispatched.push("pointerup");
    dispatchMouseLikeEvent(element, "mouseup", viewportPoint);
    eventsDispatched.push("mouseup");
    dispatchMouseLikeEvent(element, "click", viewportPoint);
    eventsDispatched.push("click");
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

  function dispatchPointerLikeEvent(element, eventType, point) {
    if (typeof PointerEvent === "function") {
      element.dispatchEvent(new PointerEvent(eventType, pointerEventInit(point)));
      return;
    }
    dispatchMouseLikeEvent(element, eventType.replace(/^pointer/, "mouse"), point);
  }

  function dispatchMouseLikeEvent(element, eventType, point) {
    element.dispatchEvent(new MouseEvent(eventType, mouseEventInit(point)));
  }

  function pointerEventInit(point) {
    return {
      ...mouseEventInit(point),
      pointerId: 1,
      pointerType: "mouse",
      isPrimary: true,
      width: 1,
      height: 1,
      pressure: eventPressure(point)
    };
  }

  function mouseEventInit(point) {
    return {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: window,
      detail: clicks,
      button: 0,
      buttons: 1,
      clientX: point.client_x,
      clientY: point.client_y,
      screenX: coordinateSpace === "screen" ? x : Math.round(Number(window.screenX || 0) + point.client_x),
      screenY: coordinateSpace === "screen" ? y : Math.round(Number(window.screenY || 0) + point.client_y)
    };
  }

  function eventPressure() {
    return 0.5;
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
