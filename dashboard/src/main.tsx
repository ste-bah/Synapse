import { Component, StrictMode, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { App } from "@/app";
import "@/styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchInterval: 3000,
      refetchOnWindowFocus: true,
      staleTime: 2500,
      retry: 1
    }
  }
});

type BootErrorBoundaryProps = {
  children: ReactNode;
};

type BootErrorBoundaryState = {
  error: Error | null;
  componentStack: string;
};

class BootErrorBoundary extends Component<BootErrorBoundaryProps, BootErrorBoundaryState> {
  state: BootErrorBoundaryState = { error: null, componentStack: "" };

  static getDerivedStateFromError(error: Error): BootErrorBoundaryState {
    return { error, componentStack: "" };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ error, componentStack: info.componentStack ?? "" });
  }

  render() {
    if (!this.state.error) return this.props.children;
    return <BootFailure error={this.state.error} componentStack={this.state.componentStack} />;
  }
}

function BootFailure({ error, componentStack = "" }: { error: unknown; componentStack?: string }) {
  const message = error instanceof Error ? error.message : String(error);
  const stack = error instanceof Error ? error.stack : "";
  return (
    <main className="min-h-screen bg-surface-0 p-6 text-primary">
      <section className="mx-auto max-w-4xl rounded-lg border border-danger-border bg-danger-bg p-4">
        <h1 className="text-lg font-semibold text-danger-fg">Dashboard boot failed</h1>
        <pre className="mt-3 max-h-[70vh] overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-danger-fg">
          {[message, stack, componentStack].filter(Boolean).join("\n\n")}
        </pre>
      </section>
    </main>
  );
}

function renderBootFailure(error: unknown) {
  const root = document.getElementById("root");
  if (!root) return;
  createRoot(root).render(<BootFailure error={error} />);
}

function bootstrapDashboard() {
  const root = document.getElementById("root");
  if (!root) throw new Error("dashboard root element is missing");
  createRoot(root).render(
    <StrictMode>
      <BootErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <TooltipProvider>
            <App />
          </TooltipProvider>
        </QueryClientProvider>
      </BootErrorBoundary>
    </StrictMode>
  );
}

try {
  bootstrapDashboard();
} catch (error) {
  renderBootFailure(error);
}
