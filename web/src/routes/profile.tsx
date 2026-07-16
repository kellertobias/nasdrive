import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import api, { type CreatedApiToken } from "../api/client";
import { Icon } from "../components/Icon";
import { TopBar } from "../components/TopBar";
import { qrSvgDataUrl } from "../lib/qr";
import { prepareCreationOptions, serializeCredential } from "../lib/webauthn";
import { removeTrustedTotp } from "../lib/totp";

export const Route = createFileRoute("/profile")({
  component: ProfilePage,
});

function ProfilePage() {
  const queryClient = useQueryClient();
  const [publicKey, setPublicKey] = useState("");
  const [label, setLabel] = useState("");
  const [error, setError] = useState("");
  const [currentPassword, setCurrentPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [totpSetup, setTotpSetup] = useState<{
    secret: string;
    url: string;
  } | null>(null);
  const [totpCode, setTotpCode] = useState("");
  const [totpModalOpen, setTotpModalOpen] = useState(false);
  const [localError, setLocalError] = useState("");
  // API token state
  const [tokenModalOpen, setTokenModalOpen] = useState(false);
  const [tokenLabel, setTokenLabel] = useState("");
  const [tokenExpiry, setTokenExpiry] = useState("30d");
  const [createdToken, setCreatedToken] = useState<CreatedApiToken | null>(
    null,
  );

  const { data: user } = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });
  const { data, isLoading } = useQuery({
    queryKey: ["sftp-keys"],
    queryFn: api.listSftpKeys,
    staleTime: 10_000,
  });
  const { data: passkeyData } = useQuery({
    queryKey: ["passkeys"],
    queryFn: api.listPasskeys,
    enabled: user?.auth.passkeys_enabled === true,
    staleTime: 10_000,
  });
  const { data: trustedData } = useQuery({
    queryKey: ["trusted-devices"],
    queryFn: api.listTrustedDevices,
    enabled: user?.auth.totp_enabled === true,
    staleTime: 10_000,
  });
  const { data: tokenData } = useQuery({
    queryKey: ["api-tokens"],
    queryFn: api.listApiTokens,
    staleTime: 10_000,
  });

  const addMutation = useMutation({
    mutationFn: () => api.addSftpKey(publicKey, label || undefined),
    onSuccess: () => {
      setPublicKey("");
      setLabel("");
      setError("");
      queryClient.invalidateQueries({ queryKey: ["sftp-keys"] });
    },
    onError: (err) => setError(String(err)),
  });

  const revokeMutation = useMutation({
    mutationFn: api.revokeSftpKey,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["sftp-keys"] }),
  });
  const passwordMutation = useMutation({
    mutationFn: () => api.changePassword(currentPassword, newPassword),
    onSuccess: () => {
      setCurrentPassword("");
      setNewPassword("");
      setLocalError("");
    },
    onError: (err) =>
      setLocalError(err instanceof Error ? err.message : String(err)),
  });
  const passkeyAddMutation = useMutation({
    mutationFn: async () => {
      const options = await api.startPasskeyRegistration();
      const credential = await navigator.credentials.create({
        publicKey: prepareCreationOptions(options),
      });
      return api.finishPasskeyRegistration(serializeCredential(credential));
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["passkeys"] }),
    onError: (err) =>
      setLocalError(err instanceof Error ? err.message : String(err)),
  });
  const passkeyRevokeMutation = useMutation({
    mutationFn: api.revokePasskey,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["passkeys"] });
      queryClient.invalidateQueries({ queryKey: ["me"] });
    },
  });
  const totpStartMutation = useMutation({
    mutationFn: api.startTotpSetup,
    onSuccess: (setup) => {
      setTotpSetup(setup);
      setTotpModalOpen(true);
    },
    onError: (err) =>
      setLocalError(err instanceof Error ? err.message : String(err)),
  });
  const totpConfirmMutation = useMutation({
    mutationFn: () => api.confirmTotpSetup(totpCode),
    onSuccess: () => {
      setTotpSetup(null);
      setTotpCode("");
      setTotpModalOpen(false);
      queryClient.invalidateQueries({ queryKey: ["me"] });
    },
    onError: (err) =>
      setLocalError(err instanceof Error ? err.message : String(err)),
  });
  const totpRemoveMutation = useMutation({
    mutationFn: api.removeTotp,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["trusted-devices"] });
      queryClient.invalidateQueries({ queryKey: ["me"] });
    },
  });
  const trustedRevokeMutation = useMutation({
    mutationFn: api.revokeTrustedDevice,
    onSuccess: (_, id) => {
      if (user) removeTrustedTotp(user.username, id);
      queryClient.invalidateQueries({ queryKey: ["trusted-devices"] });
    },
  });

  const expiryOptions: {
    label: string;
    value: string;
    seconds: number | null;
  }[] = [
    { label: "7 days", value: "7d", seconds: 7 * 86400 },
    { label: "30 days", value: "30d", seconds: 30 * 86400 },
    { label: "90 days", value: "90d", seconds: 90 * 86400 },
    { label: "1 year", value: "1y", seconds: 365 * 86400 },
    { label: "No expiry", value: "never", seconds: null },
  ];
  const selectedExpiry =
    expiryOptions.find((o) => o.value === tokenExpiry) ?? expiryOptions[1];

  const createTokenMutation = useMutation({
    mutationFn: () =>
      api.createApiToken(tokenLabel.trim(), selectedExpiry.seconds),
    onSuccess: (created) => {
      setTokenModalOpen(false);
      setTokenLabel("");
      setTokenExpiry("30d");
      setCreatedToken(created);
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] });
    },
  });
  const revokeTokenMutation = useMutation({
    mutationFn: api.revokeApiToken,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] }),
  });
  const renewTokenMutation = useMutation({
    mutationFn: (id: string) => api.renewApiToken(id, 30 * 86400),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] }),
  });

  const keys = data?.keys ?? [];
  const passkeys = passkeyData?.passkeys ?? [];
  const trustedDevices = trustedData?.devices ?? [];
  const apiTokens = tokenData?.tokens ?? [];
  const totpQrUrl = totpSetup ? qrSvgDataUrl(totpSetup.url) : "";

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        background: "var(--color-bg)",
      }}
    >
      <TopBar user={user ?? null} currentRoot="" />
      <main style={{ flex: 1, overflow: "auto", padding: "var(--space-6)" }}>
        <div style={{ maxWidth: 860, margin: "0 auto" }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              marginBottom: "var(--space-5)",
            }}
          >
            <h1
              style={{ fontSize: "var(--text-xl)", fontWeight: 600, margin: 0 }}
            >
              Profile
            </h1>
            <button
              type="button"
              onClick={() => {
                window.location.href = "/";
              }}
              style={buttonStyle}
            >
              <Icon name="arrowLeft" size={16} />
              Back to Files
            </button>
          </div>

          {user?.auth.mode === "local" && (
            <section style={{ marginBottom: "var(--space-6)" }}>
              <h2 style={sectionTitleStyle}>Account security</h2>
              <div style={{ display: "grid", gap: "var(--space-5)" }}>
                <div style={{ display: "grid", gap: "var(--space-2)" }}>
                  <input
                    value={currentPassword}
                    onChange={(e) => setCurrentPassword(e.target.value)}
                    placeholder="Current password"
                    type="password"
                    style={inputStyle}
                  />
                  <input
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    placeholder="New password"
                    type="password"
                    style={inputStyle}
                  />
                  <button
                    type="button"
                    disabled={
                      !currentPassword ||
                      !newPassword ||
                      passwordMutation.isPending
                    }
                    onClick={() => passwordMutation.mutate()}
                    style={buttonStyle}
                  >
                    <Icon name="fileLock" size={16} />
                    Change password
                  </button>
                </div>

                {user.auth.passkeys_enabled && (
                  <div>
                    <div style={sectionRowStyle}>
                      <strong>Passkeys</strong>
                      <button
                        type="button"
                        onClick={() => passkeyAddMutation.mutate()}
                        style={buttonStyle}
                      >
                        <Icon name="fileLock" size={16} />
                        Add passkey
                      </button>
                    </div>
                    {passkeys.length === 0 ? (
                      <EmptyMessage text="No passkeys added" />
                    ) : (
                      <table style={tableStyle}>
                        <tbody>
                          {passkeys.map((key) => (
                            <tr key={key.id}>
                              <td style={tdStyle}>{key.label || "Passkey"}</td>
                              <td style={tdStyle}>
                                {new Date(key.created_at).toLocaleDateString()}
                              </td>
                              <td style={tdStyle}>
                                {key.last_used_at
                                  ? new Date(
                                      key.last_used_at,
                                    ).toLocaleDateString()
                                  : "Never"}
                              </td>
                              <td style={tdStyle}>
                                {!key.revoked_at && (
                                  <button
                                    type="button"
                                    onClick={() =>
                                      passkeyRevokeMutation.mutate(key.id)
                                    }
                                    style={{
                                      ...buttonStyle,
                                      color: "var(--color-danger)",
                                    }}
                                  >
                                    <Icon name="x" size={16} />
                                    Remove
                                  </button>
                                )}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </div>
                )}

                {user.auth.totp_enabled && passkeys.length === 0 && (
                  <div>
                    <div style={sectionRowStyle}>
                      <strong>TOTP</strong>
                      <div style={{ display: "flex", gap: "var(--space-2)" }}>
                        <button
                          type="button"
                          onClick={() => {
                            setTotpCode("");
                            setTotpModalOpen(true);
                            totpStartMutation.mutate();
                          }}
                          style={buttonStyle}
                        >
                          Setup TOTP
                        </button>
                        <button
                          type="button"
                          onClick={() => totpRemoveMutation.mutate()}
                          style={buttonStyle}
                        >
                          Remove TOTP
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {user.auth.totp_enabled && (
                  <div>
                    <strong>Trusted devices</strong>
                    {trustedDevices.length === 0 ? (
                      <EmptyMessage text="No trusted devices" />
                    ) : (
                      <table style={tableStyle}>
                        <tbody>
                          {trustedDevices.map((device) => (
                            <tr key={device.id}>
                              <td style={tdStyle}>
                                {device.label || "Trusted computer"}
                              </td>
                              <td style={tdStyle}>
                                {new Date(
                                  device.created_at,
                                ).toLocaleDateString()}
                              </td>
                              <td style={tdStyle}>
                                {device.last_used_at
                                  ? new Date(
                                      device.last_used_at,
                                    ).toLocaleDateString()
                                  : "Never"}
                              </td>
                              <td style={tdStyle}>
                                {!device.revoked_at && (
                                  <button
                                    type="button"
                                    onClick={() =>
                                      trustedRevokeMutation.mutate(device.id)
                                    }
                                    style={{
                                      ...buttonStyle,
                                      color: "var(--color-danger)",
                                    }}
                                  >
                                    <Icon name="x" size={16} />
                                    Revoke
                                  </button>
                                )}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </div>
                )}
                {localError && (
                  <span
                    style={{
                      color: "var(--color-danger)",
                      fontSize: "var(--text-sm)",
                    }}
                  >
                    {localError}
                  </span>
                )}
              </div>
            </section>
          )}

          {totpModalOpen && (
            <Modal title="Setup TOTP" onClose={() => setTotpModalOpen(false)}>
              {totpStartMutation.isPending && !totpSetup ? (
                <div
                  className="shimmer"
                  style={{ width: "100%", height: 220, borderRadius: 8 }}
                />
              ) : totpSetup ? (
                <div style={{ display: "grid", gap: "var(--space-4)" }}>
                  <div
                    style={{
                      display: "grid",
                      justifyItems: "center",
                      gap: "var(--space-3)",
                    }}
                  >
                    <img
                      src={totpQrUrl}
                      alt="TOTP setup QR code"
                      style={{
                        width: 220,
                        height: 220,
                        border: "1px solid var(--color-border)",
                        borderRadius: "var(--radius-md)",
                      }}
                    />
                    <code
                      style={{
                        fontSize: "var(--text-xs)",
                        overflowWrap: "anywhere",
                      }}
                    >
                      {totpSetup.secret}
                    </code>
                  </div>
                  <input
                    value={totpCode}
                    onChange={(e) => setTotpCode(e.target.value)}
                    placeholder="TOTP code"
                    inputMode="numeric"
                    style={inputStyle}
                  />
                  {totpConfirmMutation.error && (
                    <span
                      style={{
                        color: "var(--color-danger)",
                        fontSize: "var(--text-sm)",
                      }}
                    >
                      {totpConfirmMutation.error instanceof Error
                        ? totpConfirmMutation.error.message
                        : String(totpConfirmMutation.error)}
                    </span>
                  )}
                  <div style={modalActionsStyle}>
                    <button
                      type="button"
                      onClick={() => setTotpModalOpen(false)}
                      style={buttonStyle}
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={() => totpConfirmMutation.mutate()}
                      disabled={!totpCode || totpConfirmMutation.isPending}
                      style={{
                        ...buttonStyle,
                        background: "var(--color-accent)",
                        borderColor: "var(--color-accent)",
                        color: "var(--color-accent-fg)",
                      }}
                    >
                      Confirm TOTP
                    </button>
                  </div>
                </div>
              ) : (
                <EmptyMessage text="Unable to start TOTP setup" />
              )}
            </Modal>
          )}

          <section style={{ marginBottom: "var(--space-6)" }}>
            <h2 style={sectionTitleStyle}>SFTP keys</h2>
            {user?.capabilities.sftp_enabled &&
              user.capabilities.sftp_hostname && (
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-3)",
                    padding: "var(--space-3) var(--space-4)",
                    background: "var(--color-bg-muted)",
                    borderRadius: "var(--radius-md)",
                    border: "1px solid var(--color-border)",
                    marginBottom: "var(--space-4)",
                  }}
                >
                  <Icon
                    name="terminal"
                    size={16}
                    style={{ color: "var(--color-fg-muted)", flexShrink: 0 }}
                  />
                  <div
                    style={{ display: "flex", flexDirection: "column", gap: 2 }}
                  >
                    <span
                      style={{
                        fontSize: "var(--text-xs)",
                        color: "var(--color-fg-muted)",
                        fontWeight: 500,
                      }}
                    >
                      Connection URL
                    </span>
                    <code
                      style={{ fontSize: "var(--text-sm)", userSelect: "all" }}
                    >
                      {`sftp://${user.username}@${user.capabilities.sftp_hostname}:${user.capabilities.sftp_port}`}
                    </code>
                  </div>
                </div>
              )}
            <div
              style={{
                display: "grid",
                gap: "var(--space-3)",
                marginBottom: "var(--space-4)",
              }}
            >
              <input
                value={label}
                onChange={(e) => setLabel(e.target.value)}
                placeholder="Label"
                style={inputStyle}
              />
              <textarea
                value={publicKey}
                onChange={(e) => setPublicKey(e.target.value)}
                placeholder="ssh-ed25519 AAAA..."
                rows={4}
                style={{
                  ...inputStyle,
                  resize: "vertical",
                  fontFamily: "monospace",
                }}
              />
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-3)",
                }}
              >
                <button
                  type="button"
                  disabled={!publicKey.trim() || addMutation.isPending}
                  onClick={() => addMutation.mutate()}
                  style={{
                    ...buttonStyle,
                    background: "var(--color-accent)",
                    color: "var(--color-accent-fg)",
                    borderColor: "var(--color-accent)",
                  }}
                >
                  <Icon name="upload" size={16} />
                  Add key
                </button>
                {error && (
                  <span
                    style={{
                      color: "var(--color-danger)",
                      fontSize: "var(--text-sm)",
                    }}
                  >
                    {error}
                  </span>
                )}
              </div>
            </div>

            {isLoading ? (
              <div
                className="shimmer"
                style={{ width: "100%", height: 48, borderRadius: 8 }}
              />
            ) : keys.length === 0 ? (
              <EmptyMessage text="No SFTP keys added" />
            ) : (
              <table style={tableStyle}>
                <thead>
                  <tr>
                    <th style={thStyle}>Label</th>
                    <th style={thStyle}>Fingerprint</th>
                    <th style={thStyle}>Created</th>
                    <th style={thStyle}>Last Used</th>
                    <th style={thStyle}>Status</th>
                    <th style={thStyle}></th>
                  </tr>
                </thead>
                <tbody>
                  {keys.map((key) => (
                    <tr key={key.id}>
                      <td style={tdStyle}>{key.label || "SFTP key"}</td>
                      <td style={tdStyle}>
                        <code style={{ fontSize: "var(--text-xs)" }}>
                          {key.key_fingerprint}
                        </code>
                      </td>
                      <td style={tdStyle}>
                        {new Date(key.created_at).toLocaleDateString()}
                      </td>
                      <td style={tdStyle}>
                        {key.last_used_at
                          ? new Date(key.last_used_at).toLocaleDateString()
                          : "Never"}
                      </td>
                      <td
                        style={{
                          ...tdStyle,
                          color: key.revoked_at
                            ? "var(--color-fg-muted)"
                            : "var(--color-success)",
                          fontWeight: 500,
                        }}
                      >
                        {key.revoked_at ? "Revoked" : "Active"}
                      </td>
                      <td style={tdStyle}>
                        {!key.revoked_at && (
                          <button
                            type="button"
                            onClick={() => revokeMutation.mutate(key.id)}
                            style={{
                              ...buttonStyle,
                              color: "var(--color-danger)",
                            }}
                          >
                            <Icon name="x" size={16} />
                            Revoke
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>

          <section style={{ marginBottom: "var(--space-6)" }}>
            <div style={sectionRowStyle}>
              <h2 style={{ ...sectionTitleStyle, margin: 0 }}>S3 API tokens</h2>
              <button
                type="button"
                onClick={() => setTokenModalOpen(true)}
                style={buttonStyle}
              >
                <Icon name="upload" size={16} />
                Create token
              </button>
            </div>
            <p
              style={{
                fontSize: "var(--text-sm)",
                color: "var(--color-fg-muted)",
                margin: "0 0 var(--space-3)",
              }}
            >
              Use these credentials with rclone, the AWS CLI, or any
              S3-compatible tool to access your files programmatically.
            </p>
            {apiTokens.length === 0 ? (
              <EmptyMessage text="No API tokens" />
            ) : (
              <table style={tableStyle}>
                <thead>
                  <tr>
                    <th style={thStyle}>Label</th>
                    <th style={thStyle}>Access key</th>
                    <th style={thStyle}>Created</th>
                    <th style={thStyle}>Last used</th>
                    <th style={thStyle}>Expires</th>
                    <th style={thStyle}></th>
                  </tr>
                </thead>
                <tbody>
                  {apiTokens.map((token) => {
                    const expired =
                      token.expires_at !== null &&
                      token.expires_at < Date.now();
                    return (
                      <tr key={token.id}>
                        <td style={tdStyle}>{token.label}</td>
                        <td style={tdStyle}>
                          <code style={{ fontSize: "var(--text-xs)" }}>
                            {token.access_key.slice(0, 12)}…
                          </code>
                        </td>
                        <td style={tdStyle}>
                          {new Date(token.created_at).toLocaleDateString()}
                        </td>
                        <td style={tdStyle}>
                          {token.last_used_at
                            ? new Date(token.last_used_at).toLocaleDateString()
                            : "Never"}
                        </td>
                        <td
                          style={{
                            ...tdStyle,
                            color: expired
                              ? "var(--color-danger)"
                              : token.expires_at
                                ? "var(--color-fg)"
                                : "var(--color-fg-muted)",
                          }}
                        >
                          {token.expires_at
                            ? new Date(token.expires_at).toLocaleDateString()
                            : "Never"}
                        </td>
                        <td
                          style={{
                            ...tdStyle,
                            display: "flex",
                            gap: "var(--space-1)",
                            flexWrap: "wrap",
                          }}
                        >
                          <button
                            type="button"
                            onClick={() => renewTokenMutation.mutate(token.id)}
                            disabled={renewTokenMutation.isPending}
                            style={buttonStyle}
                          >
                            +30d
                          </button>
                          <button
                            type="button"
                            onClick={() => revokeTokenMutation.mutate(token.id)}
                            style={{
                              ...buttonStyle,
                              color: "var(--color-danger)",
                            }}
                          >
                            <Icon name="x" size={16} />
                            Revoke
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </section>
        </div>
      </main>

      {tokenModalOpen && (
        <Modal
          title="Create API token"
          onClose={() => {
            setTokenModalOpen(false);
            setTokenLabel("");
          }}
        >
          <div style={{ display: "grid", gap: "var(--space-3)" }}>
            <input
              value={tokenLabel}
              onChange={(e) => setTokenLabel(e.target.value)}
              placeholder="Token label (e.g. rclone home server)"
              style={inputStyle}
              autoFocus
            />
            <select
              value={tokenExpiry}
              onChange={(e) => setTokenExpiry(e.target.value)}
              style={inputStyle}
            >
              {expiryOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            {createTokenMutation.error && (
              <span
                style={{
                  color: "var(--color-danger)",
                  fontSize: "var(--text-sm)",
                }}
              >
                {createTokenMutation.error instanceof Error
                  ? createTokenMutation.error.message
                  : String(createTokenMutation.error)}
              </span>
            )}
            <div style={modalActionsStyle}>
              <button
                type="button"
                onClick={() => {
                  setTokenModalOpen(false);
                  setTokenLabel("");
                }}
                style={buttonStyle}
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={!tokenLabel.trim() || createTokenMutation.isPending}
                onClick={() => createTokenMutation.mutate()}
                style={{
                  ...buttonStyle,
                  background: "var(--color-accent)",
                  borderColor: "var(--color-accent)",
                  color: "var(--color-accent-fg)",
                }}
              >
                Create
              </button>
            </div>
          </div>
        </Modal>
      )}

      {createdToken && (
        <Modal
          title="Save your secret key"
          onClose={() => setCreatedToken(null)}
        >
          <div style={{ display: "grid", gap: "var(--space-4)" }}>
            <p
              style={{
                fontSize: "var(--text-sm)",
                color: "var(--color-fg-muted)",
                margin: 0,
              }}
            >
              The secret key is shown only once. Copy it now and store it
              somewhere safe.
            </p>
            <div style={{ display: "grid", gap: "var(--space-2)" }}>
              <label
                style={{
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                  color: "var(--color-fg-muted)",
                  textTransform: "uppercase",
                  letterSpacing: "0.05em",
                }}
              >
                Access key
              </label>
              <CopyField value={createdToken.access_key} />
            </div>
            <div style={{ display: "grid", gap: "var(--space-2)" }}>
              <label
                style={{
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                  color: "var(--color-fg-muted)",
                  textTransform: "uppercase",
                  letterSpacing: "0.05em",
                }}
              >
                Secret key
              </label>
              <CopyField value={createdToken.secret_key} />
            </div>
            <details style={{ fontSize: "var(--text-sm)" }}>
              <summary
                style={{
                  cursor: "pointer",
                  fontWeight: 500,
                  marginBottom: "var(--space-2)",
                }}
              >
                rclone configuration
              </summary>
              <pre
                style={{
                  background: "var(--color-bg-muted)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "var(--radius-md)",
                  padding: "var(--space-3)",
                  fontSize: "var(--text-xs)",
                  overflowX: "auto",
                  margin: 0,
                }}
              >
                {[
                  `[nasdrive]`,
                  `type = s3`,
                  `provider = Other`,
                  `access_key_id = ${createdToken.access_key}`,
                  `secret_access_key = ${createdToken.secret_key}`,
                  `endpoint = ${window.location.origin}/s3`,
                  `force_path_style = true`,
                  `region = us-east-1`,
                ].join("\n")}
              </pre>
            </details>
            <div style={modalActionsStyle}>
              <button
                type="button"
                onClick={() => setCreatedToken(null)}
                style={{
                  ...buttonStyle,
                  background: "var(--color-accent)",
                  borderColor: "var(--color-accent)",
                  color: "var(--color-accent-fg)",
                }}
              >
                Done
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

function EmptyMessage({ text }: { text: string }) {
  return (
    <div
      style={{
        padding: "var(--space-8)",
        textAlign: "center",
        color: "var(--color-fg-muted)",
        fontSize: "var(--text-sm)",
      }}
    >
      {text}
    </div>
  );
}

function CopyField({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div
      style={{ display: "flex", gap: "var(--space-2)", alignItems: "center" }}
    >
      <code
        style={{
          flex: 1,
          padding: "var(--space-2) var(--space-3)",
          background: "var(--color-bg-muted)",
          border: "1px solid var(--color-border)",
          borderRadius: "var(--radius-md)",
          fontSize: "var(--text-xs)",
          overflowWrap: "anywhere",
          userSelect: "all",
        }}
      >
        {value}
      </code>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard.writeText(value).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
          });
        }}
        style={buttonStyle}
      >
        {copied ? (
          <Icon name="check" size={16} />
        ) : (
          <Icon name="copy" size={16} />
        )}
      </button>
    </div>
  );
}

function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: React.ReactNode;
  onClose: () => void;
}) {
  return (
    <div style={modalOverlayStyle} role="presentation" onMouseDown={onClose}>
      <div
        style={modalPanelStyle}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div style={modalHeaderStyle}>
          <strong>{title}</strong>
          <button type="button" onClick={onClose} style={buttonStyle}>
            <Icon name="x" size={16} />
            Close
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "var(--space-2) var(--space-3)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg)",
  color: "var(--color-fg)",
  fontSize: "var(--text-sm)",
};

const buttonStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: "var(--space-1)",
  padding: "var(--space-2) var(--space-3)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "transparent",
  color: "var(--color-fg-muted)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  fontWeight: 500,
};

const sectionTitleStyle: React.CSSProperties = {
  fontSize: "var(--text-md)",
  fontWeight: 600,
  margin: "0 0 var(--space-3)",
};

const sectionRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "var(--space-3)",
  marginBottom: "var(--space-2)",
  flexWrap: "wrap",
};

const tableStyle: React.CSSProperties = {
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "var(--text-sm)",
};

const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "var(--space-2) var(--space-3)",
  borderBottom: "1px solid var(--color-border)",
  color: "var(--color-fg-muted)",
  fontWeight: 600,
  fontSize: "var(--text-xs)",
  textTransform: "uppercase",
  letterSpacing: "0.05em",
};

const tdStyle: React.CSSProperties = {
  padding: "var(--space-2) var(--space-3)",
  borderBottom: "1px solid var(--color-border-muted)",
  verticalAlign: "middle",
};

const modalOverlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  zIndex: 60,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  padding: "var(--space-4)",
  background: "var(--color-overlay)",
};

const modalPanelStyle: React.CSSProperties = {
  width: "min(520px, 100%)",
  maxHeight: "min(700px, calc(100vh - 48px))",
  overflow: "auto",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-lg)",
  padding: "var(--space-4)",
  background: "var(--color-bg)",
  boxShadow: "var(--shadow-lg)",
};

const modalHeaderStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "var(--space-3)",
  marginBottom: "var(--space-4)",
};

const modalActionsStyle: React.CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: "var(--space-2)",
  flexWrap: "wrap",
};
