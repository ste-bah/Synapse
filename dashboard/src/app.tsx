import { useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip as ChartTooltip, XAxis, YAxis } from "recharts";
import {
  BarChart3,
  Bell,
  CheckCircle2,
  ClipboardList,
  Command,
  Gauge,
  LogIn,
  LogOut,
  Moon,
  Plus,
  RefreshCw,
  Rocket,
  Rows3,
  Save,
  Search,
  ServerCog,
  ShieldCheck,
  Sun,
  Trash2,
  UserRound,
  X,
  type LucideIcon
} from "lucide-react";
import commandMarkUrl from "@/assets/synapse-command-mark.png";
import emptyStateUrl from "@/assets/synapse-empty-state.webp";
import statusIconsUrl from "@/assets/synapse-status-icons.webp";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { StatusBadge } from "@/components/ui/badge";
import {
  AgentPeek,
  AppShell,
  DataTable,
  EmptyState,
  FleetRow,
  MetricRow,
  PageHeader,
  RawValue,
  Section,
  StatCard,
  ToolCallCard,
  TranscriptTurn
} from "@/primitives";
import {
  buildAgents,
  buildToolCalls,
  deleteDashboardView,
  fetchDashboardAuthStatus,
  fetchDashboardState,
  fetchModels,
  fetchSavedViews,
  loginDashboard,
  logoutDashboard,
  panelData,
  registerApiModel,
  saveDashboardView,
  spawnLocalModelAgent,
  type AgentSummary,
  type DashboardAuthStatus,
  type DashboardRouteReadback,
  type DashboardSavedView,
  type DashboardState,
  type FleetStatus,
  type ModelRow
} from "@/lib/dashboard-state";
import { asArray, asRecord, cn, rawText, timeAgo, unixMsToTime } from "@/lib/utils";
import { useUiStore, type DashboardRouteId, type Density, type Theme } from "@/store/ui-store";

type RouteDefinition = {
  id: DashboardRouteId;
  label: string;
  title: string;
  icon: LucideIcon;
};

const routeDefinitions: RouteDefinition[] = [
  { id: "fleet", label: "Fleet", title: "Fleet Overview", icon: Gauge },
  { id: "agent", label: "Agent", title: "Agent Detail", icon: UserRound },
  { id: "tasks", label: "Tasks", title: "Task Board", icon: ClipboardList },
  { id: "approvals", label: "Approvals", title: "Approvals Inbox", icon: CheckCircle2 },
  { id: "analytics", label: "Analytics", title: "Cost & Token Analytics", icon: BarChart3 },
  { id: "timeline", label: "Timeline", title: "Timeline", icon: Rows3 },
  { id: "system", label: "System", title: "System Status", icon: ServerCog },
  { id: "audit", label: "Audit", title: "Audit Explorer", icon: ShieldCheck }
];

export function App() {
  const density = useUiStore((state) => state.density);
  const setDensity = useUiStore((state) => state.setDensity);
  const theme = useUiStore((state) => state.theme);
  const setTheme = useUiStore((state) => state.setTheme);
  const route = useUiStore((state) => state.route);
  const setRoute = useUiStore((state) => state.setRoute);
  const savedViewId = useUiStore((state) => state.savedViewId);
  const setSavedViewId = useUiStore((state) => state.setSavedViewId);
  const selectedAgentId = useUiStore((state) => state.selectedAgentId);
  const setSelectedAgentId = useUiStore((state) => state.setSelectedAgentId);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [hashRouteReady, setHashRouteReady] = useState(false);
  const [savedViewName, setSavedViewName] = useState("");
  const [savedViewError, setSavedViewError] = useState("");

  const authQuery = useQuery({
    queryKey: ["dashboard-auth"],
    queryFn: fetchDashboardAuthStatus,
    retry: false
  });
  const query = useQuery({
    queryKey: ["dashboard-state"],
    queryFn: fetchDashboardState,
    enabled: authQuery.data?.authenticated === true
  });
  const savedViewsQuery = useQuery({
    queryKey: ["dashboard-saved-views"],
    queryFn: fetchSavedViews,
    enabled: authQuery.data?.authenticated === true
  });

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    document.documentElement.dataset.density = density;
  }, [theme, density]);

  useEffect(() => {
    const syncFromHash = () => {
      const hashRoute = routeFromHash(window.location.hash);
      if (hashRoute && useUiStore.getState().route !== hashRoute) {
        setRoute(hashRoute);
      }
    };
    syncFromHash();
    setHashRouteReady(true);
    window.addEventListener("hashchange", syncFromHash);
    return () => window.removeEventListener("hashchange", syncFromHash);
  }, [setRoute]);

  useEffect(() => {
    if (!hashRouteReady) return;
    const expected = `#/${route}`;
    if (window.location.hash !== expected) {
      window.history.replaceState(null, "", expected);
    }
  }, [hashRouteReady, route]);

  useEffect(() => {
    let icon = document.querySelector<HTMLLinkElement>('link[rel="icon"][data-synapse-generated="true"]');
    if (!icon) {
      icon = document.createElement("link");
      icon.rel = "icon";
      icon.type = "image/png";
      icon.dataset.synapseGenerated = "true";
      document.head.appendChild(icon);
    }
    icon.href = commandMarkUrl;
  }, []);

  const agents = useMemo(() => buildAgents(query.data), [query.data]);
  const toolCalls = useMemo(() => buildToolCalls(query.data), [query.data]);
  const attentionAgents = useMemo(
    () => agents.filter((agent) => ["stuck", "needs_input", "awaiting_approval", "ready_for_review"].includes(agent.status)),
    [agents]
  );
  const selectedAgent = agents.find((agent) => agent.id === selectedAgentId) ?? attentionAgents[0] ?? agents[0];

  useEffect(() => {
    if (!selectedAgentId && selectedAgent) {
      setSelectedAgentId(selectedAgent.id);
    }
  }, [selectedAgentId, selectedAgent, setSelectedAgentId]);

  const attentionCount = attentionAgents.length;
  useEffect(() => {
    document.title = attentionCount ? `${attentionCount} awaiting input - Synapse Command Center` : "Synapse Command Center";
  }, [attentionCount]);

  const advanceAttention = () => {
    if (attentionAgents.length === 0) return;
    const current = attentionAgents.findIndex((agent) => agent.id === selectedAgent?.id);
    const next = attentionAgents[(current + 1 + attentionAgents.length) % attentionAgents.length];
    setSelectedAgentId(next.id);
    setRoute("agent");
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isEditableShortcutTarget(event.target)) return;
      const key = event.key.toLowerCase();
      const plainKey = !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
      if (((event.ctrlKey || event.metaKey) && key === "k") || (plainKey && (key === "/" || key === "p"))) {
        event.preventDefault();
        setPaletteOpen(true);
      }
      if ((event.altKey && key === "n") || (plainKey && key === "n")) {
        event.preventDefault();
        advanceAttention();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const state = query.data;
  const freshnessMs = state ? Date.now() - state.generated_at_unix_ms : undefined;
  const stale = query.isError || (freshnessMs !== undefined && freshnessMs > 10000);
  const activeRoute = routeDefinitions.find((item) => item.id === route) ?? routeDefinitions[0];

  const commands = useMemo(
    () => [
      ...routeDefinitions.map((item) => ({
        id: `route-${item.id}`,
        label: item.title,
        group: "Views",
        icon: item.icon,
        action: () => setRoute(item.id)
      })),
      {
        id: "attention-next",
        label: "Next attention",
        group: "Controls",
        icon: Bell,
        action: advanceAttention
      },
      {
        id: "refresh",
        label: "Refresh dashboard state",
        group: "Controls",
        icon: RefreshCw,
        action: () => {
          query.refetch();
          savedViewsQuery.refetch();
        }
      },
      ...agents.slice(0, 8).map((agent) => ({
        id: `agent-${agent.id}`,
        label: agent.id,
        group: "Agents",
        icon: UserRound,
        action: () => {
          setSelectedAgentId(agent.id);
          setRoute("agent");
        }
      }))
    ],
    [agents, query, savedViewsQuery, setRoute, setSelectedAgentId]
  );

  if (authQuery.data?.authenticated !== true) {
    return (
      <AppShell sidebar={<Sidebar route={route} setRoute={setRoute} state={state} auth={authQuery.data} />}>
        <LoginView
          auth={authQuery.data}
          pending={authQuery.isLoading}
          onAuthenticated={() => {
            authQuery.refetch();
            query.refetch();
            savedViewsQuery.refetch();
          }}
        />
      </AppShell>
    );
  }

  const applySavedView = (view?: DashboardSavedView) => {
    setSavedViewError("");
    if (!view) {
      setSavedViewId(null);
      setSavedViewName("");
      return;
    }
    setSavedViewId(view.view_id);
    setSavedViewName(view.name);
    if (isDashboardRouteId(view.route)) setRoute(view.route);
    const filters = asRecord(view.filters);
    const agentId = rawText(filters.selectedAgentId);
    if (agentId) setSelectedAgentId(agentId);
    if (isDensity(filters.density)) setDensity(filters.density);
    if (isTheme(filters.theme)) setTheme(filters.theme);
  };

  const saveCurrentView = async () => {
    const name = savedViewName.trim();
    if (!name) return;
    setSavedViewError("");
    try {
      const saved = await saveDashboardView({
        view_id: savedViewId || undefined,
        name,
        route,
        filters: {
          selectedAgentId,
          density,
          theme
        }
      });
      setSavedViewId(saved.view.view_id);
      setSavedViewName(saved.view.name);
      await savedViewsQuery.refetch();
    } catch (error) {
      setSavedViewError(rawText(error) || "Saved view write failed");
    }
  };

  const deleteCurrentView = async () => {
    if (!savedViewId) return;
    setSavedViewError("");
    try {
      await deleteDashboardView(savedViewId);
      setSavedViewId(null);
      setSavedViewName("");
      await savedViewsQuery.refetch();
    } catch (error) {
      setSavedViewError(rawText(error) || "Saved view delete failed");
    }
  };

  return (
    <AppShell sidebar={<Sidebar route={route} setRoute={setRoute} state={state} auth={authQuery.data} />}>
      <PageHeader
        title={activeRoute.title}
        subtitle={
          <span className={stale ? "text-warning-fg" : "text-secondary"}>
            {query.isError ? rawText(query.error) : `Updated ${freshnessMs === undefined ? "pending" : timeAgo(freshnessMs)} ago`}
          </span>
        }
        actions={
          <>
            <SavedViewControls
              views={savedViewsQuery.data?.views ?? []}
              selectedId={savedViewId}
              name={savedViewName}
              error={savedViewError}
              onNameChange={setSavedViewName}
              onSelect={applySavedView}
              onSave={saveCurrentView}
              onDelete={deleteCurrentView}
              saving={savedViewsQuery.isFetching}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon" variant="ghost" onClick={() => setPaletteOpen(true)} aria-label="Open command palette" aria-keyshortcuts="/ P Control+K">
                  <Command aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Command palette</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon" variant="ghost" onClick={() => query.refetch()} aria-label="Refresh dashboard state">
                  <RefreshCw aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Refresh</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  variant="ghost"
                  onClick={() =>
                    logoutDashboard().then(() => {
                      authQuery.refetch();
                    })
                  }
                  aria-label="Lock dashboard"
                >
                  <LogOut aria-hidden="true" className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Lock</TooltipContent>
            </Tooltip>
            <DensityControl density={density} setDensity={setDensity} />
            <label className="flex items-center gap-2 text-sm text-secondary">
              {theme === "dark" ? <Moon aria-hidden="true" className="h-4 w-4" /> : <Sun aria-hidden="true" className="h-4 w-4" />}
              <Switch checked={theme === "light"} onCheckedChange={(checked) => setTheme(checked ? "light" : "dark")} aria-label="Toggle light theme" />
            </label>
          </>
        }
      />

      {route === "fleet" ? (
        <FleetView
          state={state}
          agents={agents}
          attentionAgents={attentionAgents}
          selectedAgent={selectedAgent}
          setSelectedAgentId={setSelectedAgentId}
          attentionCount={attentionCount}
          stale={stale}
          toolCalls={toolCalls}
          advanceAttention={advanceAttention}
          onSpawned={() => query.refetch()}
        />
      ) : null}
      {route === "agent" ? <AgentView state={state} selectedAgent={selectedAgent} toolCalls={toolCalls} /> : null}
      {route === "tasks" ? <TasksView agents={agents} attentionCount={attentionCount} /> : null}
      {route === "approvals" ? <ApprovalsView state={state} /> : null}
      {route === "analytics" ? <AnalyticsView state={state} agents={agents} attentionCount={attentionCount} stale={stale} /> : null}
      {route === "timeline" ? <TimelineView state={state} toolCalls={toolCalls} /> : null}
      {route === "system" ? <SystemView state={state} stale={stale} /> : null}
      {route === "audit" ? <AuditView state={state} toolCalls={toolCalls} /> : null}

      <CommandPalette open={paletteOpen} commands={commands} onClose={() => setPaletteOpen(false)} />
    </AppShell>
  );
}

function LoginView({
  auth,
  pending,
  onAuthenticated
}: {
  auth?: DashboardAuthStatus;
  pending: boolean;
  onAuthenticated: () => void;
}) {
  const [credential, setCredential] = useState("");
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setSubmitting(true);
    try {
      await loginDashboard(credential);
      setCredential("");
      onAuthenticated();
    } catch (loginError) {
      setError(rawText(loginError) || "Access denied");
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <>
      <PageHeader
        title="Dashboard Access"
        subtitle={<span>{pending ? "Checking session" : auth?.authenticated ? "Session active" : "Session required"}</span>}
        actions={<span className="text-sm text-secondary">Loopback only</span>}
      />
      <Section
        title="Unlock"
        tier="overview"
        questions={["Is a dashboard session active?", "Can the operator mint a cookie session?", "Did the login fail closed?"]}
      >
        <form className="max-w-md space-y-3" onSubmit={submit}>
          <label className="block text-sm text-secondary">
            <span className="mb-1 block text-label font-medium uppercase text-muted">Access token</span>
            <input
              className="h-10 w-full rounded-md border border-border bg-surface-1 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              type="password"
              value={credential}
              autoComplete="off"
              onChange={(event) => setCredential(event.target.value)}
            />
          </label>
          {error ? <div className="text-sm text-danger-fg">{error}</div> : null}
          <Button type="submit" variant="primary" disabled={!credential.trim() || submitting}>
            <LogIn aria-hidden="true" className="h-4 w-4" />
            Unlock
          </Button>
        </form>
      </Section>
    </>
  );
}

function Sidebar({
  route,
  setRoute,
  state,
  auth
}: {
  route: DashboardRouteId;
  setRoute: (route: DashboardRouteId) => void;
  state?: DashboardState;
  auth?: DashboardAuthStatus;
}) {
  const health = asRecord(panelData(state?.daemon));
  return (
    <nav className="space-y-4" aria-label="Dashboard">
      <button type="button" className="flex w-full items-center gap-3 text-left" onClick={() => setRoute("fleet")}>
        <img src={commandMarkUrl} alt="" className="h-10 w-10 rounded-lg border border-border bg-surface-2 object-cover" />
        <span className="min-w-0">
          <span className="block text-md font-semibold text-primary">Synapse</span>
          <span className="block truncate text-xs text-muted">{rawText(health.version || "dashboard")}</span>
        </span>
      </button>
      <div className="grid gap-1">
        {routeDefinitions.map((item) => (
          <SidebarItem key={item.id} item={item} active={route === item.id} onSelect={() => setRoute(item.id)} />
        ))}
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Loopback</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{state?.bind_addr || "pending"}</div>
      </div>
      <div className="rounded-lg border border-border bg-surface-2 p-3">
        <div className="text-label font-medium uppercase text-muted">Auth</div>
        <div className="mt-1 truncate font-mono text-sm text-primary">{auth?.authenticated ? auth.method : "locked"}</div>
      </div>
      <img src={statusIconsUrl} alt="" className="w-full rounded-lg border border-border bg-surface-2 object-cover" />
    </nav>
  );
}

function SidebarItem({ item, active, onSelect }: { item: RouteDefinition; active: boolean; onSelect: () => void }) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "flex min-h-10 w-full items-center gap-2 rounded-md px-3 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring",
        active ? "bg-surface-2 text-primary" : "text-secondary hover:bg-surface-2 hover:text-primary"
      )}
    >
      <Icon aria-hidden="true" className="h-4 w-4" />
      {item.label}
    </button>
  );
}

function SavedViewControls({
  views,
  selectedId,
  name,
  error,
  onNameChange,
  onSelect,
  onSave,
  onDelete,
  saving
}: {
  views: DashboardSavedView[];
  selectedId: string | null;
  name: string;
  error: string;
  onNameChange: (value: string) => void;
  onSelect: (view?: DashboardSavedView) => void;
  onSave: () => void;
  onDelete: () => void;
  saving: boolean;
}) {
  return (
    <div className="flex max-w-full flex-wrap items-center gap-2">
      <label className="sr-only" htmlFor="saved-view-select">
        Saved view
      </label>
      <select
        id="saved-view-select"
        className="h-9 min-w-36 rounded-md border border-border bg-surface-1 px-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
        value={selectedId ?? ""}
        onChange={(event) => onSelect(views.find((view) => view.view_id === event.target.value))}
      >
        <option value="">Default view</option>
        {views.map((view) => (
          <option key={view.view_id} value={view.view_id}>
            {view.name}
          </option>
        ))}
      </select>
      <label className="sr-only" htmlFor="saved-view-name">
        View name
      </label>
      <input
        id="saved-view-name"
        className="h-9 w-36 rounded-md border border-border bg-surface-1 px-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
        value={name}
        placeholder="View name"
        onChange={(event) => onNameChange(event.target.value)}
      />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button size="icon" variant="ghost" onClick={onSave} disabled={!name.trim() || saving} aria-label="Save view">
            <Save aria-hidden="true" className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Save view</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button size="icon" variant="ghost" onClick={onDelete} disabled={!selectedId || saving} aria-label="Delete view">
            <Trash2 aria-hidden="true" className="h-4 w-4" />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Delete view</TooltipContent>
      </Tooltip>
      {error ? <span className="max-w-56 truncate text-xs text-danger-fg">{error}</span> : null}
    </div>
  );
}

function DensityControl({
  density,
  setDensity
}: {
  density: Density;
  setDensity: (density: Density) => void;
}) {
  return (
    <div className="inline-flex rounded-lg border border-border bg-surface-1 p-1" aria-label="Density">
      {(["comfortable", "compact"] as const).map((value) => (
        <button
          key={value}
          type="button"
          onClick={() => setDensity(value)}
          className={`h-8 rounded-md px-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring ${density === value ? "bg-surface-2 text-primary" : "text-muted hover:text-primary"}`}
        >
          {value === "comfortable" ? "Comfort" : "Compact"}
        </button>
      ))}
    </div>
  );
}

function CommandPalette({
  open,
  commands,
  onClose
}: {
  open: boolean;
  commands: Array<{ id: string; label: string; group: string; icon: LucideIcon; action: () => void }>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (open) {
      setQuery("");
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);
  if (!open) return null;
  const filtered = commands.filter((command) => `${command.group} ${command.label}`.toLowerCase().includes(query.toLowerCase())).slice(0, 12);
  return (
    <div className="fixed inset-0 z-50 bg-black/60 p-4" role="dialog" aria-modal="true" aria-label="Command palette" onMouseDown={onClose}>
      <div className="mx-auto mt-16 max-w-2xl rounded-lg border border-border bg-surface-1 shadow-xl" onMouseDown={(event) => event.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search aria-hidden="true" className="h-4 w-4 text-muted" />
          <input
            ref={inputRef}
            className="h-10 min-w-0 flex-1 bg-transparent text-sm text-primary outline-none"
            value={query}
            placeholder="Search commands"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") onClose();
              if (event.key === "Enter" && filtered[0]) {
                filtered[0].action();
                onClose();
              }
            }}
          />
          <Button size="icon" variant="ghost" onClick={onClose} aria-label="Close command palette">
            <X aria-hidden="true" className="h-4 w-4" />
          </Button>
        </div>
        <div className="max-h-[60vh] overflow-auto p-2">
          {filtered.length ? (
            filtered.map((command) => {
              const Icon = command.icon;
              return (
                <button
                  key={command.id}
                  type="button"
                  className="flex min-h-11 w-full items-center gap-3 rounded-md px-3 text-left text-sm text-secondary hover:bg-surface-2 hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus-ring"
                  onClick={() => {
                    command.action();
                    onClose();
                  }}
                >
                  <Icon aria-hidden="true" className="h-4 w-4 text-info" />
                  <span className="min-w-0 flex-1 truncate">{command.label}</span>
                  <span className="text-label uppercase text-muted">{command.group}</span>
                </button>
              );
            })
          ) : (
            <EmptyState title="No matching commands" />
          )}
        </div>
      </div>
    </div>
  );
}

function FleetView({
  state,
  agents,
  attentionAgents,
  selectedAgent,
  setSelectedAgentId,
  attentionCount,
  stale,
  toolCalls,
  advanceAttention,
  onSpawned
}: {
  state?: DashboardState;
  agents: AgentSummary[];
  attentionAgents: AgentSummary[];
  selectedAgent?: AgentSummary;
  setSelectedAgentId: (id: string) => void;
  attentionCount: number;
  stale: boolean;
  toolCalls: ReturnType<typeof buildToolCalls>;
  advanceAttention: () => void;
  onSpawned: () => void;
}) {
  return (
    <>
      <OverviewBand state={state} agents={agents} attentionCount={attentionCount} stale={stale} />
      <Section
        title="Spawn Console"
        tier="triage"
        questions={["Which models can I launch right now?", "How do I add a cloud API model like DeepSeek?", "Did the spawn succeed, step by step?"]}
      >
        <SpawnConsole onSpawned={onSpawned} />
      </Section>
      <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_minmax(20rem,0.42fr)]">
        <div className="min-w-0">
          <Section
            title="Attention Groups"
            tier="triage"
            questions={["Which agents need a human now?", "Which session should I inspect first?", "What changed since the last refresh?"]}
            actions={
              <Button variant="secondary" size="sm" onClick={advanceAttention} disabled={attentionAgents.length === 0} aria-keyshortcuts="N Alt+N">
                <Bell aria-hidden="true" className="h-4 w-4" />
                Next
              </Button>
            }
          >
            <FleetList agents={attentionAgents.length ? attentionAgents : agents} selectedId={selectedAgent?.id} onSelect={setSelectedAgentId} />
          </Section>
          <ToolActivity toolCalls={toolCalls} />
          <Section
            title="Fleet Table"
            tier="drill-down"
            questions={["Which sessions are live?", "Which rows are stale?", "Which row links to detail?"]}
          >
            <FleetTable agents={agents} onSelect={setSelectedAgentId} />
          </Section>
        </div>
        <aside className="min-w-0">
          <Section
            title="Peek Panel"
            tier="drill-down"
            questions={["Why is this agent in its current state?", "Which detail surface proves it?", "Is raw verification available without flooding the page?"]}
          >
            <AgentPeek agent={selectedAgent} />
          </Section>
          <Section
            title="System Shape"
            tier="overview"
            questions={["Is storage pressure rising?", "Which column family is largest?", "Is the daemon still local?"]}
          >
            <SystemShape state={state} />
          </Section>
        </aside>
      </div>
      <TranscriptSamples state={state} />
    </>
  );
}

function AgentView({ state, selectedAgent, toolCalls }: { state?: DashboardState; selectedAgent?: AgentSummary; toolCalls: ReturnType<typeof buildToolCalls> }) {
  return (
    <div className="grid gap-6 xl:grid-cols-[minmax(0,0.42fr)_minmax(0,1fr)]">
      <Section
        title="Agent Surfaces"
        tier="drill-down"
        questions={["Which agent is selected?", "Which state explains the current badge?", "Can raw verification stay collapsed?"]}
      >
        <AgentPeek agent={selectedAgent} />
      </Section>
      <div className="min-w-0">
        <ToolActivity toolCalls={toolCalls} />
        <TranscriptSamples state={state} />
      </div>
    </div>
  );
}

function TasksView({ agents, attentionCount }: { agents: AgentSummary[]; attentionCount: number }) {
  const reviewAgents = agents.filter((agent) => agent.status === "ready_for_review").length;
  return (
    <>
      <Section
        title="Task Queue"
        tier="overview"
        questions={["How many review items need action?", "Which rows are attention candidates?", "Is the shell route stable for #924?"]}
      >
        <div className="grid gap-4 md:grid-cols-3">
          <StatCard label="Attention" value={attentionCount} status={attentionCount ? "needs_input" : "done"} />
          <StatCard label="Review" value={reviewAgents} status={reviewAgents ? "ready_for_review" : "idle"} />
          <StatCard label="Shell" value="ready" status="working" />
        </div>
      </Section>
      <Section
        title="Board"
        tier="triage"
        questions={["Which task lane is populated?", "Which attempt needs review?", "Where will queue transitions appear?"]}
      >
        <EmptyStateArt title="No task board rows in this shell feed" />
      </Section>
    </>
  );
}

function ApprovalsView({ state }: { state?: DashboardState }) {
  return (
    <div className="grid gap-6 xl:grid-cols-3">
      <ApprovalPanel title="Approvals" panel={state?.approvals} />
      <ApprovalPanel title="Suggestions" panel={state?.suggestions} />
      <ApprovalPanel title="Armed Runs" panel={state?.armed_runs} />
    </div>
  );
}

function ApprovalPanel({ title, panel }: { title: string; panel?: DashboardState["approvals"] }) {
  const data = asRecord(panelData(panel));
  const rows = asArray<Record<string, unknown>>(data.rows);
  return (
    <Section
      title={title}
      tier="triage"
      questions={["Which approvals are pending?", "What source row backs this list?", "Does raw detail stay collapsed?"]}
    >
      <div className="space-y-3">
        <StatCard label="Rows" value={rows.length} status={rows.length ? "awaiting_approval" : "done"} delta={rawText(data.tool || panel?.source)} />
        {rows.length ? rows.slice(0, 4).map((row, index) => <RawValue key={index} value={row} label="Approval row" />) : <EmptyStateArt title="No approval rows" />}
      </div>
    </Section>
  );
}

function AnalyticsView({ state, agents, attentionCount, stale }: { state?: DashboardState; agents: AgentSummary[]; attentionCount: number; stale: boolean }) {
  return (
    <>
      <OverviewBand state={state} agents={agents} attentionCount={attentionCount} stale={stale} />
      <Section
        title="Storage Distribution"
        tier="triage"
        questions={["Which store has the most rows?", "Is pressure elevated?", "Which counters changed since refresh?"]}
      >
        <SystemShape state={state} />
      </Section>
    </>
  );
}

function TimelineView({ state, toolCalls }: { state?: DashboardState; toolCalls: ReturnType<typeof buildToolCalls> }) {
  return (
    <>
      <ToolActivity toolCalls={toolCalls} />
      <TranscriptSamples state={state} />
    </>
  );
}

function SystemView({ state, stale }: { state?: DashboardState; stale: boolean }) {
  return (
    <>
      <Section
        title="Daemon"
        tier="overview"
        questions={["Is the daemon healthy?", "Is the dashboard stale?", "Which process owns the server?"]}
      >
        <div className="grid gap-4 md:grid-cols-3">
          <StatCard label="Freshness" value={stale ? "stale" : "live"} status={stale ? "stuck" : "working"} />
          <StatCard label="Bind" value={state?.bind_addr || "pending"} status="working" />
          <StatCard label="Auth" value={rawText(asRecord(panelData(state?.auth)).active_session_count || 0)} status="done" />
        </div>
      </Section>
      <Section
        title="System Shape"
        tier="triage"
        questions={["Which resource is largest?", "Which counters support that read?", "Can the raw readback be inspected?"]}
      >
        <SystemShape state={state} />
        <div className="mt-3">
          <RawValue value={{ daemon: state?.daemon, storage: state?.storage, lease: state?.lease }} label="System readback" />
        </div>
      </Section>
    </>
  );
}

function AuditView({ state, toolCalls }: { state?: DashboardState; toolCalls: ReturnType<typeof buildToolCalls> }) {
  return (
    <>
      <ToolActivity toolCalls={toolCalls} />
      <Section
        title="Command Audit"
        tier="drill-down"
        questions={["Which audit rows are visible?", "Which row source backs the panel?", "Does raw detail stay collapsed?"]}
      >
        <RawValue value={state?.command_audit} label="Command audit readback" />
      </Section>
    </>
  );
}

function OverviewBand({
  state,
  agents,
  attentionCount,
  stale
}: {
  state?: DashboardState;
  agents: AgentSummary[];
  attentionCount: number;
  stale: boolean;
}) {
  const health = asRecord(panelData(state?.daemon));
  const storage = asRecord(panelData(state?.storage));
  const storagePressure = rawText(asRecord(storage.pressure_level).name || asRecord(storage.pressure_level).value || "unknown");
  const liveAgents = agents.filter((agent) => agent.lifecycle === "live").length;
  const toolCount = Number(health.tool_count || 0);
  return (
    <Section title="Overview" tier="overview" questions={["Is anything wrong?", "How many agents are live?", "Is the daemon stale?"]}>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <StatCard label="Attention" value={attentionCount} status={attentionCount ? "needs_input" : "done"} delta={attentionCount ? "human review queued" : "quiet"} />
        <StatCard label="Live Agents" value={liveAgents} status={liveAgents ? "working" : "idle"} delta={`${agents.length} total rows`} />
        <StatCard label="Tools" value={toolCount} status={toolCount ? "done" : "stuck"} delta="strict client surface" />
        <StatCard label="Freshness" value={stale ? "stale" : "live"} status={stale ? "stuck" : "working"} delta={storagePressure} />
      </div>
    </Section>
  );
}

const deepSeekPresets = {
  flash: {
    label: "DeepSeek Flash",
    name: "deepseek-flash",
    base_url: "",
    model_id: "deepseek-v4-flash",
    runtime_preset: "deepseek_v4_flash_non_thinking",
    api_key_env_var: "DEEPSEEK_API_KEY",
    api_key: "",
    context_length: "1000000",
    max_tools: "128",
    notes: "DeepSeek V4 Flash non-thinking API agent"
  },
  reasoning: {
    label: "DeepSeek Reasoning",
    name: "deepseek-reasoning",
    base_url: "",
    model_id: "deepseek-v4-flash",
    runtime_preset: "deepseek_v4_reasoning",
    api_key_env_var: "DEEPSEEK_API_KEY",
    api_key: "",
    context_length: "1000000",
    max_tools: "128",
    notes: "DeepSeek V4 Flash reasoning API agent"
  }
};

function SpawnConsole({ onSpawned }: { onSpawned: () => void }) {
  const modelsQuery = useQuery({
    queryKey: ["dashboard-models"],
    queryFn: fetchModels
  });
  const models = modelsQuery.data ?? [];
  const [selectedModel, setSelectedModel] = useState("");
  const [prompt, setPrompt] = useState('Use workspace_put with key issue985-deepseek-smoke and value {"ok":true}.');
  const [workingDir, setWorkingDir] = useState("C:\\code\\Synapse");
  const [registerForm, setRegisterForm] = useState(deepSeekPresets.flash);
  const [pendingAction, setPendingAction] = useState<"register" | "spawn" | "">("");
  const [error, setError] = useState("");
  const [lastRegister, setLastRegister] = useState<DashboardRouteReadback | null>(null);
  const [lastSpawn, setLastSpawn] = useState<DashboardRouteReadback | null>(null);

  useEffect(() => {
    if (!selectedModel && models.length > 0) {
      const firstLaunchable = models.find((model) => model.enabled && model.last_probe?.healthy) ?? models[0];
      setSelectedModel(firstLaunchable.name);
    }
  }, [models, selectedModel]);

  const selected = models.find((model) => model.name === selectedModel);
  const canSpawn = Boolean(selectedModel && prompt.trim() && !pendingAction);

  const submitRegister = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setPendingAction("register");
    try {
      const readback = await registerApiModel({
        name: registerForm.name,
        base_url: registerForm.base_url,
        model_id: registerForm.model_id,
        runtime_preset: registerForm.runtime_preset,
        api_key_env_var: registerForm.api_key_env_var,
        api_key: registerForm.api_key.trim() ? registerForm.api_key : undefined,
        context_length: parsePositiveInteger(registerForm.context_length, "context_length"),
        max_tools: parsePositiveInteger(registerForm.max_tools, "max_tools"),
        notes: registerForm.notes,
        probe_timeout_ms: 30000
      });
      setLastRegister(readback);
      const row = asRecord(asRecord(readback.register).row) as unknown as ModelRow;
      if (row.name) setSelectedModel(row.name);
      await modelsQuery.refetch();
    } catch (registerError) {
      setError(rawText(registerError) || "API model registration failed");
    } finally {
      setPendingAction("");
    }
  };

  const submitSpawn = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError("");
    setPendingAction("spawn");
    try {
      const readback = await spawnLocalModelAgent({
        model_ref: selectedModel,
        prompt,
        working_dir: workingDir.trim() || undefined,
        wait_timeout_ms: 300000,
        hold_open_ms: 0
      });
      setLastSpawn(readback);
      await modelsQuery.refetch();
      onSpawned();
    } catch (spawnError) {
      setError(rawText(spawnError) || "Local-model spawn failed");
    } finally {
      setPendingAction("");
    }
  };

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(24rem,1.1fr)]">
      <div className="min-w-0 space-y-4">
        <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0">
              <h3 className="text-md font-semibold tracking-normal text-primary">Models</h3>
              <p className="mt-1 text-sm text-secondary">{modelsQuery.isFetching ? "Refreshing registry" : `${models.length} registry rows`}</p>
            </div>
            <Button size="sm" variant="ghost" onClick={() => modelsQuery.refetch()} disabled={modelsQuery.isFetching}>
              <RefreshCw aria-hidden="true" className="h-4 w-4" />
              Refresh
            </Button>
          </div>
          {models.length ? (
            <DataTable
              data={models}
              getRowId={(model) => model.name}
              columns={[
                {
                  id: "status",
                  header: "Status",
                  cell: ({ row }) => <StatusBadge status={modelFleetStatus(row.original)} />
                },
                {
                  accessorKey: "name",
                  header: "Name",
                  cell: ({ row }) => <span className="font-mono text-primary">{row.original.name}</span>
                },
                { accessorKey: "model_id", header: "Model" },
                {
                  id: "runtime",
                  header: "Runtime",
                  cell: ({ row }) => <span className="font-mono">{row.original.runtime_preset || "open_ai_compatible"}</span>
                },
                {
                  id: "env",
                  header: "Key env",
                  cell: ({ row }) => <span className="font-mono">{row.original.api_key_env_var || "none"}</span>
                },
                {
                  id: "key",
                  header: "API key",
                  cell: ({ row }) =>
                    row.original.has_api_key_secret ? (
                      <span className="font-mono text-success" title="Encrypted API key stored at rest (DPAPI)">🔑 stored</span>
                    ) : row.original.api_key_env_var ? (
                      <span className="font-mono text-warning" title="No stored key; resolves from daemon environment">env only</span>
                    ) : (
                      <span className="font-mono text-muted">none</span>
                    )
                }
              ]}
            />
          ) : (
            <EmptyStateArt title={modelsQuery.isError ? rawText(modelsQuery.error) || "Model registry unavailable" : "No model rows"} />
          )}
        </div>

        <form className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]" onSubmit={submitSpawn}>
          <div className="mb-3 flex items-center gap-2">
            <Rocket aria-hidden="true" className="h-4 w-4 text-info" />
            <h3 className="text-md font-semibold tracking-normal text-primary">Spawn</h3>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Model</span>
              <select
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                value={selectedModel}
                onChange={(event) => setSelectedModel(event.target.value)}
              >
                {models.map((model) => (
                  <option key={model.name} value={model.name}>
                    {model.name}
                  </option>
                ))}
              </select>
            </label>
            <label className="block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Working dir</span>
              <input
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 font-mono text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                value={workingDir}
                onChange={(event) => setWorkingDir(event.target.value)}
              />
            </label>
          </div>
          <label className="mt-3 block text-sm text-secondary">
            <span className="mb-1 block text-label font-medium uppercase text-muted">Prompt</span>
            <textarea
              className="min-h-28 w-full rounded-md border border-border bg-surface-2 px-3 py-2 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
            />
          </label>
          <div className="mt-3 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0 text-sm text-secondary">
              {selected ? `${selected.model_id} / ${selected.last_probe?.status || "unprobed"}` : "No model selected"}
            </div>
            <Button type="submit" variant="primary" disabled={!canSpawn}>
              <Rocket aria-hidden="true" className="h-4 w-4" />
              {pendingAction === "spawn" ? "Spawning" : "Spawn"}
            </Button>
          </div>
        </form>
      </div>

      <div className="min-w-0 space-y-4">
        <form className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]" onSubmit={submitRegister}>
          <div className="mb-3 flex items-center gap-2">
            <Plus aria-hidden="true" className="h-4 w-4 text-info" />
            <h3 className="text-md font-semibold tracking-normal text-primary">Add API Model</h3>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            <label className="mt-3 block text-sm text-secondary">
              <span className="mb-1 block text-label font-medium uppercase text-muted">Preset</span>
              <select
                className="h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring"
                value={registerForm.runtime_preset}
                onChange={(event) => {
                  const preset = Object.values(deepSeekPresets).find((item) => item.runtime_preset === event.target.value);
                  if (preset) setRegisterForm(preset);
                }}
              >
                {Object.values(deepSeekPresets).map((preset) => (
                  <option key={preset.runtime_preset} value={preset.runtime_preset}>
                    {preset.label}
                  </option>
                ))}
              </select>
            </label>
            <TextField label="Name" value={registerForm.name} onChange={(value) => setRegisterForm((form) => ({ ...form, name: value }))} />
            <TextField label="Model" value={registerForm.model_id} onChange={(value) => setRegisterForm((form) => ({ ...form, model_id: value }))} />
            <TextField label="Base URL" value={registerForm.base_url} onChange={(value) => setRegisterForm((form) => ({ ...form, base_url: value }))} mono />
            <TextField label="Key env" value={registerForm.api_key_env_var} onChange={(value) => setRegisterForm((form) => ({ ...form, api_key_env_var: value }))} mono />
            <TextField
              label="API key (stored encrypted)"
              value={registerForm.api_key}
              onChange={(value) => setRegisterForm((form) => ({ ...form, api_key: value }))}
              type="password"
              autoComplete="off"
              placeholder="sk-… (encrypted at rest via Windows DPAPI)"
              mono
            />
            <TextField label="Context" value={registerForm.context_length} onChange={(value) => setRegisterForm((form) => ({ ...form, context_length: value }))} mono />
            <TextField label="Max tools" value={registerForm.max_tools} onChange={(value) => setRegisterForm((form) => ({ ...form, max_tools: value }))} mono />
          </div>
          <TextField label="Notes" value={registerForm.notes} onChange={(value) => setRegisterForm((form) => ({ ...form, notes: value }))} />
          <div className="mt-3 flex justify-end">
            <Button type="submit" variant="secondary" disabled={pendingAction === "register"}>
              <Plus aria-hidden="true" className="h-4 w-4" />
              {pendingAction === "register" ? "Registering" : "Register"}
            </Button>
          </div>
        </form>
        {error ? <div className="rounded-lg border border-danger-border bg-danger-bg p-3 text-sm text-danger-fg">{error}</div> : null}
        {lastRegister ? <RawValue value={lastRegister} label="Register readback" /> : null}
        {lastSpawn ? <RawValue value={lastSpawn} label="Spawn readback" /> : null}
      </div>
    </div>
  );
}

function TextField({
  label,
  value,
  onChange,
  mono = false,
  type = "text",
  placeholder,
  autoComplete
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  mono?: boolean;
  type?: "text" | "password";
  placeholder?: string;
  autoComplete?: string;
}) {
  return (
    <label className="mt-3 block text-sm text-secondary">
      <span className="mb-1 block text-label font-medium uppercase text-muted">{label}</span>
      <input
        className={`h-10 w-full rounded-md border border-border bg-surface-2 px-3 text-sm text-primary outline-none focus:ring-2 focus:ring-focus-ring ${mono ? "font-mono" : ""}`}
        type={type}
        placeholder={placeholder}
        autoComplete={autoComplete}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function parsePositiveInteger(value: string, field: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${field} must be a positive integer`);
  }
  return parsed;
}

function modelFleetStatus(model: ModelRow): FleetStatus {
  if (!model.enabled) return "idle";
  if (model.last_probe?.healthy) return "done";
  if (model.last_probe) return "stuck";
  return "needs_input";
}

function FleetList({
  agents,
  selectedId,
  onSelect
}: {
  agents: AgentSummary[];
  selectedId?: string;
  onSelect: (id: string) => void;
}) {
  if (agents.length === 0) return <EmptyStateArt title="No agent rows" />;
  return (
    <div className="rounded-lg border border-border bg-surface-1">
      {agents.map((agent) => (
        <FleetRow key={agent.id} agent={agent} selected={agent.id === selectedId} onSelect={() => onSelect(agent.id)} />
      ))}
    </div>
  );
}

function FleetTable({ agents, onSelect }: { agents: AgentSummary[]; onSelect: (id: string) => void }) {
  if (agents.length === 0) return <EmptyStateArt title="No fleet rows" />;
  return (
    <DataTable
      data={agents}
      getRowId={(agent) => agent.id}
      columns={[
        {
          id: "status",
          header: "Status",
          cell: ({ row }) => <StatusBadge status={row.original.status} />
        },
        {
          accessorKey: "id",
          header: "Agent",
          cell: ({ row }) => (
            <button className="truncate text-left text-primary underline-offset-4 hover:underline" type="button" onClick={() => onSelect(row.original.id)}>
              {row.original.id}
            </button>
          )
        },
        { accessorKey: "kind", header: "Kind" },
        { accessorKey: "lifecycle", header: "Lifecycle" },
        {
          id: "summary",
          header: "Summary",
          cell: ({ row }) => <span className="line-clamp-2">{row.original.summary}</span>
        },
        {
          id: "diff",
          header: "Diff",
          cell: ({ row }) => `${row.original.diffStats.actions}/${row.original.diffStats.transcripts}`
        }
      ]}
    />
  );
}

function ToolActivity({ toolCalls }: { toolCalls: ReturnType<typeof buildToolCalls> }) {
  return (
    <Section title="Tool Activity" tier="triage" questions={["Which tools are still running?", "Which calls failed?", "Where is the verification detail?"]}>
      {toolCalls.length ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {toolCalls.slice(0, 6).map((call) => (
            <ToolCallCard call={call} key={call.id} />
          ))}
        </div>
      ) : (
        <EmptyStateArt title="No command audit rows" />
      )}
    </Section>
  );
}

function SystemShape({ state }: { state?: DashboardState }) {
  const storage = asRecord(panelData(state?.storage));
  const counts = asRecord(storage.cf_row_counts);
  const chartData = Object.entries(counts)
    .map(([name, value]) => ({ name: name.replace("CF_", ""), rows: Number(value) || 0 }))
    .sort((a, b) => b.rows - a.rows)
    .slice(0, 8);
  if (!chartData.length) return <EmptyStateArt title="No storage rows" />;
  return (
    <div className="space-y-4">
      <div className="h-64 rounded-lg border border-border bg-surface-1 p-3">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={chartData} margin={{ top: 8, right: 8, bottom: 8, left: 8 }}>
            <CartesianGrid stroke="var(--border-subtle)" vertical={false} />
            <XAxis dataKey="name" stroke="var(--text-muted)" tickLine={false} axisLine={false} />
            <YAxis stroke="var(--text-muted)" tickLine={false} axisLine={false} />
            <ChartTooltip contentStyle={{ background: "var(--surface-3)", border: "1px solid var(--border)", color: "var(--text-primary)" }} />
            <Bar dataKey="rows" fill="var(--info)" radius={[4, 4, 0, 0]} />
          </BarChart>
        </ResponsiveContainer>
      </div>
      <div className="rounded-lg border border-border bg-surface-1 p-[var(--density-card-padding)]">
        <MetricRow label="Schema" value={rawText(storage.schema_version)} />
        <MetricRow label="Policy count" value={rawText(storage.audit_retention_policy_count)} />
        <MetricRow label="Generated" value={unixMsToTime(state?.generated_at_unix_ms)} />
      </div>
    </div>
  );
}

function TranscriptSamples({ state }: { state?: DashboardState }) {
  const rows = asArray<Record<string, unknown>>(asRecord(panelData(state?.agent_transcripts)).rows).slice(0, 4);
  return (
    <Section title="Transcript Samples" tier="drill-down" questions={["What did recent agents say?", "Was output sanitized before render?", "Where is the source row?"]}>
      {rows.length ? (
        <div className="grid gap-3 lg:grid-cols-2">
          {rows.map((row, index) => (
            <TranscriptTurn key={`${rawText(row.spawn_id)}-${rawText(row.line_no)}-${index}`} row={row} />
          ))}
        </div>
      ) : (
        <EmptyStateArt title="No transcript rows" />
      )}
    </Section>
  );
}

function EmptyStateArt({ title }: { title: string }) {
  return (
    <div className="overflow-hidden rounded-lg border border-border bg-surface-1">
      <img src={emptyStateUrl} alt="" className="h-32 w-full object-cover opacity-80" />
      <div className="border-t border-border-subtle p-3">
        <EmptyState title={title} />
      </div>
    </div>
  );
}

function routeFromHash(hash: string): DashboardRouteId | null {
  const raw = hash.replace(/^#\/?/, "").split(/[/?]/)[0];
  return isDashboardRouteId(raw) ? raw : null;
}

function isEditableShortcutTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tagName = target.tagName.toLowerCase();
  return tagName === "input" || tagName === "textarea" || tagName === "select";
}

function isDashboardRouteId(value: unknown): value is DashboardRouteId {
  return typeof value === "string" && routeDefinitions.some((item) => item.id === value);
}

function isDensity(value: unknown): value is Density {
  return value === "comfortable" || value === "compact";
}

function isTheme(value: unknown): value is Theme {
  return value === "dark" || value === "light";
}
