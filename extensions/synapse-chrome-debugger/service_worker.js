const PROTOCOL_VERSION = 1;
const BRIDGE_BUILD_ID = "synapse-chrome-bridge-2026-06-17-screenshot-retry-v1";
const BRIDGE_BUILD_SHA256 = "196f108467bbaf4d40a1768a09eb5cdc838f4b5b12d425d63beb6c139ed67adf";
const COMMAND_CAPABILITIES = Object.freeze([
  "openTab",
  "closeTab",
  "capturePageScreenshot",
  "targetInfo",
  "targetInfoPageText",
  "navigateTab",
  "activateTab",
  "evaluateScript",
  "pageVitals",
  "domAction",
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
const WEBSOCKET_KEEPALIVE_MS = 20000;
const RECONNECT_INITIAL_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const DISCONNECTED_KEEPALIVE_MS = 20000;
const AGENT_NAVIGATION_CLAIM_TTL_MS = 30000;
const MAX_RECENT_NAVIGATION_KEYS = 128;
const MAX_PAGE_TEXT_CHARS = 4096;
const OPEN_WINDOW_BOUNDS_TOLERANCE_PX = 96;
const CAPTURE_SCREENSHOT_ATTACH_TIMEOUT_MS = 5000;
const CAPTURE_SCREENSHOT_COMMAND_TIMEOUT_MS = 15000;
const CAPTURE_SCREENSHOT_MAX_ATTEMPTS = 2;
const CAPTURE_SCREENSHOT_RETRY_DELAY_MS = 150;

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

function startBridge() {
  if (chrome.runtime.id !== EXPECTED_EXTENSION_ID) {
    disableBridgePermanently(
      `extension id mismatch: actual=${chrome.runtime.id} expected=${EXPECTED_EXTENSION_ID}; ` +
        `reload the unpacked extension from the Synapse extension directory so the daemon can ` +
        `authenticate the bridge origin`,
      ERROR_EXTENSION_ID_MISMATCH
    );
    return;
  }
  connectDaemon();
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
  connectInFlight = registerDaemon()
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

chrome.runtime.onInstalled.addListener(startBridge);
chrome.runtime.onStartup.addListener(startBridge);
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

startBridge();

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
    } else if (kind === "closeTab") {
      result = await handleCloseTab(params);
    } else if (kind === "capturePageScreenshot") {
      result = await handleCapturePageScreenshot(params);
    } else if (kind === "targetInfo" || kind === "targetInfoPageText") {
      result = await handleTargetInfo(params);
    } else if (kind === "typeActiveElement") {
      result = await handleTypeActiveElement(params);
    } else if (kind === "setFieldValue") {
      result = await handleSetFieldValue(params);
    } else if (kind === "evaluateScript") {
      result = await handleEvaluateScript(params);
    } else if (kind === "pageVitals") {
      result = await handlePageVitals(params);
    } else if (kind === "navigateTab") {
      result = await handleNavigateTab(params);
    } else if (kind === "activateTab") {
      result = await handleActivateTab(params);
    } else if (kind === "domAction") {
      result = await handleDomAction(params);
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
    capabilities: [...COMMAND_CAPABILITIES],
    commandCapabilities: [...COMMAND_CAPABILITIES]
  };
}

function rejectAttachCommand(kind, params) {
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `normal Synapse Chrome Bridge refused ${String(kind)} before browser debugger startup; ` +
      `the normal end-user extension exposes chrome.debugger only for guarded ` +
      `session-owned capturePageScreenshot, not DOM/action attach commands. ` +
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
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const before = await tabPageState(selected.tabId, selected.target);
  const windowId = before.chrome_window_id || selected.target.chromeWindowId;
  const expectedWindowId = optionalInteger(params.expectedChromeWindowId);
  if (
    Number.isInteger(expectedWindowId) &&
    Number.isInteger(windowId) &&
    windowId !== expectedWindowId
  ) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `capturePageScreenshot refused target ${selected.target.id}: tab is in Chrome window ${windowId}, expected ${expectedWindowId}`
    );
  }
  if (!Number.isInteger(windowId)) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `capturePageScreenshot could not resolve chrome window id for target ${JSON.stringify(selected.target.id)}`
    );
  }
  const captureWindow = await chromeWindowState(windowId);
  if (!chrome.debugger || typeof chrome.debugger.attach !== "function" || typeof chrome.debugger.sendCommand !== "function") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.debugger unavailable; extension is missing debugger permission for Page.captureScreenshot"
    );
  }
  const capture = await capturePageScreenshotWithRetry(selected);
  const screenshot = capture.screenshot;
  const imageData = typeof screenshot?.data === "string" ? screenshot.data : "";
  if (!imageData) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "Page.captureScreenshot returned no image data"
    );
  }
  const imageDataUrl = `data:image/png;base64,${imageData}`;
  const after = await tabPageState(selected.tabId, selected.target);
  return {
    extension_id: chrome.runtime.id,
    target_id: after.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: windowId,
    chrome_window_focused: typeof captureWindow?.focused === "boolean" ? captureWindow.focused : null,
    chrome_window_state: typeof captureWindow?.state === "string" ? captureWindow.state : "",
    url: after.url || "",
    title: after.title || "",
    ready_state: after.ready_state || "",
    before_active: Boolean(before.active),
    active_for_capture: Boolean(after.active),
    before_highlighted: Boolean(before.highlighted),
    highlighted_for_capture: Boolean(after.highlighted),
    previous_active_tab_id: null,
    restored_previous_active: false,
    image_format: "png",
    image_data_url: imageDataUrl,
    image_data_url_len: imageDataUrl.length,
    readback_backend: "chrome.debugger.Page.captureScreenshot+chrome.tabs.get",
    capture_attempt_count: capture.attempts.length,
    capture_attempts: capture.attempts,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function capturePageScreenshotWithRetry(selected) {
  const attempts = [];
  let lastError = null;
  for (let attempt = 1; attempt <= CAPTURE_SCREENSHOT_MAX_ATTEMPTS; attempt += 1) {
    const startedAt = Date.now();
    try {
      const screenshot = await capturePageScreenshotAttempt(selected.tabId);
      attempts.push({
        attempt,
        ok: true,
        elapsed_ms: elapsedSince(startedAt)
      });
      return { screenshot, attempts };
    } catch (error) {
      const detail = errorMessage(error);
      lastError = error;
      attempts.push({
        attempt,
        ok: false,
        elapsed_ms: elapsedSince(startedAt),
        error_detail: detail,
        retryable: isCapturePageScreenshotRetryableError(error)
      });
      if (
        attempt >= CAPTURE_SCREENSHOT_MAX_ATTEMPTS ||
        !isCapturePageScreenshotRetryableError(error)
      ) {
        break;
      }
      await delay(CAPTURE_SCREENSHOT_RETRY_DELAY_MS);
    }
  }
  throw bridgeError(
    ERROR_AXTREE_FAILED,
    `capturePageScreenshot failed for target ${selected.target.id} after ${attempts.length} attempts: ` +
      `${formatCaptureAttempts(attempts)}; last_error=${errorMessage(lastError)}`
  );
}

async function capturePageScreenshotAttempt(tabId) {
  const debuggee = { tabId };
  let attached = false;
  let screenshot;
  let primaryError = null;
  let detachError = null;
  try {
    await withTimeout(
      chrome.debugger.attach(debuggee, "1.3"),
      CAPTURE_SCREENSHOT_ATTACH_TIMEOUT_MS,
      `chrome.debugger.attach(tabId=${tabId})`
    );
    attached = true;
    await withTimeout(
      chrome.debugger.sendCommand(debuggee, "Page.enable", {}),
      CAPTURE_SCREENSHOT_ATTACH_TIMEOUT_MS,
      `chrome.debugger.sendCommand(Page.enable, tabId=${tabId})`
    );
    screenshot = await withTimeout(
      chrome.debugger.sendCommand(debuggee, "Page.captureScreenshot", {
        format: "png",
        fromSurface: true
      }),
      CAPTURE_SCREENSHOT_COMMAND_TIMEOUT_MS,
      `chrome.debugger.sendCommand(Page.captureScreenshot, tabId=${tabId})`
    );
  } catch (error) {
    primaryError = error;
  } finally {
    if (attached) {
      try {
        await withTimeout(
          chrome.debugger.detach(debuggee),
          CAPTURE_SCREENSHOT_ATTACH_TIMEOUT_MS,
          `chrome.debugger.detach(tabId=${tabId})`
        );
      } catch (error) {
        detachError = errorMessage(error);
      }
    }
  }
  if (detachError) {
    const detail = primaryError
      ? `${errorMessage(primaryError)}; detach failed: ${detachError}`
      : `debugger detach failed: ${detachError}`;
    throw new Error(detail);
  }
  if (primaryError) {
    throw primaryError;
  }
  return screenshot;
}

function isCapturePageScreenshotRetryableError(error) {
  const message = errorMessage(error);
  return message.includes("Page.captureScreenshot") && message.includes("timed out after");
}

function formatCaptureAttempts(attempts) {
  return attempts
    .map((attempt) => {
      if (attempt.ok) {
        return `attempt ${attempt.attempt} ok elapsed_ms=${attempt.elapsed_ms}`;
      }
      return `attempt ${attempt.attempt} error retryable=${attempt.retryable} ` +
        `elapsed_ms=${attempt.elapsed_ms} detail=${attempt.error_detail}`;
    })
    .join("; ");
}

async function handleEvaluateScript(params) {
  const selected = await selectTabTarget(params, { requireTargetId: true });
  const expression = String(params.expression ?? "");
  const args = Array.isArray(params.args) ? params.args : [];
  const awaitPromise = params.awaitPromise !== false;
  const returnByValue = params.returnByValue !== false;
  if (!expression.trim()) {
    throw bridgeError(ERROR_ATTACH_FAILED, "evaluateScript requires a non-empty expression");
  }
  const before = await tabPageState(selected.tabId, selected.target);
  const windowId = before.chrome_window_id || selected.target.chromeWindowId;
  if (!Number.isInteger(windowId)) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `evaluateScript could not resolve chrome window id for target ${JSON.stringify(selected.target.id)}`
    );
  }
  if (!chrome.debugger || typeof chrome.debugger.attach !== "function" || typeof chrome.debugger.sendCommand !== "function") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.debugger unavailable; extension is missing debugger permission for Runtime.evaluate"
    );
  }
  let runtimeExpression;
  try {
    runtimeExpression = runtimeEvaluateExpression(expression, args);
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `evaluateScript parameter serialization failed: ${errorMessage(error)}`
    );
  }
  const debuggee = { tabId: selected.tabId };
  let attached = false;
  let evaluated;
  let detachError = null;
  try {
    await withTimeout(
      chrome.debugger.attach(debuggee, "1.3"),
      5000,
      `chrome.debugger.attach(tabId=${selected.tabId})`
    );
    attached = true;
    await withTimeout(
      chrome.debugger.sendCommand(debuggee, "Runtime.enable", {}),
      5000,
      `chrome.debugger.sendCommand(Runtime.enable, tabId=${selected.tabId})`
    );
    evaluated = await withTimeout(
      chrome.debugger.sendCommand(debuggee, "Runtime.evaluate", {
        expression: `${runtimeExpression}\n//# sourceURL=synapse://browser_evaluate`,
        objectGroup: "synapse_browser_evaluate",
        awaitPromise,
        returnByValue,
        allowUnsafeEvalBlockedByCSP: true
      }),
      15000,
      `chrome.debugger.sendCommand(Runtime.evaluate, tabId=${selected.tabId})`
    );
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `Runtime.evaluate failed for target ${selected.target.id}: ${errorMessage(error)}`
    );
  } finally {
    if (attached) {
      try {
        await withTimeout(
          chrome.debugger.detach(debuggee),
          5000,
          `chrome.debugger.detach(tabId=${selected.tabId})`
        );
      } catch (error) {
        detachError = errorMessage(error);
      }
    }
  }
  if (detachError) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `Runtime.evaluate completed for target ${selected.target.id} but failed to detach debugger from tab ${selected.tabId}: ${detachError}`
    );
  }
  if (evaluated?.exceptionDetails) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `Runtime.evaluate threw for target ${selected.target.id}: ${formatRuntimeException(evaluated.exceptionDetails)}`
    );
  }
  if (!evaluated?.result || typeof evaluated.result !== "object") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "Runtime.evaluate returned no remote object result"
    );
  }
  const state = await tabPageState(selected.tabId, selected.target);
  return {
    extension_id: chrome.runtime.id,
    target_id: state.target_id || selected.target.id,
    tab_id: selected.tabId,
    chrome_window_id: state.chrome_window_id,
    url: state.url || "",
    title: state.title || "",
    ready_state: state.ready_state || "",
    scope: "page",
    readback_backend: "chrome.debugger.Runtime.evaluate+chrome.tabs.get",
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...runtimeRemoteObjectResult(evaluated.result, returnByValue)
  };
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
      target: { tabId: selected.tabId },
      func: typeActiveElementInPage,
      args: [text]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript typeActiveElement(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const first = Array.isArray(results) ? results[0] : null;
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
  let injected;
  try {
    injected = await chrome.scripting.executeScript({
      target: { tabId: selected.tabId },
      func: performDomActionInPage,
      args: [{
        action,
        selector: stringOrNull(params.selector),
        elementId: stringOrNull(params.elementId),
        role: stringOrNull(params.role),
        name: stringOrNull(params.name),
        value: stringOrNull(params.value),
        option: stringOrNull(params.option),
        clicks: normalizeClickCount(params.clicks),
        maxPageTextChars: MAX_PAGE_TEXT_CHARS
      }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      `chrome.scripting.executeScript domAction(${selected.tabId}, ${action}) failed: ${errorMessage(error)}`
    );
  }

  const first = Array.isArray(injected) ? injected[0] : null;
  const actionResult = first?.result;
  if (!actionResult || typeof actionResult !== "object") {
    throw bridgeError(
      ERROR_CHROME_SCRIPTING_EXECUTE_FAILED,
      "chrome.scripting.executeScript domAction returned no structured result"
    );
  }
  if (!actionResult.ok) {
    throw bridgeError(
      String(actionResult.error_code || ERROR_AXTREE_FAILED),
      `domAction ${action} failed: ${String(actionResult.error_detail || "")}`
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
  const expectedWindowId = optionalInteger(params.expectedChromeWindowId);
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
      bounds: chromeWindowBounds(windowInfo)
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
        `openTab refused hwnd hint ${String(params.hwnd)}: expected_window_bounds matched ${boundsMatches.length} Chrome windows within ${OPEN_WINDOW_BOUNDS_TOLERANCE_PX}px`
      );
    }
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
    .map((windowInfo) => {
      const score = expectedBounds
        ? windowBoundsDeltaScore(windowInfo.bounds, expectedBounds)
        : Number.POSITIVE_INFINITY;
      const scoreText = Number.isFinite(score) ? String(score) : "na";
      return `id=${windowInfo.id}/focused=${windowInfo.focused}/state=${windowInfo.state || "unknown"}/score=${scoreText}`;
    })
    .join(",");
  throw bridgeError(
    ERROR_AXTREE_FAILED,
    `openTab refused hwnd hint ${String(params.hwnd)}: candidate_count=${candidates.length} non_focused_count=${nonFocused.length} non_focused_non_minimized_count=${nonFocusedNonMinimized.length} candidates=[${candidateSummary}]; OS HWND cannot be mapped inside the normal extension bridge`
  );
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
  return Math.max(
    Math.abs(actual.x - expected.x),
    Math.abs(actual.y - expected.y),
    Math.abs(actual.w - expected.w),
    Math.abs(actual.h - expected.h)
  );
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
      target: { tabId },
      func: readActiveElementInPage
    });
    const first = Array.isArray(results) ? results[0] : null;
    if (!first || typeof first.result !== "object" || first.result === null) {
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
      ...first.result
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
      target: { tabId },
      func: readPageTextInPage,
      args: [MAX_PAGE_TEXT_CHARS]
    });
    const first = Array.isArray(results) ? results[0] : null;
    if (!first || typeof first.result !== "object" || first.result === null) {
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
      ...first.result
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
      target: { tabId },
      func: readPageVitalsInPage
    });
    const first = Array.isArray(results) ? results[0] : null;
    if (!first || typeof first.result !== "object" || first.result === null) {
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
      ...first.result
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
    max_chars: limit
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

function runtimeEvaluateExpression(expression, args) {
  if (args.length === 0) {
    return expression;
  }
  const serializedArgs = JSON.stringify(args);
  if (serializedArgs === undefined) {
    throw new Error("evaluateScript args could not be serialized to JSON");
  }
  return `((__synapseFn, __synapseArgs) => {
  if (typeof __synapseFn !== "function") {
    throw new TypeError("evaluateScript with args requires expression to evaluate to a function");
  }
  return __synapseFn(...__synapseArgs);
})((${expression}), ${serializedArgs})`;
}

function runtimeRemoteObjectResult(remote, returnByValue) {
  const resultType = String(remote.type || "");
  const resultSubtype = typeof remote.subtype === "string" ? remote.subtype : null;
  const description = typeof remote.description === "string"
    ? remote.description
    : remote.value === undefined
      ? resultType || "undefined"
      : String(remote.value);
  return {
    ok: true,
    result_type: resultType,
    result_subtype: resultSubtype,
    returned_by_value: Boolean(returnByValue),
    value: remote.value === undefined ? null : remote.value,
    description,
    unserializable_value: typeof remote.unserializableValue === "string"
      ? remote.unserializableValue
      : null
  };
}

function formatRuntimeException(details) {
  const parts = [];
  if (typeof details.text === "string" && details.text) {
    parts.push(details.text);
  }
  const exception = details.exception;
  if (exception && typeof exception === "object") {
    if (typeof exception.description === "string" && exception.description) {
      parts.push(exception.description);
    } else if (exception.value !== undefined) {
      parts.push(String(exception.value));
    }
  }
  const line = Number.isInteger(details.lineNumber) ? details.lineNumber : null;
  const column = Number.isInteger(details.columnNumber) ? details.columnNumber : null;
  if (line !== null || column !== null) {
    parts.push(`line=${line ?? "?"} column=${column ?? "?"}`);
  }
  return parts.length > 0 ? parts.join(" | ") : JSON.stringify(details);
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
  const before = readActiveElementInPage();
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
  const beforeInput = dispatchSyntheticInputEvent(element, "beforeinput", inputText, true);
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

  const expectedValue = applyTextToEditable(element, inputText);
  dispatchSyntheticInputEvent(element, "input", inputText, false);
  eventsDispatched.push("input");
  element.dispatchEvent(new Event("change", { bubbles: true }));
  eventsDispatched.push("change");

  const after = readActiveElementInPage();
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
      target: { tabId: selected.tabId },
      func: setFieldValueInPage,
      args: [{ selector, activeElement, text }]
    });
  } catch (error) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `chrome.scripting.executeScript setFieldValue(${selected.tabId}) failed: ${errorMessage(error)}`
    );
  }
  const first = Array.isArray(results) ? results[0] : null;
  const result = first?.result;
  if (!result || typeof result !== "object") {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      "chrome.scripting.executeScript setFieldValue returned no structured result"
    );
  }
  if (!result.ok) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `setFieldValue failed: code=${String(result.error_code || "UNKNOWN")} detail=${String(result.error_detail || "")}`
    );
  }
  return {
    extension_id: chrome.runtime.id,
    target_id: selected.target.id,
    tab_id: selected.tabId,
    chars_requested: [...text].length,
    readback_backend: "chrome.scripting.executeScript",
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason,
    ...result
  };
}

function setFieldValueInPage(request) {
  const selector = request && request.selector != null ? String(request.selector) : null;
  const wantActive = Boolean(request && request.activeElement);
  const inputText = String((request && request.text) ?? "");

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

  const tag = String(element.tagName || "").toLowerCase();
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
  if (["click", "press", "select", "submit"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(
    ERROR_CHROME_DOM_ACTION_UNSUPPORTED,
    `domAction action must be one of click, press, select, submit; got ${JSON.stringify(action)}`
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
    option: stringOrEmpty(request?.option)
  };
  const clicks = Number.isSafeInteger(request?.clicks) ? request.clicks : 1;
  const maxPageTextChars = Number.isSafeInteger(request?.maxPageTextChars)
    ? Math.min(Math.max(request.maxPageTextChars, 0), 65536)
    : 4096;

  if (!["click", "press", "select", "submit"].includes(action)) {
    return fail(ERROR_ACTION_UNSUPPORTED, `unsupported DOM action ${JSON.stringify(action)}`);
  }

  const beforeUrl = String(location.href || "");
  const beforePageText = readPageText(maxPageTextChars);
  const resolved = resolveDomActionElement(action, locator);
  if (!resolved.ok) {
    return resolved;
  }
  const element = resolved.element;
  const beforeElement = elementSummary(element);

  if (!isElementEnabled(element)) {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is disabled or aria-disabled", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement
    });
  }
  if (!isElementVisible(element) && action !== "submit") {
    return fail(ERROR_ELEMENT_NOT_ACTIONABLE, "resolved element is not visible/actionable", {
      matched_count: resolved.matchedCount,
      resolved_by: resolved.resolvedBy,
      before_element: beforeElement
    });
  }

  const eventsDispatched = [];
  let actionReadback = {};
  try {
    if (action === "click" || action === "press") {
      actionReadback = performClick(element, clicks, eventsDispatched);
    } else if (action === "select") {
      actionReadback = performNativeSelect(element, locator.option || locator.value, eventsDispatched);
    } else if (action === "submit") {
      actionReadback = performSubmit(element, eventsDispatched);
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

  function performNativeSelect(element, requested, events) {
    if (tag(element) !== "select") {
      throw actionError(
        ERROR_ACTION_UNSUPPORTED,
        `domAction select supports native <select> only; resolved ${tag(element)} role=${inferRole(element)}`
      );
    }
    const optionNeedle = stringOrEmpty(requested);
    if (!optionNeedle) {
      throw actionError(ERROR_ACTION_UNSUPPORTED, "domAction select requires option or value");
    }
    const options = Array.from(element.options || []);
    const exactValue = options.filter((option) => String(option.value ?? "") === optionNeedle);
    const exactText = options.filter((option) => normalizeText(option.textContent) === normalizeText(optionNeedle));
    const matches = exactValue.length > 0 ? exactValue : exactText;
    if (matches.length === 0) {
      throw actionError(ERROR_ELEMENT_NOT_FOUND, `no <option> matched ${JSON.stringify(optionNeedle)}`);
    }
    if (matches.length > 1) {
      throw actionError(ERROR_ELEMENT_AMBIGUOUS, `${matches.length} <option> elements matched ${JSON.stringify(optionNeedle)}`);
    }
    const option = matches[0];
    element.value = String(option.value ?? "");
    option.selected = true;
    element.dispatchEvent(new Event("input", { bubbles: true }));
    events.push("input");
    element.dispatchEvent(new Event("change", { bubbles: true }));
    events.push("change");
    if (element.value !== String(option.value ?? "")) {
      throw actionError(
        ERROR_POSTCONDITION_FAILED,
        `select value readback ${JSON.stringify(element.value)} did not equal requested option value ${JSON.stringify(option.value)}`
      );
    }
    return {
      selected_value: String(option.value ?? ""),
      selected_text: trimForReadback(option.textContent, 160),
      selected_index: element.selectedIndex
    };
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
      option: loc.option || null
    });
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
    disableBridgePermanently(
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
