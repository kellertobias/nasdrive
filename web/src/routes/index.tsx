import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import api from "../api/client";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { AppLogo } from "../components/AppLogo";
import { prepareRequestOptions, serializeCredential } from "../lib/webauthn";
import { storeTrustedTotp, trustedTotpProof } from "../lib/totp";
import { takePendingShareId } from "../lib/shareTarget";

export const Route = createFileRoute("/")({
  component: IndexPage,
});

function IndexPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totpCode, setTotpCode] = useState("");
  const [trustComputer, setTrustComputer] = useState(false);
  const [totpChallenge, setTotpChallenge] = useState<string | null>(null);
  const [error, setError] = useState("");
  const { data: user } = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
  });
  const { data: authConfig } = useQuery({
    queryKey: ["auth-config"],
    queryFn: api.authConfig,
    retry: false,
    staleTime: 60_000,
  });

  const afterLogin = () => {
    setError("");
    setTotpChallenge(null);
    queryClient.invalidateQueries({ queryKey: ["me"] });
  };

  // @tour authentication:10 The login form fires a mutation
  // The entry point for everything that follows. Before sending anything it calls
  // `trustedTotpProof(username)` to see whether this browser was previously marked as a
  // [trusted device](glossary:trusted-device), and passes the result as `trusted_device`.
  //
  // `onSuccess` branches on `result.requires_totp`: either it stashes `result.challenge_id`
  // for the second-factor prompt, or it calls `afterLogin()`, which invalidates the
  // `["me"]` query so the app re-fetches identity.

  const loginMutation = useMutation({
    mutationFn: async () => {
      const trusted_device = await trustedTotpProof(username);
      return api.localLogin({ username, password, trusted_device });
    },
    onSuccess: (result) => {
      if (result.requires_totp && result.challenge_id) {
        setTotpChallenge(result.challenge_id);
        setTotpCode("");
      } else {
        afterLogin();
      }
    },
    onError: (err) =>
      setError(err instanceof Error ? err.message : String(err)),
  });

  const totpMutation = useMutation({
    mutationFn: () =>
      api.localLoginTotp({
        challenge_id: totpChallenge || "",
        code: totpCode,
        trust_computer: trustComputer,
        device_label: navigator.userAgent.slice(0, 120),
      }),
    onSuccess: (result) => {
      if (result.trusted_device) {
        storeTrustedTotp(username, result.trusted_device);
      }
      afterLogin();
    },
    onError: (err) =>
      setError(err instanceof Error ? err.message : String(err)),
  });

  const passkeyMutation = useMutation({
    mutationFn: async () => {
      const options = await api.startPasskeyLogin(username);
      const credential = await navigator.credentials.get({
        publicKey: prepareRequestOptions(options),
      });
      return api.finishPasskeyLogin(serializeCredential(credential));
    },
    onSuccess: afterLogin,
    onError: (err) =>
      setError(err instanceof Error ? err.message : String(err)),
  });

  useEffect(() => {
    if (user && user.roots.length > 0) {
      const pendingShareId = takePendingShareId();
      if (pendingShareId) {
        navigate({
          to: "/share-target",
          search: { shareId: pendingShareId },
          replace: true,
        });
        return;
      }

      // Redirect to home root if present, else first available root
      const defaultRoot =
        user.roots.find((r) => r.kind === "home") || user.roots[0];
      navigate({
        to: "/r/$root/$",
        params: { root: defaultRoot.key, _splat: "" },
        replace: true,
      });
    }
  }, [user, navigate]);

  if (!user) {
    const mode = authConfig?.mode ?? "sso";
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          minHeight: "100vh",
          gap: "var(--space-6)",
          background: "var(--color-bg)",
        }}
      >
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--space-3)",
            marginBottom: "var(--space-2)",
          }}
        >
          <AppLogo size={76} wordmarkSize={28} />
        </div>
        {mode === "sso" ? (
          <a
            href="/auth/oidc/login"
            style={primaryLinkStyle}
            onMouseOver={(e) =>
              (e.currentTarget.style.background = "var(--color-accent-hover)")
            }
            onMouseOut={(e) =>
              (e.currentTarget.style.background = "var(--color-accent)")
            }
          >
            Sign in with SSO
          </a>
        ) : (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (totpChallenge) totpMutation.mutate();
              else loginMutation.mutate();
            }}
            style={{
              display: "grid",
              gap: "var(--space-3)",
              width: "min(360px, calc(100vw - 32px))",
            }}
          >
            <input
              value={username}
              disabled={!!totpChallenge}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="Username"
              autoComplete="username"
              style={inputStyle}
            />
            {!totpChallenge && (
              <input
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder="Password"
                type="password"
                autoComplete="current-password"
                style={inputStyle}
              />
            )}
            {totpChallenge && (
              <>
                <input
                  value={totpCode}
                  onChange={(e) => setTotpCode(e.target.value)}
                  placeholder="TOTP code"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  style={inputStyle}
                />
                <label
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    fontSize: "var(--text-sm)",
                    color: "var(--color-fg-muted)",
                  }}
                >
                  <input
                    type="checkbox"
                    checked={trustComputer}
                    onChange={(e) => setTrustComputer(e.target.checked)}
                  />
                  Trust this computer
                </label>
              </>
            )}
            <button
              type="submit"
              disabled={
                !username ||
                (!totpChallenge && !password) ||
                loginMutation.isPending ||
                totpMutation.isPending
              }
              style={buttonStyle}
            >
              <Icon name="user" size={16} />
              {totpChallenge ? "Verify code" : "Sign in"}
            </button>
            {!totpChallenge && authConfig?.passkeys_enabled && (
              <button
                type="button"
                disabled={!username || passkeyMutation.isPending}
                onClick={() => passkeyMutation.mutate()}
                style={secondaryButtonStyle}
              >
                <Icon name="fileLock" size={16} />
                Sign in with passkey
              </button>
            )}
            {totpChallenge && (
              <button
                type="button"
                onClick={() => {
                  setTotpChallenge(null);
                  setTotpCode("");
                }}
                style={secondaryButtonStyle}
              >
                Back
              </button>
            )}
            {error && (
              <div
                style={{
                  color: "var(--color-danger)",
                  fontSize: "var(--text-sm)",
                }}
              >
                {error}
              </div>
            )}
          </form>
        )}
      </div>
    );
  }

  // User is authenticated but has no roots — show a message
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        minHeight: "100vh",
        gap: "var(--space-4)",
        background: "var(--color-bg)",
        color: "var(--color-fg)",
      }}
    >
      <AppLogo size={44} wordmarkSize={28} />
      <div
        style={{
          fontSize: "var(--text-sm)",
          color: "var(--color-fg-muted)",
          textAlign: "center",
          maxWidth: 400,
          lineHeight: 1.6,
        }}
      >
        No folders are configured yet. Set the{" "}
        <code
          style={{
            padding: "1px 4px",
            background: "var(--color-bg-muted)",
            borderRadius: "var(--radius-sm)",
            fontSize: "var(--text-xs)",
          }}
        >
          COMMON_FOLDERS
        </code>{" "}
        environment variable to get started.
      </div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  boxSizing: "border-box",
  width: "100%",
  padding: "var(--space-3)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg)",
  color: "var(--color-fg)",
  fontSize: "var(--text-base)",
};

const buttonStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: "var(--space-2)",
  padding: "var(--space-3) var(--space-4)",
  background: "var(--color-accent)",
  color: "var(--color-accent-fg)",
  border: "1px solid var(--color-accent)",
  borderRadius: "var(--radius-md)",
  fontWeight: 500,
  fontSize: "var(--text-base)",
  cursor: "pointer",
};

const secondaryButtonStyle: React.CSSProperties = {
  ...buttonStyle,
  background: "transparent",
  color: "var(--color-fg-muted)",
  borderColor: "var(--color-border)",
};

const primaryLinkStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "var(--space-3) var(--space-6)",
  background: "var(--color-accent)",
  color: "var(--color-accent-fg)",
  borderRadius: "var(--radius-md)",
  textDecoration: "none",
  fontWeight: 500,
  fontSize: "var(--text-base)",
  transition: `background var(--duration-fast) var(--ease-out)`,
};
