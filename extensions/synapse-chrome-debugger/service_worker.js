const HOST_NAME = "com.synapse.chrome_debugger";
const PROTOCOL_VERSION = 1;
const ERROR_ATTACH_FAILED = "A11Y_CDP_ATTACH_FAILED";
const ERROR_AXTREE_FAILED = "A11Y_CDP_AXTREE_FAILED";
const ERROR_DEBUGGER_WARNING_UNSUPPRESSED = "A11Y_CDP_DEBUGGER_WARNING_UNSUPPRESSED";
const ERROR_EXTENSION_DETACHED = "A11Y_CDP_EXTENSION_DETACHED";
const ERROR_EXTENSION_TIMEOUT = "A11Y_CDP_EXTENSION_TIMEOUT";
const ERROR_EXTENSION_UNAVAILABLE = "A11Y_CDP_EXTENSION_UNAVAILABLE";
const SILENT_DEBUGGER_SWITCH = "--silent-debugger-extension-api";
const DETACH_SURFACE_MS = 5000;

let nativePort = null;
let reconnectTimer = null;
const attachedTabs = new Set();
const intentionalDetachTabs = new Set();
const recentDetachByTab = new Map();

function connectNative() {
  if (nativePort) {
    return;
  }
  try {
    nativePort = chrome.runtime.connectNative(HOST_NAME);
  } catch (error) {
    nativePort = null;
    scheduleReconnect(`connectNative failed: ${errorMessage(error)}`);
    return;
  }
  nativePort.onMessage.addListener((message) => {
    handleCommand(message).catch((error) => {
      postResponse(message?.id ?? "", false, null, errorPayload(error));
    });
  });
  nativePort.onDisconnect.addListener(() => {
    const detail = chrome.runtime.lastError?.message || "native port disconnected";
    nativePort = null;
    scheduleReconnect(detail);
  });
  postNative({
    type: "hello",
    extensionId: chrome.runtime.id,
    version: chrome.runtime.getManifest().version,
    protocolVersion: PROTOCOL_VERSION,
    userAgent: navigator.userAgent
  });
}

function scheduleReconnect(detail) {
  if (reconnectTimer) {
    return;
  }
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectNative();
  }, 1000);
  console.warn(`Synapse native bridge disconnected: ${detail}`);
}

chrome.runtime.onInstalled.addListener(connectNative);
chrome.runtime.onStartup.addListener(connectNative);
chrome.debugger.onDetach.addListener((source, reason) => {
  const tabId = source.tabId;
  const intentional = typeof tabId === "number" && intentionalDetachTabs.has(tabId);
  if (typeof source.tabId === "number") {
    attachedTabs.delete(source.tabId);
    if (intentional) {
      intentionalDetachTabs.delete(source.tabId);
    } else {
      recentDetachByTab.set(source.tabId, Date.now());
    }
  }
  postNative({
    type: "event",
    event: "debuggerDetached",
    tabId: source.tabId ?? null,
    targetId: source.targetId ?? null,
    intentional,
    reason
  });
});

connectNative();

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
      result = await handleSnapshot(params);
    } else if (kind === "clickNode") {
      result = await handleClickNode(params);
    } else if (kind === "typeNode") {
      result = await handleTypeNode(params);
    } else if (kind === "nodeValue") {
      result = await handleNodeValue(params);
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
    postResponse(id, true, result, null);
  } catch (error) {
    postResponse(id, false, null, errorPayload(error));
  }
}

async function handleSnapshot(params) {
  const selected = await selectPageTarget(params);
  return await withAttached(selected, params, async (debuggee) => {
    await sendCdp(debuggee, "Accessibility.enable", {});
    const tree = await sendCdp(debuggee, "Accessibility.getFullAXTree", {});
    const axNodes = Array.isArray(tree.nodes) ? tree.nodes : [];
    const byAxId = new Map(axNodes.map((node) => [String(node.nodeId), node]));
    const maxNodes = Math.max(0, Number(params.maxNodes ?? 200));
    const domNodes = [];
    let totalAxNodes = 0;
    for (const node of axNodes) {
      if (node.ignored) {
        continue;
      }
      totalAxNodes += 1;
      const backend = Number(node.backendDOMNodeId);
      if (!Number.isFinite(backend)) {
        continue;
      }
      const role = axValueString(node.role);
      if (!role) {
        continue;
      }
      const bbox = domNodes.length < maxNodes
        ? await boxForBackend(debuggee, backend)
        : null;
      domNodes.push({
        backend_node_id: backend,
        parent_backend_node_id: nearestBackendAncestor(node, byAxId),
        role,
        name: axValueString(node.name),
        value: nonEmptyOrNull(axValueString(node.value)),
        bbox,
        child_count: Array.isArray(node.childIds) ? node.childIds.length : 0,
        enabled: !axBoolProperty(node, "disabled"),
        focused: axBoolProperty(node, "focused")
      });
    }
    return {
      extension_id: chrome.runtime.id,
      nodes: domNodes,
      total_ax_nodes: totalAxNodes,
      page_url: selected.url || "",
      target_id: selected.target.id,
      session_id: `tab-${selected.tabId}`,
      target_candidate_count: selected.targetCandidateCount,
      target_selection_reason: selected.selectionReason
    };
  });
}

async function handleClickNode(params) {
  const backendNodeId = requiredNumber(params.backendNodeId, "backendNodeId");
  const selected = await selectPageTarget(params);
  return await withAttached(selected, params, async (debuggee) => {
    await sendCdp(debuggee, "DOM.scrollIntoViewIfNeeded", { backendNodeId });
    const bbox = await boxForBackend(debuggee, backendNodeId);
    if (!bbox || bbox.w <= 0 || bbox.h <= 0) {
      throw bridgeError(ERROR_AXTREE_FAILED, `backendNodeId ${backendNodeId} has no clickable box model`);
    }
    const point = {
      x: bbox.x + bbox.w / 2,
      y: bbox.y + bbox.h / 2
    };
    const button = normalizeButton(params.button);
    const clickCount = Math.max(1, Number(params.clickCount ?? 1));
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: point.x,
      y: point.y,
      button: "none",
      buttons: 0,
      clickCount: 0
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: point.x,
      y: point.y,
      button,
      buttons: buttonMask(button),
      clickCount
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: point.x,
      y: point.y,
      button,
      buttons: 0,
      clickCount
    });
    return { x: point.x, y: point.y, target_id: selected.target.id };
  });
}

async function handleTypeNode(params) {
  const backendNodeId = requiredNumber(params.backendNodeId, "backendNodeId");
  const text = String(params.text ?? "");
  const selected = await selectPageTarget(params);
  return await withAttached(selected, params, async (debuggee) => {
    await sendCdp(debuggee, "DOM.scrollIntoViewIfNeeded", { backendNodeId });
    const bbox = await boxForBackend(debuggee, backendNodeId);
    if (!bbox || bbox.w <= 0 || bbox.h <= 0) {
      throw bridgeError(ERROR_AXTREE_FAILED, `backendNodeId ${backendNodeId} has no focusable box model`);
    }
    const point = {
      x: bbox.x + bbox.w / 2,
      y: bbox.y + bbox.h / 2
    };
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseMoved",
      x: point.x,
      y: point.y,
      button: "none",
      buttons: 0,
      clickCount: 0
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mousePressed",
      x: point.x,
      y: point.y,
      button: "left",
      buttons: 1,
      clickCount: 1
    });
    await sendCdp(debuggee, "Input.dispatchMouseEvent", {
      type: "mouseReleased",
      x: point.x,
      y: point.y,
      button: "left",
      buttons: 0,
      clickCount: 1
    });
    try {
      await sendCdp(debuggee, "DOM.focus", { backendNodeId });
    } catch (_) {
      // The click above is the authoritative focus/caret placement. Some AX
      // wrapper nodes are not directly focusable even though the click lands
      // in the editable control.
    }
    await sendCdp(debuggee, "Input.insertText", { text });
    return { x: point.x, y: point.y, target_id: selected.target.id };
  });
}

async function handleNodeValue(params) {
  const backendNodeId = requiredNumber(params.backendNodeId, "backendNodeId");
  const selected = await selectPageTarget(params);
  return await withAttached(selected, params, async (debuggee) => {
    const resolved = await sendCdp(debuggee, "DOM.resolveNode", { backendNodeId });
    const objectId = resolved?.object?.objectId;
    if (!objectId) {
      throw bridgeError(ERROR_AXTREE_FAILED, `DOM.resolveNode returned no objectId for backendNodeId ${backendNodeId}`);
    }
    const readback = await sendCdp(debuggee, "Runtime.callFunctionOn", {
      objectId,
      returnByValue: true,
      silent: true,
      functionDeclaration: `function() {
        if (this === null || this === undefined) { return ""; }
        if ("value" in this) { return String(this.value ?? ""); }
        if ("checked" in this) { return String(Boolean(this.checked)); }
        if (this.isContentEditable && this.innerText !== null && this.innerText !== undefined) {
          return String(this.innerText).replace(/\\n$/, "");
        }
        if (this.textContent !== null && this.textContent !== undefined) {
          return String(this.textContent);
        }
        return "";
      }`
    });
    return {
      value: String(readback?.result?.value ?? ""),
      target_id: selected.target.id
    };
  });
}

async function handleOpenTab(params) {
  const requestedUrl = normalizeOpenUrl(params.url);
  const beforePages = await pageTargets();
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
  const target = await waitForTargetForTab(tab.id, 10000);
  const afterPages = await pageTargets();
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
  const selected = await selectPageTarget(params, { requireTargetId: true });
  const beforePages = await pageTargets();
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
  const selected = await selectPageTarget(params, { requireTargetId: true });
  return {
    extension_id: chrome.runtime.id,
    target_id: selected.target.id,
    tab_id: selected.tabId,
    target_type: selected.target.type || "page",
    url: selected.target.url || "",
    title: selected.target.title || "",
    target_candidate_count: selected.targetCandidateCount,
    target_selection_reason: selected.selectionReason
  };
}

async function handleNavigateTab(params) {
  const selected = await selectPageTarget(params, { requireTargetId: true });
  const action = normalizeNavigateAction(params.action);
  const requestedUrl = action === "navigate" ? requiredUrl(params.url) : null;
  const waitTimeoutMs = normalizeWaitTimeout(params.waitTimeoutMs);
  const ignoreCache = Boolean(params.ignoreCache);
  const before = await tabPageState(selected.tabId, selected.target);
  let readbackExpectation = null;
  try {
    if (action === "navigate") {
      await chrome.tabs.update(selected.tabId, { url: requestedUrl });
      if (requestedUrl !== before.url) {
        readbackExpectation = {
          description: `tab url to become ${JSON.stringify(requestedUrl)} or differ from ${JSON.stringify(before.url)}`,
          matches: (state) => state.url === requestedUrl || state.url !== before.url
        };
      }
    } else if (action === "reload") {
      await chrome.tabs.reload(selected.tabId, { bypassCache: ignoreCache });
    } else if (action === "back") {
      await chrome.tabs.goBack(selected.tabId);
      readbackExpectation = {
        description: `tab url to change after chrome.tabs.goBack from ${JSON.stringify(before.url)}`,
        matches: (state) => state.url !== before.url
      };
    } else if (action === "forward") {
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

async function selectPageTarget(params, options = {}) {
  const pages = await pageTargets();
  if (pages.length === 0) {
    throw bridgeError(ERROR_ATTACH_FAILED, "chrome.debugger.getTargets returned no page targets");
  }
  const targetIdHint = String(params.targetIdHint || "").trim();
  const urlHint = String(params.foregroundUrlHint || "").trim();
  const titleHint = String(params.foregroundTitle || "").trim();
  const selectedById = targetIdHint
    ? pages.find((target) => target.id === targetIdHint)
    : null;
  if (selectedById) {
    return selectedPage(selectedById, pages.length, "target_id_hint");
  }
  if (options.requireTargetId) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      targetIdHint
        ? `targetIdHint ${targetIdHint} was not found in chrome.debugger.getTargets`
        : "targetIdHint is required for mutating tab navigation"
    );
  }
  if (targetIdHint) {
    throw bridgeError(
      ERROR_AXTREE_FAILED,
      `targetIdHint ${targetIdHint} was not found in chrome.debugger.getTargets`
    );
  }
  if (urlHint) {
    const matches = pages.filter((target) => urlMatchesHint(target.url || "", urlHint));
    if (matches.length === 1) {
      return selectedPage(matches[0], pages.length, "url_hint");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `url hint matched ${matches.length} page targets`);
    }
  }
  if (titleHint) {
    const matches = pages.filter((target) => {
      const title = target.title || "";
      return title && (titleHint.includes(title) || title.includes(titleHint));
    });
    if (matches.length === 1) {
      return selectedPage(matches[0], pages.length, "foreground_title");
    }
    if (matches.length > 1) {
      throw bridgeError(ERROR_AXTREE_FAILED, `title hint matched ${matches.length} page targets`);
    }
  }
  return selectedPage(pages[0], pages.length, "fallback_first_page");
}

async function pageTargets() {
  const targets = await chrome.debugger.getTargets();
  return targets.filter((target) => target.type === "page" && typeof target.tabId === "number");
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

async function withAttached(selected, params, operation) {
  requireAttachSuppressionVerified(params);
  const debuggee = await attachForCommand(selected);
  try {
    return await operation(debuggee);
  } finally {
    await detachAfterCommand(debuggee, selected.tabId);
  }
}

function requireAttachSuppressionVerified(params) {
  if (params?.debuggerAttachSuppressionVerified === true) {
    return;
  }
  throw bridgeError(
    ERROR_DEBUGGER_WARNING_UNSUPPRESSED,
    `chrome.debugger.attach refused before attach because the daemon did not attest debugger warning suppression; ` +
      `hwnd=${String(params?.hwnd ?? "unknown")} requiredFlag=${SILENT_DEBUGGER_SWITCH}`
  );
}

async function attachForCommand(selected) {
  const tabId = selected.tabId;
  const recentDetachAt = recentDetachByTab.get(tabId);
  if (recentDetachAt && Date.now() - recentDetachAt < DETACH_SURFACE_MS) {
    throw bridgeError(
      ERROR_EXTENSION_DETACHED,
      `debugger detached from tab ${tabId}; reason was surfaced and command refused`
    );
  }
  const debuggee = { tabId };
  try {
    await chrome.debugger.attach(debuggee, "1.3");
  } catch (error) {
    if (!await existingAttachStillUsable(debuggee, error)) {
      throw bridgeError(ERROR_ATTACH_FAILED, `chrome.debugger.attach tab ${tabId}: ${errorMessage(error)}`);
    }
  }
  attachedTabs.add(tabId);
  await sendCdp(debuggee, "Runtime.enable", {});
  await sendCdp(debuggee, "DOM.enable", {});
  await sendCdp(debuggee, "Target.setAutoAttach", {
    autoAttach: true,
    waitForDebuggerOnStart: false,
    flatten: true,
    filter: [{ type: "iframe", exclude: false }]
  });
  return debuggee;
}

async function detachAfterCommand(debuggee, tabId) {
  if (!attachedTabs.has(tabId)) {
    return;
  }
  intentionalDetachTabs.add(tabId);
  try {
    await chrome.debugger.detach(debuggee);
  } catch (error) {
    intentionalDetachTabs.delete(tabId);
    throw bridgeError(ERROR_EXTENSION_DETACHED, `chrome.debugger.detach tab ${tabId}: ${errorMessage(error)}`);
  } finally {
    attachedTabs.delete(tabId);
  }
}

async function existingAttachStillUsable(debuggee, attachError) {
  if (!/another debugger/i.test(errorMessage(attachError))) {
    return false;
  }
  try {
    await chrome.debugger.sendCommand(debuggee, "Runtime.enable", {});
    return true;
  } catch (_) {
    return false;
  }
}

async function sendCdp(debuggee, method, params) {
  try {
    return await chrome.debugger.sendCommand(debuggee, method, params);
  } catch (error) {
    const message = errorMessage(error);
    if (/detached|not attached/i.test(message)) {
      throw bridgeError(ERROR_EXTENSION_DETACHED, `${method}: ${message}`);
    }
    throw bridgeError(ERROR_AXTREE_FAILED, `${method}: ${message}`);
  }
}

async function boxForBackend(debuggee, backendNodeId) {
  try {
    const response = await sendCdp(debuggee, "DOM.getBoxModel", { backendNodeId });
    return rectFromQuad(response?.model?.content || []);
  } catch (_) {
    return null;
  }
}

function nearestBackendAncestor(node, byAxId) {
  let parentId = node.parentId;
  let guard = 256;
  while (parentId && guard > 0) {
    const parent = byAxId.get(String(parentId));
    if (!parent) {
      return null;
    }
    const backend = Number(parent.backendDOMNodeId);
    if (Number.isFinite(backend)) {
      return backend;
    }
    parentId = parent.parentId;
    guard -= 1;
  }
  return null;
}

function rectFromQuad(quad) {
  if (!Array.isArray(quad) || quad.length < 8) {
    return null;
  }
  const xs = [quad[0], quad[2], quad[4], quad[6]].map(Number);
  const ys = [quad[1], quad[3], quad[5], quad[7]].map(Number);
  if (![...xs, ...ys].every(Number.isFinite)) {
    return null;
  }
  const minX = Math.min(...xs);
  const maxX = Math.max(...xs);
  const minY = Math.min(...ys);
  const maxY = Math.max(...ys);
  return {
    x: Math.round(minX),
    y: Math.round(minY),
    w: Math.max(0, Math.round(maxX - minX)),
    h: Math.max(0, Math.round(maxY - minY))
  };
}

function axValueString(value) {
  if (!value || value.value === undefined || value.value === null) {
    return "";
  }
  return String(value.value);
}

function axBoolProperty(node, name) {
  const properties = Array.isArray(node.properties) ? node.properties : [];
  const found = properties.find((property) => property.name === name);
  return Boolean(found?.value?.value);
}

function nonEmptyOrNull(value) {
  return value ? value : null;
}

function urlMatchesHint(url, hint) {
  return url === hint || url.startsWith(hint) || hint.startsWith(url);
}

function requiredNumber(value, name) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw bridgeError(ERROR_ATTACH_FAILED, `${name} must be a finite number`);
  }
  return number;
}

function normalizeButton(button) {
  const normalized = String(button || "left").toLowerCase();
  if (["left", "right", "middle"].includes(normalized)) {
    return normalized;
  }
  throw bridgeError(ERROR_ATTACH_FAILED, `unsupported mouse button ${button}`);
}

function buttonMask(button) {
  if (button === "left") {
    return 1;
  }
  if (button === "right") {
    return 2;
  }
  if (button === "middle") {
    return 4;
  }
  return 0;
}

async function waitForTargetForTab(tabId, waitTimeoutMs) {
  const started = Date.now();
  let lastCount = 0;
  while (Date.now() - started <= waitTimeoutMs) {
    const pages = await pageTargets();
    lastCount = pages.length;
    const target = pages.find((candidate) => candidate.tabId === tabId);
    if (target?.id) {
      return target;
    }
    await sleep(100);
  }
  throw bridgeError(
    ERROR_EXTENSION_TIMEOUT,
    `chrome.debugger.getTargets did not expose a page target for new tab ${tabId} within ${waitTimeoutMs} ms; lastPageTargetCount=${lastCount}`
  );
}

async function waitForTargetAbsent(targetId, waitTimeoutMs) {
  const started = Date.now();
  let pages = [];
  while (Date.now() - started <= waitTimeoutMs) {
    pages = await pageTargets();
    if (!pages.some((candidate) => candidate.id === targetId)) {
      return pages;
    }
    await sleep(100);
  }
  throw bridgeError(
    ERROR_EXTENSION_TIMEOUT,
    `chrome.debugger.getTargets still contains closed target ${JSON.stringify(targetId)} after ${waitTimeoutMs} ms; lastPageTargetCount=${pages.length}`
  );
}

async function waitForPageState(debuggee, waitTimeoutMs, expectation = null) {
  const started = Date.now();
  let last = null;
  let lastError = null;
  while (Date.now() - started <= waitTimeoutMs) {
    try {
      last = await pageState(debuggee);
      lastError = null;
      const loaded = last.ready_state === "complete" || last.ready_state === "interactive";
      if (loaded && (!expectation || expectation.matches(last))) {
        return last;
      }
    } catch (error) {
      lastError = error?.message ? String(error.message) : String(error);
    }
    await sleep(100);
  }
  const detail = last
    ? `waiting for ${expectation?.description || "stable loaded page"}; ` +
      `last url=${JSON.stringify(last.url)} title=${JSON.stringify(last.title)} ` +
      `readyState=${JSON.stringify(last.ready_state)} historyIndex=${last.history_current_index} ` +
      `historyEntries=${last.history_entry_count}`
    : lastError
      ? `last readback error=${JSON.stringify(lastError)}`
    : "no page state readback";
  throw bridgeError(ERROR_EXTENSION_TIMEOUT, `page readback did not settle within ${waitTimeoutMs} ms; ${detail}`);
}

async function pageState(debuggee) {
  const dom = await sendCdp(debuggee, "Runtime.evaluate", {
    returnByValue: true,
    expression: `(() => ({
      url: String(location.href || ""),
      title: String(document.title || ""),
      ready_state: String(document.readyState || "")
    }))()`
  });
  const value = dom?.result?.value || {};
  const history = await sendCdp(debuggee, "Page.getNavigationHistory", {});
  const entries = Array.isArray(history?.entries) ? history.entries : [];
  return {
    url: String(value.url || ""),
    title: String(value.title || ""),
    ready_state: String(value.ready_state || ""),
    history_current_index: Number.isFinite(Number(history?.currentIndex))
      ? Number(history.currentIndex)
      : -1,
    history_entry_count: entries.length
  };
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
  const target = await targetForTab(tabId).catch(() => fallbackTarget);
  return {
    target_id: String(target?.id || fallbackTarget?.id || ""),
    url: String(tab.pendingUrl || tab.url || target?.url || fallbackTarget?.url || ""),
    title: String(tab.title || target?.title || fallbackTarget?.title || ""),
    ready_state: String(tab.status || ""),
    history_current_index: -1,
    history_entry_count: 0
  };
}

async function targetForTab(tabId) {
  const pages = await pageTargets();
  const target = pages.find((candidate) => candidate.tabId === tabId);
  if (!target) {
    throw bridgeError(ERROR_ATTACH_FAILED, `chrome.debugger.getTargets did not expose a page target for tab ${tabId}`);
  }
  return target;
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

function postResponse(id, ok, result, error) {
  postNative({
    type: "response",
    id,
    ok,
    result,
    error
  });
}

function postNative(message) {
  if (!nativePort) {
    console.warn("Synapse native bridge unavailable; message dropped");
    return;
  }
  nativePort.postMessage(message);
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
