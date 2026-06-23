import { useState, useEffect, useCallback } from "react";
import api, { formatApiError, formatApiErrorDetails } from "../api/client";
import { Icon } from "./Icon";
import { ErrorDialog } from "./ErrorNotice";
import type { ErrorNoticeData } from "./ErrorNotice";

interface ShareDialogProps {
  open: boolean;
  root: string;
  path: string;
  isDirectory: boolean;
  onClose: () => void;
}

type TargetKind = "public" | "guest";

interface ExistingShare {
  id: string;
  root_key: string;
  relative_path: string;
  target_kind: string;
  allow_upload: boolean;
  allow_download: boolean;
  expires_at: number | null;
  created_at: number;
  revoked_at: number | null;
  access_count: number;
  last_accessed_at: number | null;
}

export function ShareDialog({
  open,
  root,
  path,
  isDirectory,
  onClose,
}: ShareDialogProps) {
  const [targetKind, setTargetKind] = useState<TargetKind>("public");
  const [password, setPassword] = useState("");
  const [allowDownload, setAllowDownload] = useState(true);
  const [allowUpload, setAllowUpload] = useState(false);
  const [expiresIn, setExpiresIn] = useState<number | null>(86400); // 1 day default
  const [shareUrl, setShareUrl] = useState("");
  const [creating, setCreating] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  const [dialogError, setDialogError] = useState<ErrorNoticeData | null>(null);
  const [existingShares, setExistingShares] = useState<ExistingShare[]>([]);

  // Generate a random password
  const generatePassword = useCallback(() => {
    const chars = "ABCDEFGHJKMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";
    let result = "";
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    for (let i = 0; i < 16; i++) {
      result += chars[bytes[i] % chars.length];
    }
    setPassword(result);
  }, []);

  // Load existing shares for this path on open
  useEffect(() => {
    if (open) {
      setShareUrl("");
      setError("");
      setCopied(false);
      setCreating(false);
      if (targetKind === "guest" && !password) {
        generatePassword();
      }
      // Load existing shares
      api
        .listShares?.()
        .then((resp: { shares: ExistingShare[] }) => {
          const filtered = resp.shares.filter(
            (s) =>
              !s.revoked_at &&
              (!s.expires_at || s.expires_at > Date.now()) &&
              s.root_key === root &&
              s.relative_path === path,
          );
          setExistingShares(filtered);
        })
        .catch(() => {});
    }
  }, [open, targetKind, password, generatePassword, root, path]);

  if (!open) return null;

  const handleCreate = async () => {
    setCreating(true);
    setError("");
    try {
      const resp = await api.createShare(root, path, {
        target_kind: targetKind,
        password: targetKind === "guest" ? password : undefined,
        allow_download: allowDownload,
        allow_upload: allowUpload,
        expires_in: expiresIn,
      });
      setShareUrl(resp.url);
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  };

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(shareUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Fallback
      const input = document.createElement("input");
      input.value = shareUrl;
      document.body.appendChild(input);
      input.select();
      document.execCommand("copy");
      document.body.removeChild(input);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const handleRevoke = async (shareId: string) => {
    try {
      await api.revokeShare(shareId);
      setExistingShares((prev) => prev.filter((s) => s.id !== shareId));
    } catch (err) {
      setDialogError({
        id: Date.now(),
        title: "Failed to revoke share",
        message: formatApiError(err),
        details: formatApiErrorDetails(err),
      });
    }
  };

  const expiryOptions = [
    { label: "1 hour", value: 3600 },
    { label: "1 day", value: 86400 },
    { label: "7 days", value: 604800 },
    { label: "30 days", value: 2592000 },
    { label: "Never", value: null },
  ];

  const fileName = path.split("/").pop() || root;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      className="fade-in"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        style={{
          background: "var(--color-bg)",
          borderRadius: "var(--radius-xl)",
          boxShadow: "var(--shadow-xl)",
          width: 480,
          maxWidth: "95vw",
          maxHeight: "90vh",
          overflowY: "auto",
        }}
        className="slide-in"
      >
        {/* Header */}
        <div
          style={{
            padding: "var(--space-6) var(--space-6) var(--space-4)",
            borderBottom: "1px solid var(--color-border-muted)",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-3)",
              marginBottom: "var(--space-2)",
            }}
          >
            <Icon name="file" size={20} color="var(--color-accent)" />
            <h2
              style={{ margin: 0, fontSize: "var(--text-lg)", fontWeight: 600 }}
            >
              Share
            </h2>
          </div>
          <div
            style={{
              fontSize: "var(--text-sm)",
              color: "var(--color-fg-muted)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {fileName}
          </div>
        </div>

        <div style={{ padding: "var(--space-4) var(--space-6)" }}>
          {/* Target kind tabs */}
          <div
            style={{
              display: "flex",
              gap: "var(--space-1)",
              marginBottom: "var(--space-4)",
              background: "var(--color-bg-muted)",
              borderRadius: "var(--radius-md)",
              padding: 2,
            }}
          >
            {(["public", "guest"] as const).map((kind) => (
              <button
                key={kind}
                onClick={() => {
                  setTargetKind(kind);
                  setShareUrl("");
                  if (kind === "guest" && !password) generatePassword();
                }}
                style={{
                  flex: 1,
                  padding: "var(--space-2)",
                  border: "none",
                  borderRadius: "var(--radius-md)",
                  background:
                    targetKind === kind ? "var(--color-bg)" : "transparent",
                  boxShadow: targetKind === kind ? "var(--shadow-sm)" : "none",
                  color:
                    targetKind === kind
                      ? "var(--color-fg)"
                      : "var(--color-fg-muted)",
                  cursor: "pointer",
                  fontSize: "var(--text-sm)",
                  fontWeight: 500,
                  transition: "all var(--duration-fast) var(--ease-out)",
                }}
              >
                {kind === "public" ? "Anyone with link" : "Password protected"}
              </button>
            ))}
          </div>

          {/* Password field (guest only) */}
          {targetKind === "guest" && (
            <div style={{ marginBottom: "var(--space-4)" }}>
              <label
                style={{
                  display: "block",
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                  color: "var(--color-fg-muted)",
                  marginBottom: "var(--space-1)",
                  textTransform: "uppercase",
                  letterSpacing: "var(--tracking-wide)",
                }}
              >
                Password
              </label>
              <div style={{ display: "flex", gap: "var(--space-2)" }}>
                <input
                  type="text"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  style={{
                    flex: 1,
                    padding: "var(--space-2) var(--space-3)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    fontSize: "var(--text-sm)",
                    fontFamily: "monospace",
                    background: "var(--color-bg)",
                    color: "var(--color-fg)",
                    boxSizing: "border-box",
                  }}
                />
                <button
                  onClick={generatePassword}
                  title="Generate new password"
                  style={{
                    padding: "var(--space-2)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    background: "transparent",
                    color: "var(--color-fg-muted)",
                    cursor: "pointer",
                    display: "flex",
                    alignItems: "center",
                  }}
                >
                  <Icon name="settings" size={16} />
                </button>
              </div>
            </div>
          )}

          {/* Permissions */}
          <div style={{ marginBottom: "var(--space-4)" }}>
            <label
              style={{
                display: "block",
                fontSize: "var(--text-xs)",
                fontWeight: 600,
                color: "var(--color-fg-muted)",
                marginBottom: "var(--space-2)",
                textTransform: "uppercase",
                letterSpacing: "var(--tracking-wide)",
              }}
            >
              Permissions
            </label>
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--space-2)",
              }}
            >
              <label
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  fontSize: "var(--text-sm)",
                  cursor: "pointer",
                }}
              >
                <input
                  type="checkbox"
                  checked={allowDownload}
                  onChange={(e) => setAllowDownload(e.target.checked)}
                />
                Allow download
              </label>
              {isDirectory && (
                <label
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    fontSize: "var(--text-sm)",
                    cursor: "pointer",
                  }}
                >
                  <input
                    type="checkbox"
                    checked={allowUpload}
                    onChange={(e) => setAllowUpload(e.target.checked)}
                  />
                  Allow upload
                </label>
              )}
            </div>
          </div>

          {/* Expiry */}
          <div style={{ marginBottom: "var(--space-4)" }}>
            <label
              style={{
                display: "block",
                fontSize: "var(--text-xs)",
                fontWeight: 600,
                color: "var(--color-fg-muted)",
                marginBottom: "var(--space-2)",
                textTransform: "uppercase",
                letterSpacing: "var(--tracking-wide)",
              }}
            >
              Expires
            </label>
            <div
              style={{
                display: "flex",
                gap: "var(--space-1)",
                flexWrap: "wrap",
              }}
            >
              {expiryOptions.map((opt) => (
                <button
                  key={opt.label}
                  onClick={() => setExpiresIn(opt.value)}
                  style={{
                    padding: "var(--space-1) var(--space-3)",
                    border: "1px solid",
                    borderColor:
                      expiresIn === opt.value
                        ? "var(--color-accent)"
                        : "var(--color-border)",
                    borderRadius: "var(--radius-full)",
                    background:
                      expiresIn === opt.value
                        ? "var(--color-accent-muted)"
                        : "transparent",
                    color:
                      expiresIn === opt.value
                        ? "var(--color-accent)"
                        : "var(--color-fg-muted)",
                    cursor: "pointer",
                    fontSize: "var(--text-xs)",
                    fontWeight: 500,
                    transition: "all var(--duration-fast) var(--ease-out)",
                  }}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>

          {/* Generated link */}
          {shareUrl && (
            <div
              style={{
                padding: "var(--space-3)",
                background: "var(--color-bg-muted)",
                borderRadius: "var(--radius-md)",
                marginBottom: "var(--space-4)",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  marginBottom: "var(--space-2)",
                }}
              >
                <Icon name="file" size={14} color="var(--color-success)" />
                <span
                  style={{
                    fontSize: "var(--text-xs)",
                    fontWeight: 600,
                    color: "var(--color-success)",
                  }}
                >
                  Share link created
                </span>
              </div>
              <div style={{ display: "flex", gap: "var(--space-2)" }}>
                <input
                  type="text"
                  value={shareUrl}
                  readOnly
                  style={{
                    flex: 1,
                    padding: "var(--space-2) var(--space-3)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    fontSize: "var(--text-xs)",
                    fontFamily: "monospace",
                    background: "var(--color-bg)",
                    color: "var(--color-fg)",
                    boxSizing: "border-box",
                  }}
                  onClick={(e) => (e.target as HTMLInputElement).select()}
                />
                <button
                  onClick={handleCopy}
                  style={{
                    padding: "var(--space-2) var(--space-3)",
                    border: "none",
                    borderRadius: "var(--radius-md)",
                    background: copied
                      ? "var(--color-success)"
                      : "var(--color-accent)",
                    color: "#fff",
                    cursor: "pointer",
                    fontSize: "var(--text-sm)",
                    fontWeight: 500,
                    minWidth: 70,
                    transition: "all var(--duration-fast) var(--ease-out)",
                  }}
                >
                  {copied ? "✓ Copied" : "Copy"}
                </button>
              </div>
              {targetKind === "guest" && (
                <div
                  style={{
                    marginTop: "var(--space-2)",
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-muted)",
                  }}
                >
                  Password:{" "}
                  <code
                    style={{
                      fontFamily: "monospace",
                      color: "var(--color-fg)",
                    }}
                  >
                    {password}
                  </code>
                </div>
              )}
            </div>
          )}

          {error && (
            <div
              style={{
                padding: "var(--space-2) var(--space-3)",
                background: "var(--color-danger-muted)",
                borderRadius: "var(--radius-md)",
                fontSize: "var(--text-sm)",
                color: "var(--color-danger)",
                marginBottom: "var(--space-4)",
              }}
            >
              {error}
            </div>
          )}
        </div>

        {/* Existing shares section */}
        {existingShares.length > 0 && (
          <div
            style={{
              padding: "0 var(--space-6) var(--space-4)",
              borderTop: "1px solid var(--color-border-muted)",
              paddingTop: "var(--space-4)",
            }}
          >
            <div
              style={{
                fontSize: "var(--text-xs)",
                fontWeight: 600,
                color: "var(--color-fg-muted)",
                marginBottom: "var(--space-2)",
                textTransform: "uppercase",
                letterSpacing: "var(--tracking-wide)",
              }}
            >
              Active shares ({existingShares.length})
            </div>
            {existingShares.slice(0, 5).map((s) => (
              <div
                key={s.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "var(--space-2) 0",
                  fontSize: "var(--text-sm)",
                  borderBottom: "1px solid var(--color-border-muted)",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    flexDirection: "column",
                    gap: "var(--space-1)",
                  }}
                >
                  <div>
                    <span style={{ color: "var(--color-fg)" }}>
                      {s.target_kind === "public" ? "Public" : "Password"}
                    </span>
                    <span
                      style={{
                        color: "var(--color-fg-subtle)",
                        marginLeft: "var(--space-2)",
                        fontSize: "var(--text-xs)",
                      }}
                    >
                      Created: {new Date(s.created_at).toLocaleDateString()}
                    </span>
                    <span
                      style={{
                        color: "var(--color-fg-subtle)",
                        marginLeft: "var(--space-2)",
                        fontSize: "var(--text-xs)",
                      }}
                    >
                      Expires:{" "}
                      {s.expires_at
                        ? new Date(s.expires_at).toLocaleDateString()
                        : "Never"}
                    </span>
                  </div>
                  <div
                    style={{
                      color: "var(--color-fg-subtle)",
                      fontSize: "var(--text-xs)",
                    }}
                  >
                    {s.access_count} views • Last accessed:{" "}
                    {s.last_accessed_at
                      ? new Date(s.last_accessed_at).toLocaleDateString()
                      : "Never"}
                  </div>
                </div>
                <button
                  onClick={() => handleRevoke(s.id)}
                  style={{
                    padding: "var(--space-1) var(--space-2)",
                    border: "none",
                    borderRadius: "var(--radius-md)",
                    background: "transparent",
                    color: "var(--color-danger)",
                    cursor: "pointer",
                    fontSize: "var(--text-xs)",
                  }}
                >
                  Revoke
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Footer */}
        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "var(--space-2)",
            padding: "var(--space-4) var(--space-6)",
            borderTop: "1px solid var(--color-border-muted)",
          }}
        >
          <button
            onClick={onClose}
            style={{
              padding: "var(--space-2) var(--space-4)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "transparent",
              color: "var(--color-fg)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
            }}
          >
            Close
          </button>
          {!shareUrl && (
            <button
              onClick={handleCreate}
              disabled={
                creating || (targetKind === "guest" && password.length < 4)
              }
              style={{
                padding: "var(--space-2) var(--space-4)",
                border: "none",
                borderRadius: "var(--radius-md)",
                background: "var(--color-accent)",
                color: "var(--color-accent-fg)",
                cursor: creating ? "wait" : "pointer",
                fontWeight: 500,
                fontSize: "var(--text-sm)",
                opacity: creating ? 0.7 : 1,
              }}
            >
              {creating ? "Creating…" : "Create Link"}
            </button>
          )}
        </div>
      </div>

      <ErrorDialog error={dialogError} onClose={() => setDialogError(null)} />
    </div>
  );
}
