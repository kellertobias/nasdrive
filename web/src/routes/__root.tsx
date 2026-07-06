import {
  createRootRoute,
  Outlet,
  useRouterState,
} from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import api, { ApiError, formatApiError } from "../api/client";
import { Icon } from "../components/Icon";

export const Route = createRootRoute({
  component: RootLayout,
});

function DevModeBanner() {
  const { data } = useQuery({
    queryKey: ["auth-config"],
    queryFn: api.authConfig,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  if (!data?.dev_auth_bypass) {
    return null;
  }

  return (
    <div
      role="alert"
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 8,
        width: "100%",
        padding: "8px 16px",
        background: "var(--color-danger)",
        color: "var(--color-danger-fg)",
        fontWeight: 600,
        fontSize: 13,
        textAlign: "center",
        letterSpacing: "0.01em",
        zIndex: 1000,
      }}
    >
      <Icon name="alertTriangle" size={16} color="var(--color-danger-fg)" />
      <span>
        Dev mode is active — authentication is bypassed and every request runs
        as the configured dev user. Do not expose this instance publicly.
      </span>
    </div>
  );
}

function RootLayout() {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const { error, isLoading } = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  if (isLoading) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          minHeight: "100vh",
          background: "var(--color-bg)",
        }}
      >
        <div
          className="shimmer"
          style={{
            width: 200,
            height: 20,
            borderRadius: "var(--radius-md)",
          }}
        />
      </div>
    );
  }

  return (
    <>
      <DevModeBanner />
      {shouldShowGlobalApiError(error, pathname) && (
        <GlobalApiErrorBanner error={error} />
      )}
      <Outlet />
    </>
  );
}

function shouldShowGlobalApiError(error: unknown, pathname: string) {
  if (!error) return false;
  if (
    error instanceof ApiError &&
    error.status === 401 &&
    (pathname === "/" || pathname === "/share-target" || pathname.startsWith("/s/"))
  ) {
    return false;
  }
  return true;
}

function GlobalApiErrorBanner({ error }: { error: unknown }) {
  return (
    <div
      role="alert"
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        gap: 8,
        width: "100%",
        padding: "8px 16px",
        background: "var(--color-bg-muted)",
        color: "var(--color-fg)",
        borderBottom: "1px solid var(--color-warning)",
        fontWeight: 600,
        fontSize: 13,
        textAlign: "center",
        zIndex: 1000,
      }}
    >
      <Icon name="alertTriangle" size={16} color="var(--color-warning)" />
      <span>{formatApiError(error)}</span>
    </div>
  );
}
