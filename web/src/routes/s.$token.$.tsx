import {
  createFileRoute,
  useParams,
  useNavigate,
} from "@tanstack/react-router";
import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import api from "../api/client";
import type { FileEntry } from "../api/client";
import {
  getFileIcon,
  formatFileSize,
  formatDate,
  formatModifiedDate,
  getPreviewType,
} from "../lib/icons";
import { FileIcon, Icon } from "../components/Icon";
import { DirectoryReadme } from "../components/DirectoryReadme";
import { MediaPreview } from "../components/MediaPreview";

export const Route = createFileRoute("/s/$token/$")({
  component: ShareViewer,
});

type Phase = "loading" | "password" | "browsing" | "error";

interface ShareMeta {
  name: string;
  is_directory: boolean;
  requires_password: boolean;
  owner_display_name: string;
  allow_upload: boolean;
  allow_download: boolean;
  expires_at: number | null;
}

function ShareViewer() {
  const { token } = useParams({ from: "/s/$token/$" });
  const params = Route.useParams();
  const subPath = params._splat || "";
  const navigate = useNavigate();

  const [phase, setPhase] = useState<Phase>("loading");
  const [meta, setMeta] = useState<ShareMeta | null>(null);
  const [bearer, setBearer] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [authError, setAuthError] = useState("");
  const [singleFileInfo, setSingleFileInfo] = useState<FileEntry | null>(null);
  const [previewTarget, setPreviewTarget] = useState<{
    entry: FileEntry;
    path: string;
  } | null>(null);
  const [uploadItems, setUploadItems] = useState<
    Array<{
      id: string;
      name: string;
      progress: number;
      status: "uploading" | "done" | "error";
      error?: string;
    }>
  >([]);
  const [showUploadProgress, setShowUploadProgress] = useState(false);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  const uploadFileInputRef = useRef<HTMLInputElement>(null);
  const uploadHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dragCounterRef = useRef(0);
  const listScrollRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => listScrollRef.current,
    estimateSize: () => 42,
    overscan: 12,
  });

  // Load metadata on mount
  useEffect(() => {
    api
      .shareMetadata(token)
      .then((m) => {
        setMeta(m);
        if (m.requires_password) {
          setPhase("password");
        } else {
          // Auto-auth for public shares
          api
            .shareAuth(token)
            .then((resp) => {
              setBearer(resp.bearer);
              setPhase("browsing");
            })
            .catch((e) => {
              setError(String(e));
              setPhase("error");
            });
        }
      })
      .catch(() => {
        setError("This share link is invalid or has expired.");
        setPhase("error");
      });
  }, [token]);

  // Load directory listing when browsing. A share with downloads disabled (an
  // upload-only drop box) intentionally cannot be listed — the server rejects
  // the listing to avoid leaking the folder's contents — so don't request it;
  // the recipient just sees the upload control and an empty state.
  useEffect(() => {
    if (
      phase === "browsing" &&
      bearer &&
      meta?.is_directory &&
      meta?.allow_download
    ) {
      api
        .shareList(token, bearer, subPath)
        .then((listing) => setEntries(listing.entries))
        .catch((e) => setError(String(e)));
    }
  }, [phase, bearer, token, subPath, meta?.is_directory, meta?.allow_download]);

  useEffect(() => {
    setSingleFileInfo(null);
    if (
      phase !== "browsing" ||
      !bearer ||
      meta?.is_directory ||
      !meta?.allow_download
    )
      return;

    let cancelled = false;
    api
      .shareInfo(token, bearer, "")
      .then((info) => {
        if (!cancelled) setSingleFileInfo(info);
      })
      .catch(() => {
        if (!cancelled) setSingleFileInfo(null);
      });

    return () => {
      cancelled = true;
    };
  }, [bearer, meta?.allow_download, meta?.is_directory, phase, token]);

  const handlePasswordAuth = async () => {
    setAuthError("");
    try {
      const resp = await api.shareAuth(token, password);
      setBearer(resp.bearer);
      setPhase("browsing");
    } catch {
      setAuthError("Incorrect password");
    }
  };

  const handleDownload = useCallback(
    (path: string) => {
      if (bearer) {
        window.open(api.shareDownloadUrl(token, bearer, path), "_blank");
      }
    },
    [token, bearer],
  );

  // Cancel pending upload auto-hide on unmount.
  useEffect(
    () => () => {
      if (uploadHideTimerRef.current) clearTimeout(uploadHideTimerRef.current);
    },
    [],
  );

  const handleShareUpload = useCallback(
    async (files: File[]) => {
      if (!bearer || files.length === 0) return;
      if (uploadHideTimerRef.current) {
        clearTimeout(uploadHideTimerRef.current);
        uploadHideTimerRef.current = null;
      }

      const items = files.map((f, i) => ({
        id: `${Date.now()}-${i}`,
        name: f.name,
        progress: 0,
        status: "uploading" as const,
      }));
      setUploadItems(items);
      setShowUploadProgress(true);

      await Promise.allSettled(
        items.map(async (item, idx) => {
          try {
            await api.shareUpload(
              token,
              bearer,
              subPath,
              [files[idx]],
              (pct) => {
                setUploadItems((prev) =>
                  prev.map((u) =>
                    u.id === item.id ? { ...u, progress: pct } : u,
                  ),
                );
              },
            );
            setUploadItems((prev) =>
              prev.map((u) =>
                u.id === item.id ? { ...u, status: "done", progress: 100 } : u,
              ),
            );
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setUploadItems((prev) =>
              prev.map((u) =>
                u.id === item.id ? { ...u, status: "error", error: msg } : u,
              ),
            );
          }
        }),
      );

      // Refresh listing (only when listing is permitted — see browsing effect).
      if (meta?.is_directory && meta?.allow_download) {
        const listing = await api
          .shareList(token, bearer, subPath)
          .catch(() => null);
        if (listing) setEntries(listing.entries);
      }

      uploadHideTimerRef.current = setTimeout(() => {
        setShowUploadProgress(false);
        setUploadItems([]);
        uploadHideTimerRef.current = null;
      }, 2000);
    },
    [bearer, token, subPath, meta?.is_directory, meta?.allow_download],
  );

  const uploadProgressOverall = useMemo(
    () =>
      uploadItems.length > 0
        ? Math.round(
            uploadItems.reduce((s, u) => s + u.progress, 0) /
              uploadItems.length,
          )
        : 0,
    [uploadItems],
  );

  const previewDialog =
    previewTarget && bearer ? (
      <ShareMediaPreviewDialog
        token={token}
        bearer={bearer}
        target={previewTarget}
        onClose={() => setPreviewTarget(null)}
      />
    ) : null;

  // Loading
  if (phase === "loading") {
    return (
      <div style={pageStyle}>
        <div style={cardStyle}>
          <div
            className="shimmer"
            style={{ width: 200, height: 24, borderRadius: 8 }}
          />
          <div
            className="shimmer"
            style={{ width: 300, height: 16, borderRadius: 8, marginTop: 12 }}
          />
        </div>
      </div>
    );
  }

  // Error
  if (phase === "error") {
    return (
      <div style={pageStyle}>
        <div style={cardStyle}>
          <Icon
            name="alertTriangle"
            size={48}
            color="var(--color-danger)"
            style={{ marginBottom: 16 }}
          />
          <h1
            style={{
              fontSize: "var(--text-xl)",
              fontWeight: 600,
              margin: "0 0 8px",
            }}
          >
            Share not available
          </h1>
          <p
            style={{
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-sm)",
            }}
          >
            {error || "This share link is invalid or has expired."}
          </p>
        </div>
      </div>
    );
  }

  // Password gate
  if (phase === "password") {
    return (
      <div style={pageStyle}>
        <div style={{ ...cardStyle, maxWidth: 400 }}>
          <Icon
            name="file"
            size={40}
            color="var(--color-accent)"
            style={{ marginBottom: 16 }}
          />
          <h1
            style={{
              fontSize: "var(--text-xl)",
              fontWeight: 600,
              margin: "0 0 4px",
            }}
          >
            {meta?.name || "Shared file"}
          </h1>
          <p
            style={{
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-sm)",
              margin: "0 0 20px",
            }}
          >
            Shared by {meta?.owner_display_name}
            {meta?.expires_at && (
              <>
                <br />
                Expires {formatDate(meta.expires_at)}
              </>
            )}
          </p>

          <form
            onSubmit={(e) => {
              e.preventDefault();
              handlePasswordAuth();
            }}
            style={{ width: "100%" }}
          >
            <input
              type="password"
              value={password}
              onChange={(e) => {
                setPassword(e.target.value);
                setAuthError("");
              }}
              placeholder="Enter password"
              autoFocus
              style={{
                width: "100%",
                padding: "var(--space-3)",
                border: `1px solid ${authError ? "var(--color-danger)" : "var(--color-border)"}`,
                borderRadius: "var(--radius-md)",
                fontSize: "var(--text-sm)",
                background: "var(--color-bg)",
                color: "var(--color-fg)",
                boxSizing: "border-box",
                marginBottom: "var(--space-2)",
              }}
            />
            {authError && (
              <div
                style={{
                  fontSize: "var(--text-xs)",
                  color: "var(--color-danger)",
                  marginBottom: "var(--space-2)",
                }}
              >
                {authError}
              </div>
            )}
            <button
              type="submit"
              style={{
                width: "100%",
                padding: "var(--space-3)",
                border: "none",
                borderRadius: "var(--radius-md)",
                background: "var(--color-accent)",
                color: "var(--color-accent-fg)",
                cursor: "pointer",
                fontWeight: 600,
                fontSize: "var(--text-sm)",
              }}
            >
              Open
            </button>
          </form>
        </div>
      </div>
    );
  }

  // Browsing — single file download
  if (!meta?.is_directory) {
    const singlePreviewType = singleFileInfo
      ? getPreviewType(singleFileInfo)
      : null;
    const canPreviewSingle =
      meta?.allow_download &&
      (singlePreviewType === "video" || singlePreviewType === "audio");

    return (
      <div style={pageStyle}>
        <div style={cardStyle}>
          <Icon
            name="file"
            size={48}
            color="var(--color-accent)"
            style={{ marginBottom: 16 }}
          />
          <h1
            style={{
              fontSize: "var(--text-xl)",
              fontWeight: 600,
              margin: "0 0 4px",
            }}
          >
            {meta?.name || "File"}
          </h1>
          <p
            style={{
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-sm)",
              margin: "0 0 20px",
            }}
          >
            Shared by {meta?.owner_display_name}
            {meta?.expires_at && (
              <>
                <br />
                Expires {formatDate(meta.expires_at)}
              </>
            )}
          </p>
          {meta?.allow_download && (
            <div
              style={{
                display: "flex",
                gap: "var(--space-2)",
                flexWrap: "wrap",
                justifyContent: "center",
              }}
            >
              {canPreviewSingle && singleFileInfo && (
                <button
                  onClick={() =>
                    setPreviewTarget({ entry: singleFileInfo, path: "" })
                  }
                  style={primaryButtonStyle}
                >
                  <Icon
                    name={singlePreviewType === "audio" ? "music" : "video"}
                    size={16}
                  />
                  Preview
                </button>
              )}
              <button
                onClick={() => handleDownload("")}
                style={secondaryButtonStyle}
              >
                <Icon name="download" size={16} />
                Download
              </button>
            </div>
          )}
        </div>
        {previewDialog}
      </div>
    );
  }

  const handleDragEnter = useCallback(
    (e: React.DragEvent) => {
      if (!meta?.allow_upload) return;
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current++;
      if (e.dataTransfer.types.includes("Files")) setIsDraggingFiles(true);
    },
    [meta?.allow_upload],
  );

  const handleDragLeave = useCallback(
    (e: React.DragEvent) => {
      if (!meta?.allow_upload) return;
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current = Math.max(0, dragCounterRef.current - 1);
      if (dragCounterRef.current === 0) setIsDraggingFiles(false);
    },
    [meta?.allow_upload],
  );

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      if (!meta?.allow_upload) return;
      e.preventDefault();
      e.stopPropagation();
    },
    [meta?.allow_upload],
  );

  const handleDrop = useCallback(
    async (e: React.DragEvent) => {
      if (!meta?.allow_upload) return;
      e.preventDefault();
      e.stopPropagation();
      dragCounterRef.current = 0;
      setIsDraggingFiles(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) await handleShareUpload(files);
    },
    [meta?.allow_upload, handleShareUpload],
  );

  // Directory listing
  return (
    <div
      style={{
        position: "relative",
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        background: "var(--color-bg)",
      }}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
    >
      {/* Header */}
      <header
        style={{
          padding: "var(--space-3) var(--space-6)",
          borderBottom: "1px solid var(--color-border)",
          display: "flex",
          alignItems: "center",
          gap: "var(--space-3)",
        }}
      >
        {subPath ? (
          <button
            onClick={() => {
              const parts = subPath.split("/");
              parts.pop();
              const newPath = parts.join("/");
              navigate({
                to: "/s/$token/$",
                params: { token, _splat: newPath },
              });
            }}
            style={{
              background: "transparent",
              border: "none",
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              color: "var(--color-fg-muted)",
              padding: "var(--space-2)",
              marginRight: "var(--space-2)",
              borderRadius: "var(--radius-md)",
            }}
            onMouseOver={(e) => {
              e.currentTarget.style.background = "var(--color-bg-muted)";
              e.currentTarget.style.color = "var(--color-fg)";
            }}
            onMouseOut={(e) => {
              e.currentTarget.style.background = "transparent";
              e.currentTarget.style.color = "var(--color-fg-muted)";
            }}
            title="Go back"
          >
            <Icon name="arrowLeft" size={20} />
          </button>
        ) : (
          <Icon name="folder" size={20} color="var(--color-accent)" />
        )}
        <div style={{ flex: 1 }}>
          <h1
            style={{ margin: 0, fontSize: "var(--text-base)", fontWeight: 600 }}
          >
            {meta?.name || "Shared folder"}
          </h1>
          <div
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--color-fg-muted)",
            }}
          >
            Shared by {meta?.owner_display_name}
            {subPath && <span> · {subPath}</span>}
            {meta?.expires_at && (
              <span> · Expires {formatDate(meta.expires_at)}</span>
            )}
          </div>
        </div>
        {meta?.allow_upload && (
          <>
            <input
              ref={uploadFileInputRef}
              type="file"
              multiple
              style={{ display: "none" }}
              onChange={(e) => {
                const files = Array.from(e.target.files || []);
                if (e.target) (e.target as HTMLInputElement).value = "";
                handleShareUpload(files);
              }}
            />
            <button
              onClick={() => uploadFileInputRef.current?.click()}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
                padding: "var(--space-2) var(--space-3)",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-md)",
                background: "var(--color-bg)",
                color: "var(--color-fg)",
                cursor: "pointer",
                fontWeight: 500,
                fontSize: "var(--text-sm)",
                transition: "background var(--duration-fast)",
              }}
              onMouseOver={(e) => {
                e.currentTarget.style.background = "var(--color-bg-muted)";
              }}
              onMouseOut={(e) => {
                e.currentTarget.style.background = "var(--color-bg)";
              }}
            >
              <Icon name="upload" size={16} />
              Upload
            </button>
          </>
        )}
        {meta?.allow_download && entries.length > 0 && (
          <button
            onClick={() => api.shareDownloadZip(token, bearer, [subPath || ""])}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              padding: "var(--space-2) var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-bg)",
              color: "var(--color-fg)",
              cursor: "pointer",
              fontWeight: 500,
              fontSize: "var(--text-sm)",
              transition: "background var(--duration-fast)",
            }}
            onMouseOver={(e) => {
              e.currentTarget.style.background = "var(--color-bg-muted)";
            }}
            onMouseOut={(e) => {
              e.currentTarget.style.background = "var(--color-bg)";
            }}
          >
            <Icon name="download" size={16} />
            Download all
          </button>
        )}
      </header>

      {/* File list */}
      <div
        ref={listScrollRef}
        style={{
          flex: 1,
          overflow: "auto",
          padding: "var(--space-4) var(--space-6)",
        }}
        className="flex flex-col-reverse lg:flex-row gap-6 items-start"
      >
        <div className="flex-1 min-w-0 w-full">
          {entries.length === 0 && (
            <div
              style={{
                textAlign: "center",
                padding: "var(--space-8)",
                color: "var(--color-fg-muted)",
              }}
            >
              This folder is empty
            </div>
          )}

          {entries.length > 0 && (
            <div
              style={{
                position: "relative",
                height: rowVirtualizer.getTotalSize(),
              }}
            >
              {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                const entry = entries[virtualRow.index];
                const icon = getFileIcon(entry);
                const entryPath = subPath
                  ? `${subPath}/${entry.name}`
                  : entry.name;
                const previewType = getPreviewType(entry);
                const isMediaPreview =
                  previewType === "video" || previewType === "audio";

                return (
                  <div
                    key={entry.name}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      right: 0,
                      height: 42,
                      transform: `translateY(${virtualRow.start}px)`,
                      display: "grid",
                      gridTemplateColumns: "1fr 100px 140px",
                      alignItems: "center",
                      padding: "var(--space-2) var(--space-3)",
                      borderRadius: "var(--radius-md)",
                      cursor: "pointer",
                      transition:
                        "background var(--duration-fast) var(--ease-out)",
                    }}
                    onClick={() => {
                      if (entry.is_dir) {
                        navigate({
                          to: "/s/$token/$",
                          params: { token, _splat: entryPath },
                        });
                      } else if (isMediaPreview && meta?.allow_download) {
                        setPreviewTarget({ entry, path: entryPath });
                      } else if (meta?.allow_download) {
                        handleDownload(entryPath);
                      }
                    }}
                    onMouseOver={(e) => {
                      e.currentTarget.style.background =
                        "var(--color-bg-muted)";
                    }}
                    onMouseOut={(e) => {
                      e.currentTarget.style.background = "transparent";
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "var(--space-2)",
                        overflow: "hidden",
                      }}
                    >
                      <FileIcon svg={icon.svg} color={icon.color} size={18} />
                      <span
                        style={{
                          fontSize: "var(--text-sm)",
                          fontWeight: entry.is_dir ? 500 : 400,
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {entry.name}
                      </span>
                      {isMediaPreview && meta?.allow_download && (
                        <span
                          style={{
                            fontSize: "var(--text-xs)",
                            color: "var(--color-accent)",
                            whiteSpace: "nowrap",
                          }}
                        >
                          Preview
                        </span>
                      )}
                    </div>
                    <div
                      className="tabular-nums"
                      style={{
                        fontSize: "var(--text-sm)",
                        color: "var(--color-fg-muted)",
                      }}
                    >
                      {entry.is_dir ? "—" : formatFileSize(entry.size)}
                    </div>
                    <div
                      className="tabular-nums"
                      style={{
                        fontSize: "var(--text-sm)",
                        color: "var(--color-fg-muted)",
                      }}
                    >
                      {formatModifiedDate(entry.modified_at)}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {entries.length > 0 && (
          <DirectoryReadme
            entries={entries}
            shareConfig={{ token, bearer, subPath }}
          />
        )}
      </div>
      {previewDialog}

      {/* Drop overlay */}
      {isDraggingFiles && meta?.allow_upload && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            background: "rgba(59, 130, 246, 0.08)",
            border: "2px dashed var(--color-accent)",
            borderRadius: "var(--radius-lg)",
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: "var(--space-3)",
            zIndex: 20,
            pointerEvents: "none",
          }}
        >
          <Icon
            name="upload"
            size={48}
            color="var(--color-accent)"
            style={{ opacity: 0.7 }}
          />
          <div
            style={{
              fontSize: "var(--text-lg)",
              fontWeight: 600,
              color: "var(--color-accent)",
            }}
          >
            Drop to upload
          </div>
        </div>
      )}

      {/* Upload progress panel */}
      {showUploadProgress && (
        <div
          style={{
            position: "fixed",
            bottom: "var(--space-6)",
            right: "var(--space-6)",
            width: 300,
            background: "var(--color-bg)",
            border: "1px solid var(--color-border)",
            borderRadius: "var(--radius-lg)",
            boxShadow: "var(--shadow-lg)",
            padding: "var(--space-4)",
            zIndex: 50,
          }}
        >
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              marginBottom: "var(--space-2)",
              fontSize: "var(--text-sm)",
              fontWeight: 600,
            }}
          >
            <span>
              Uploading {uploadItems.filter((u) => u.status === "done").length}/
              {uploadItems.length}
            </span>
            <span style={{ color: "var(--color-fg-muted)" }}>
              {uploadProgressOverall}%
            </span>
          </div>
          <div
            style={{
              height: 4,
              borderRadius: 2,
              background: "var(--color-bg-muted)",
              overflow: "hidden",
              marginBottom: "var(--space-3)",
            }}
          >
            <div
              style={{
                height: "100%",
                width: `${uploadProgressOverall}%`,
                background: "var(--color-accent)",
                borderRadius: 2,
                transition: "width 200ms ease-out",
              }}
            />
          </div>
          {uploadItems.slice(0, 8).map((item) => (
            <div
              key={item.id}
              style={{
                padding: "var(--space-1) 0",
                fontSize: "var(--text-xs)",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                }}
              >
                <span
                  style={{
                    color:
                      item.status === "done"
                        ? "var(--color-success)"
                        : item.status === "error"
                          ? "var(--color-danger)"
                          : "var(--color-fg-muted)",
                  }}
                >
                  {item.status === "done"
                    ? "✓"
                    : item.status === "error"
                      ? "✗"
                      : "⋯"}
                </span>
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {item.name}
                </span>
                {item.status !== "error" && (
                  <span
                    className="tabular-nums"
                    style={{ color: "var(--color-fg-subtle)" }}
                  >
                    {item.progress}%
                  </span>
                )}
              </div>
              {item.status === "error" && item.error && (
                <div
                  style={{
                    marginLeft: "calc(var(--space-2) + 1ch)",
                    color: "var(--color-danger)",
                    opacity: 0.8,
                    marginTop: 2,
                  }}
                >
                  {item.error}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ShareMediaPreviewDialog({
  token,
  bearer,
  target,
  onClose,
}: {
  token: string;
  bearer: string;
  target: { entry: FileEntry; path: string };
  onClose: () => void;
}) {
  const previewType = getPreviewType(target.entry);
  const kind = previewType === "audio" ? "audio" : "video";
  const actualUrl = api.shareDownloadUrl(token, bearer, target.path);
  const createPreviewUrl = useCallback(
    (session: string) =>
      api.sharePreviewUrl(token, bearer, target.path, session),
    [bearer, target.path, token],
  );
  const loadPreviewStatus = useCallback(
    (session: string) =>
      api.sharePreviewStatus(token, bearer, target.path, session),
    [bearer, target.path, token],
  );
  const loadFileInfo = useCallback(
    () => api.shareInfo(token, bearer, target.path),
    [bearer, target.path, token],
  );

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const eventTarget = e.target instanceof Element ? e.target : null;
      if (
        eventTarget?.closest(
          '.video-js, audio, video, input, textarea, select, button, a, [role="slider"]',
        )
      ) {
        if (
          e.key === "ArrowLeft" ||
          e.key === "ArrowRight" ||
          e.key === " " ||
          e.key === "Space" ||
          e.key === "Spacebar"
        ) {
          return;
        }
      }
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div
      style={previewOverlayStyle}
      onClickCapture={(e) => {
        if (
          shouldCloseFromMediaBackdropClick(
            e.currentTarget,
            e.clientX,
            e.clientY,
          )
        ) {
          onClose();
        }
      }}
    >
      <div data-preview-no-close style={previewTopBarStyle}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            overflow: "hidden",
            minWidth: 0,
          }}
        >
          <Icon
            name={kind === "audio" ? "music" : "video"}
            size={16}
            color="#fff"
          />
          <span
            style={{
              fontSize: "var(--text-sm)",
              fontWeight: 600,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {target.entry.name}
          </span>
          <span
            style={{
              color: "rgba(255,255,255,0.48)",
              fontSize: "var(--text-xs)",
            }}
          >
            Shared preview
          </span>
        </div>
        <div style={{ display: "flex", gap: "var(--space-2)" }}>
          <a
            href={actualUrl}
            target="_blank"
            rel="noopener"
            style={previewIconButtonStyle}
            title="Download"
          >
            <Icon name="download" size={16} color="#fff" />
          </a>
          <button
            type="button"
            onClick={onClose}
            style={previewIconButtonStyle}
            title="Close"
          >
            <Icon name="x" size={16} color="#fff" />
          </button>
        </div>
      </div>

      <div
        style={{
          flex: 1,
          minHeight: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          padding: "var(--space-4)",
        }}
      >
        <MediaPreview
          entry={target.entry}
          kind={kind}
          actualUrl={actualUrl}
          canTranscode
          createPreviewUrl={createPreviewUrl}
          loadPreviewStatus={loadPreviewStatus}
          loadFileInfo={loadFileInfo}
        />
      </div>
    </div>
  );
}

const MEDIA_BACKDROP_SAFE_ZONE_PX = 50;

function shouldCloseFromMediaBackdropClick(
  container: HTMLElement,
  clientX: number,
  clientY: number,
): boolean {
  const protectedElements = Array.from(
    container.querySelectorAll<HTMLElement>("[data-preview-no-close]"),
  );

  return !protectedElements.some((element) => {
    const rect = element.getBoundingClientRect();
    return (
      clientX >= rect.left - MEDIA_BACKDROP_SAFE_ZONE_PX &&
      clientX <= rect.right + MEDIA_BACKDROP_SAFE_ZONE_PX &&
      clientY >= rect.top - MEDIA_BACKDROP_SAFE_ZONE_PX &&
      clientY <= rect.bottom + MEDIA_BACKDROP_SAFE_ZONE_PX
    );
  });
}

const pageStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  minHeight: "100vh",
  background: "var(--color-bg)",
  padding: "var(--space-4)",
};

const cardStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  textAlign: "center",
  padding: "var(--space-8)",
  background: "var(--color-bg)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-xl)",
  boxShadow: "var(--shadow-lg)",
  maxWidth: 500,
  width: "100%",
};

const primaryButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "var(--space-3) var(--space-5)",
  border: "none",
  borderRadius: "var(--radius-md)",
  background: "var(--color-accent)",
  color: "var(--color-accent-fg)",
  cursor: "pointer",
  fontWeight: 600,
  fontSize: "var(--text-sm)",
};

const secondaryButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "var(--space-3) var(--space-5)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg)",
  color: "var(--color-fg)",
  cursor: "pointer",
  fontWeight: 600,
  fontSize: "var(--text-sm)",
};

const previewOverlayStyle: React.CSSProperties = {
  position: "fixed",
  inset: 0,
  zIndex: 250,
  display: "flex",
  flexDirection: "column",
  background: "rgba(0, 0, 0, 0.86)",
};

const previewTopBarStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: "var(--space-3)",
  padding: "var(--space-3) var(--space-4)",
  color: "#fff",
  flexShrink: 0,
};

const previewIconButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: 32,
  height: 32,
  border: "none",
  borderRadius: "var(--radius-md)",
  background: "rgba(255,255,255,0.1)",
  cursor: "pointer",
  textDecoration: "none",
};
