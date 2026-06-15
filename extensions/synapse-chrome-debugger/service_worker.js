const PROTOCOL_VERSION = 1;
const EXPECTED_EXTENSION_ID = "leoocgnkjnplbfdbklajepahofecgfbk";
const DAEMON_BASE_URL = "http://127.0.0.1:7700";
const ERROR_ATTACH_FAILED = "A11Y_CDP_ATTACH_FAILED";
const ERROR_AXTREE_FAILED = "A11Y_CDP_AXTREE_FAILED";
const ERROR_DEBUGGER_WARNING_UNSUPPRESSED = "A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED";
const ERROR_EXTENSION_TIMEOUT = "A11Y_CDP_EXTENSION_TIMEOUT";
const ERROR_EXTENSION_ID_MISMATCH = "SYNAPSE_CHROME_EXTENSION_ID_MISMATCH";
const ERROR_DAEMON_UNAVAILABLE = "SYNAPSE_CHROME_DAEMON_UNAVAILABLE";
const TAB_TARGET_PREFIX = "chrome-tab:";
const BRIDGE_TOKEN_HEADER = "X-Synapse-Bridge-Token";
const DAEMON_WS_BASE_URL = "ws://127.0.0.1:7700";
const WEBSOCKET_KEEPALIVE_MS = 20000;
const RECONNECT_INITIAL_MS = 1000;
const RECONNECT_MAX_MS = 30000;
const DISCONNECTED_KEEPALIVE_MS = 20000;
const AGENT_NAVIGATION_CLAIM_TTL_MS = 30000;
const MAX_RECENT_NAVIGATION_KEYS = 128;

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
    extensionId: chrome.runtime.id,
    version: chrome.runtime.getManifest().version,
    protocolVersion: PROTOCOL_VERSION,
    transport: "direct_http",
    userAgent: navigator.userAgent
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
    } else if (kind === "targetInfo") {
      result = await handleTargetInfo(params);
    } else if (kind === "navigateTab") {
      result = await handleNavigateTab(params);
    } else {
      throw bridgeError(ERROR_ATTACH_FAILED, `unknown command kind ${String(kind)}`);
    }
    await postResponse(id, true, result, null);
  } catch (error) {
    await postResponse(id, false, null, errorPayload(error));
  }
}

function rejectAttachCommand(kind, params) {
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `normal Synapse Chrome Bridge refused ${String(kind)} before browser debugger startup; ` +
      `the normal end-user extension is tabs-only and contains no debugger transport. ` +
      `hwnd=${String(params?.hwnd ?? "unknown")} remediation=use raw CDP with a dedicated ` +
      `Synapse-launched automation profile`
  );
}

async function handleOpenTab(params) {
  const requestedUrl = normalizeOpenUrl(params.url);
  const agentSessionId = normalizeOptionalSessionId(params.agentSessionId);
  const beforePages = await tabTargets();
  let tab;
  try {
    tab = await chrome.tabs.create({
      url: requestedUrl || "about:blank",
      active: false
    });
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
  const afterPages = await tabTargets();
  return {
    extension_id: chrome.runtime.id,
    target_id: target.id,
    tab_id: tab.id,
    target_type: target.type || "page",
    url: target.url || tab.url || requestedUrl || "about:blank",
    title: target.title || tab.title || "",
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
    readback_backend: "chrome.tabs.get",
    active_element: activeElement,
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
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

async function selectTabTarget(params, options = {}) {
  const tabs = await tabTargets();
  if (tabs.length === 0) {
    throw bridgeError(ERROR_AXTREE_FAILED, "chrome.tabs.query returned no tab targets");
  }
  const targetIdHint = String(params.targetIdHint || "").trim();
  const tabIdHint = tabIdFromTargetId(targetIdHint);
  if (Number.isInteger(tabIdHint)) {
    const selectedById = tabs.find((target) => target.tabId === tabIdHint);
    if (selectedById) {
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
