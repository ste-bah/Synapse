import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Density = "comfortable" | "compact";
export type Theme = "dark" | "light";
export type DashboardRouteId = "fleet" | "agent" | "tasks" | "approvals" | "analytics" | "timeline" | "system" | "audit";

interface UiState {
  density: Density;
  theme: Theme;
  route: DashboardRouteId;
  savedViewId: string | null;
  selectedAgentId: string | null;
  setDensity: (density: Density) => void;
  setTheme: (theme: Theme) => void;
  setRoute: (route: DashboardRouteId) => void;
  setSavedViewId: (id: string | null) => void;
  setSelectedAgentId: (id: string | null) => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      density: "comfortable",
      theme: "dark",
      route: "fleet",
      savedViewId: null,
      selectedAgentId: null,
      setDensity: (density) => set({ density }),
      setTheme: (theme) => set({ theme }),
      setRoute: (route) => set({ route }),
      setSavedViewId: (savedViewId) => set({ savedViewId }),
      setSelectedAgentId: (selectedAgentId) => set({ selectedAgentId })
    }),
    {
      name: "synapse-command-center-ui",
      partialize: (state) => ({
        density: state.density,
        theme: state.theme,
        route: state.route,
        savedViewId: state.savedViewId
      })
    }
  )
);
