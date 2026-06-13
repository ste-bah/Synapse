import type { Decorator, Preview } from "@storybook/react-vite";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useUiStore } from "@/store/ui-store";
import "@/styles.css";

const withProviders: Decorator = (Story, context) => {
  const theme = context.globals.theme === "light" ? "light" : "dark";
  const density = context.globals.density === "compact" ? "compact" : "comfortable";
  document.documentElement.dataset.theme = theme;
  document.documentElement.dataset.density = density;
  useUiStore.setState({ theme, density });

  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        refetchOnWindowFocus: false,
        refetchInterval: false,
        staleTime: Number.POSITIVE_INFINITY
      }
    }
  });

  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <Story />
      </TooltipProvider>
    </QueryClientProvider>
  );
};

const preview: Preview = {
  decorators: [withProviders],
  globalTypes: {
    theme: {
      description: "Dashboard theme",
      defaultValue: "dark",
      toolbar: {
        icon: "mirror",
        items: ["dark", "light"]
      }
    },
    density: {
      description: "Dashboard density",
      defaultValue: "comfortable",
      toolbar: {
        icon: "component",
        items: ["comfortable", "compact"]
      }
    }
  },
  parameters: {
    a11y: {
      context: "body",
      options: {
        runOnly: {
          type: "tag",
          values: ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]
        }
      }
    },
    controls: {
      expanded: true
    }
  }
};

export default preview;
