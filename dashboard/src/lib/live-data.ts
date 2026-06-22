import { asRecord, rawText, timeAgo } from "@/lib/utils";

export const DASHBOARD_LIVE_SCOPES = ["fleet", "agent", "tasks", "system", "audit"] as const;

export type DashboardPanelScope = (typeof DASHBOARD_LIVE_SCOPES)[number];

export type DashboardLiveStatus = "connecting" | "open" | "disconnected";

export interface DashboardEventSubscription {
  ok: boolean;
  source_of_truth: string;
  scope: DashboardPanelScope;
  subscription_id: string;
  event_url: string;
  replay_contract: string;
}

export interface DashboardLiveScopeState {
  scope: DashboardPanelScope;
  status: DashboardLiveStatus;
  subscriptionId?: string;
  eventUrl?: string;
  lastEventId?: string;
  lastUpdateUnixMs?: number;
  lastResyncUnixMs?: number;
  eventCount: number;
  resyncCount: number;
  error?: string;
}

export type DashboardLiveState = Record<DashboardPanelScope, DashboardLiveScopeState>;

export interface DashboardLiveEvent {
  seq?: number;
  source?: string;
  kind?: string;
  data?: unknown;
}

export interface DashboardLiveEventMeta {
  lossy: boolean;
  streamSeq?: number;
  lastEventId?: string;
  reason: "event" | "lossy_replay" | "subscription_lossy";
}

export interface DashboardEventSourceLike {
  onopen: ((event: Event) => void) | null;
  onerror: ((event: Event) => void) | null;
  addEventListener(type: string, listener: (event: MessageEvent) => void): void;
  close(): void;
}

export interface DashboardLiveControllerOptions {
  scopes?: readonly DashboardPanelScope[];
  subscribe?: (scope: DashboardPanelScope) => Promise<DashboardEventSubscription>;
  eventSourceFactory?: (url: string) => DashboardEventSourceLike;
  onScopeEvent?: (
    scope: DashboardPanelScope,
    event: DashboardLiveEvent | undefined,
    meta: DashboardLiveEventMeta
  ) => void;
  onStateChange?: (state: DashboardLiveState) => void;
  now?: () => number;
  setTimer?: (callback: () => void, delayMs: number) => unknown;
  clearTimer?: (timer: unknown) => void;
  reconnectDelayMs?: number;
}

export function initialDashboardLiveState(
  scopes: readonly DashboardPanelScope[] = DASHBOARD_LIVE_SCOPES
): DashboardLiveState {
  return scopes.reduce((state, scope) => {
    state[scope] = {
      scope,
      status: "connecting",
      eventCount: 0,
      resyncCount: 0
    };
    return state;
  }, {} as DashboardLiveState);
}

export function dashboardLiveScopeForRoute(route: string): DashboardPanelScope {
  if (route === "agent") return "agent";
  if (route === "tasks") return "tasks";
  if (route === "audit") return "audit";
  if (route === "system" || route === "timeline" || route === "analytics") return "system";
  return "fleet";
}

export function dashboardQueryKeysForScope(scope: DashboardPanelScope): readonly (readonly unknown[])[] {
  if (scope === "audit") {
    return [["dashboard-state"], ["dashboard-audit-query"]];
  }
  if (scope === "system") {
    return [["dashboard-state"], ["dashboard-models"], ["dashboard-templates"]];
  }
  return [["dashboard-state"]];
}

export function dashboardEventScopesForEvent(event: DashboardLiveEvent): DashboardPanelScope[] {
  const kind = rawText(event.kind).toLowerCase();
  const source = rawText(event.source).toLowerCase();
  const scopes = new Set<DashboardPanelScope>();

  if (kind === "agent_state_changed") {
    scopes.add("fleet");
    scopes.add("agent");
    scopes.add("tasks");
  }
  if (kind === "workspace.put") {
    scopes.add("fleet");
    scopes.add("agent");
    scopes.add("tasks");
    scopes.add("system");
  }
  if (kind === "profile-changed" || kind === "scope-transitioned") {
    scopes.add("agent");
    scopes.add("system");
  }
  if (/approval|attention|needs_input|task/.test(kind)) {
    scopes.add("fleet");
    scopes.add("tasks");
  }
  if (/lease|target|session/.test(kind)) {
    scopes.add("agent");
    scopes.add("system");
  }
  if (source === "action_emitter" || source === "reflex") {
    scopes.add("audit");
    scopes.add("system");
  }
  if (["system", "filesystem", "process", "clipboard", "perception_audio"].includes(source)) {
    scopes.add("system");
  }
  if (scopes.size === 0) scopes.add("system");
  return Array.from(scopes);
}

export function dashboardLiveLabel(
  state: DashboardLiveScopeState | undefined,
  nowMs: number
): { text: string; tone: "ok" | "warning" | "danger" | "muted" } {
  if (!state) return { text: "SSE pending", tone: "muted" };
  if (state.status === "disconnected") return { text: "SSE disconnected", tone: "danger" };
  if (state.status === "connecting") return { text: "SSE connecting", tone: "warning" };
  if (state.lastResyncUnixMs && nowMs - state.lastResyncUnixMs < 15000) {
    return { text: `SSE resynced ${timeAgo(nowMs - state.lastResyncUnixMs)} ago`, tone: "warning" };
  }
  if (!state.lastUpdateUnixMs) return { text: "SSE connected", tone: "ok" };
  return { text: `SSE live ${timeAgo(nowMs - state.lastUpdateUnixMs)} ago`, tone: "ok" };
}

export async function subscribeDashboardEvents(
  scope: DashboardPanelScope
): Promise<DashboardEventSubscription> {
  const response = await fetch("/dashboard/events/subscribe", {
    method: "POST",
    cache: "no-store",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ scope })
  });
  const body = (await response.json().catch(() => ({}))) as Record<string, unknown>;
  if (!response.ok || body.ok === false) {
    const code = rawText(body.code) || `HTTP ${response.status}`;
    const message = rawText(body.message) || response.statusText;
    throw new Error(`${code}: ${message}`);
  }
  return {
    ok: body.ok === true,
    source_of_truth: rawText(body.source_of_truth),
    scope,
    subscription_id: rawText(body.subscription_id),
    event_url: rawText(body.event_url),
    replay_contract: rawText(body.replay_contract)
  };
}

export function createDashboardLiveController(options: DashboardLiveControllerOptions = {}) {
  const scopes = options.scopes ?? DASHBOARD_LIVE_SCOPES;
  const subscribe = options.subscribe ?? subscribeDashboardEvents;
  const eventSourceFactory =
    options.eventSourceFactory ??
    ((url: string) => {
      if (typeof EventSource === "undefined") throw new Error("EventSource is unavailable");
      return new EventSource(url) as DashboardEventSourceLike;
    });
  const now = options.now ?? Date.now;
  const setTimer =
    options.setTimer ??
    ((callback: () => void, delayMs: number) => window.setTimeout(callback, delayMs));
  const clearTimer =
    options.clearTimer ??
    ((timer: unknown) => window.clearTimeout(timer as number));
  const reconnectDelayMs = options.reconnectDelayMs ?? 5000;
  let stopped = false;
  let state = initialDashboardLiveState(scopes);
  const sources = new Map<DashboardPanelScope, DashboardEventSourceLike>();
  const timers = new Map<DashboardPanelScope, unknown>();

  const emit = () => options.onStateChange?.(cloneLiveState(state));
  const updateScope = (scope: DashboardPanelScope, patch: Partial<DashboardLiveScopeState>) => {
    state = {
      ...state,
      [scope]: {
        ...state[scope],
        ...patch
      }
    };
    emit();
  };

  const scheduleReconnect = (scope: DashboardPanelScope) => {
    if (stopped || timers.has(scope)) return;
    const timer = setTimer(() => {
      timers.delete(scope);
      void connect(scope);
    }, reconnectDelayMs);
    timers.set(scope, timer);
  };
  const clearReconnect = (scope: DashboardPanelScope) => {
    const timer = timers.get(scope);
    if (!timer) return;
    clearTimer(timer);
    timers.delete(scope);
  };

  const connect = async (scope: DashboardPanelScope) => {
    if (stopped) return;
    sources.get(scope)?.close();
    sources.delete(scope);
    updateScope(scope, { status: "connecting", error: undefined });
    try {
      const subscription = await subscribe(scope);
      if (stopped) return;
      updateScope(scope, {
        subscriptionId: subscription.subscription_id,
        eventUrl: subscription.event_url,
        error: undefined
      });
      const source = eventSourceFactory(subscription.event_url);
      source.onopen = () => {
        clearReconnect(scope);
        updateScope(scope, { status: "open", error: undefined });
      };
      source.onerror = () => {
        updateScope(scope, {
          status: "disconnected",
          error: "event stream disconnected"
        });
        scheduleReconnect(scope);
      };
      source.addEventListener("subscription_started", (message) => {
        handleSubscriptionStarted(scope, message);
      });
      source.addEventListener("synapse/event", (message) => {
        handleEvent(scope, message);
      });
      sources.set(scope, source);
    } catch (error) {
      updateScope(scope, {
        status: "disconnected",
        error: error instanceof Error ? error.message : rawText(error) || "event subscription failed"
      });
      scheduleReconnect(scope);
    }
  };

  const handleSubscriptionStarted = (scope: DashboardPanelScope, message: MessageEvent) => {
    const data = asRecord(parseSseJson(message.data));
    if (data.lossy !== true) return;
    const nowUnixMs = now();
    updateScope(scope, {
      status: "open",
      lastResyncUnixMs: nowUnixMs,
      resyncCount: state[scope].resyncCount + 1,
      error: undefined
    });
    options.onScopeEvent?.(scope, undefined, {
      lossy: true,
      lastEventId: message.lastEventId,
      reason: "subscription_lossy"
    });
  };

  const handleEvent = (scope: DashboardPanelScope, message: MessageEvent) => {
    const body = asRecord(parseSseJson(message.data));
    const params = asRecord(body.params);
    const event = asRecord(params.event) as unknown as DashboardLiveEvent;
    const lossy = params.lossy === true;
    const streamSeq = Number(params.stream_seq);
    const matchesScope = dashboardEventScopesForEvent(event).includes(scope);
    if (!matchesScope && !lossy) return;
    const nowUnixMs = now();
    updateScope(scope, {
      status: "open",
      lastEventId: message.lastEventId,
      lastUpdateUnixMs: nowUnixMs,
      lastResyncUnixMs: lossy ? nowUnixMs : state[scope].lastResyncUnixMs,
      eventCount: state[scope].eventCount + 1,
      resyncCount: lossy ? state[scope].resyncCount + 1 : state[scope].resyncCount,
      error: undefined
    });
    options.onScopeEvent?.(scope, event, {
      lossy,
      streamSeq: Number.isFinite(streamSeq) ? streamSeq : undefined,
      lastEventId: message.lastEventId,
      reason: lossy ? "lossy_replay" : "event"
    });
  };

  return {
    start() {
      stopped = false;
      for (const scope of scopes) void connect(scope);
      emit();
    },
    stop() {
      stopped = true;
      for (const source of sources.values()) source.close();
      sources.clear();
      for (const timer of timers.values()) clearTimer(timer);
      timers.clear();
    },
    snapshot() {
      return cloneLiveState(state);
    }
  };
}

function parseSseJson(value: unknown): unknown {
  if (typeof value !== "string") return {};
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return {};
  }
}

function cloneLiveState(state: DashboardLiveState): DashboardLiveState {
  return Object.fromEntries(
    Object.entries(state).map(([scope, scopeState]) => [scope, { ...scopeState }])
  ) as DashboardLiveState;
}
