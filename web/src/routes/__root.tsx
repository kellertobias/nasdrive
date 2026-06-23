import { createRootRoute, Outlet } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import api from "../api/client";
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
  const { isLoading } = useQuery({
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
      <Outlet />
    </>
  );
}
