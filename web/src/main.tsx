import { Component, StrictMode } from "react";
import type { ErrorInfo, ReactNode } from "react";
import ReactDOM from "react-dom/client";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { routeTree } from "./routeTree.gen";

import "./styles/globals.css";
import "video.js/dist/video-js.css";
import "./styles/media-player.css";

interface GlobalErrorBoundaryState {
  error: Error | null;
  info: ErrorInfo | null;
}

class GlobalErrorBoundary extends Component<
  { children: ReactNode },
  GlobalErrorBoundaryState
> {
  state: GlobalErrorBoundaryState = { error: null, info: null };

  static getDerivedStateFromError(error: Error): GlobalErrorBoundaryState {
    return { error, info: null };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ error, info });
  }

  render() {
    const { error, info } = this.state;

    if (!error) {
      return this.props.children;
    }

    const details = [error.stack || error.message, info?.componentStack]
      .filter(Boolean)
      .join("\n\n");

    return (
      <div
        role="alert"
        style={{
          minHeight: "100vh",
          background: "var(--color-bg)",
          color: "var(--color-fg)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "var(--space-4)",
        }}
      >
        <div
          style={{
            width: 680,
            maxWidth: "100%",
            border: "1px solid var(--color-border)",
            borderRadius: "var(--radius-lg)",
            background: "var(--color-bg)",
            boxShadow: "var(--shadow-xl)",
            padding: "var(--space-5)",
          }}
        >
          <h1
            style={{
              margin: "0 0 var(--space-2)",
              fontSize: "var(--text-xl)",
              fontWeight: 600,
            }}
          >
            The app crashed
          </h1>
          <p
            style={{
              margin: "0 0 var(--space-4)",
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-sm)",
              lineHeight: "var(--leading-base)",
            }}
          >
            {error.message || "An unexpected client error occurred."}
          </p>
          <pre
            style={{
              margin: 0,
              maxHeight: "50vh",
              overflow: "auto",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-bg-muted)",
              padding: "var(--space-3)",
              fontSize: "var(--text-xs)",
              lineHeight: "var(--leading-sm)",
            }}
          >
            {details}
          </pre>
        </div>
      </div>
    );
  }
}

// Create TanStack Query client
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: true,
      refetchOnReconnect: true,
      retry: 1,
    },
  },
});

// Create the router
const router = createRouter({
  routeTree,
  context: {},
});

// Register the router type for type safety
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

// Detect system theme preference and apply
function applyTheme() {
  const stored = localStorage.getItem("nasfiles-theme");
  if (stored === "dark") {
    document.documentElement.classList.add("dark");
  } else if (stored === "light") {
    document.documentElement.classList.add("light");
  }
  // Otherwise, let the CSS media query handle it
}
applyTheme();

if ("serviceWorker" in navigator) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("/sw.js").catch((err) => {
      console.warn("Service worker registration failed", err);
    });
  });
}

const rootElement = document.getElementById("root")!;
if (!rootElement.innerHTML) {
  const root = ReactDOM.createRoot(rootElement);
  root.render(
    <StrictMode>
      <GlobalErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </GlobalErrorBoundary>
    </StrictMode>,
  );
}
