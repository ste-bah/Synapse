import { useMemo } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { App } from "@/app";
import { dashboardFixture } from "@/stories/fixtures";

const meta = {
  title: "Command Center/Fleet Overview",
  component: App,
  parameters: {
    layout: "fullscreen"
  }
} satisfies Meta<typeof App>;

export default meta;

type Story = StoryObj<typeof meta>;

function DashboardScenario({
  mode
}: {
  mode: "populated" | "empty" | "error" | "loading";
}) {
  const queryClient = useMemo(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            retry: false,
            refetchOnWindowFocus: false,
            refetchInterval: false,
            staleTime: Number.POSITIVE_INFINITY
          }
        }
      }),
    [mode]
  );

  globalThis.fetch = async () => {
    if (mode === "loading") return new Promise<Response>(() => {});
    if (mode === "error") return new Response(JSON.stringify({ code: "ISSUE947_STORY_ERROR" }), { status: 503 });
    return Response.json(dashboardFixture(mode));
  };

  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <App />
      </TooltipProvider>
    </QueryClientProvider>
  );
}

export const Populated: Story = {
  name: "Populated",
  parameters: { docs: { description: { story: "coverage:view:populated" } } },
  render: () => <DashboardScenario mode="populated" />
};

export const Empty: Story = {
  name: "Empty",
  parameters: { docs: { description: { story: "coverage:view:empty" } } },
  render: () => <DashboardScenario mode="empty" />
};

export const Loading: Story = {
  name: "Loading",
  parameters: { docs: { description: { story: "coverage:view:loading" } } },
  render: () => <DashboardScenario mode="loading" />
};

export const Error: Story = {
  name: "Error",
  parameters: { docs: { description: { story: "coverage:view:error" } } },
  render: () => <DashboardScenario mode="error" />
};
