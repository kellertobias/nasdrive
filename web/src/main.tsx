import { StrictMode } from "react";
import ReactDOM from "react-dom/client";
import { RouterProvider, createRouter } from "@tanstack/react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { routeTree } from "./routeTree.gen";

import "./styles/globals.css";
import "video.js/dist/video-js.css";
import "./styles/media-player.css";

// Create TanStack Query client
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      refetchOnWindowFocus: false,
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

const rootElement = document.getElementById("root")!;
if (!rootElement.innerHTML) {
  const root = ReactDOM.createRoot(rootElement);
  root.render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </StrictMode>,
  );
}
