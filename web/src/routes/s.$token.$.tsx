import {
  createFileRoute,
  useParams,
  useNavigate,
} from "@tanstack/react-router";
import { useState, useEffect, useCallback, useRef, useMemo } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import api, { DownloadAbortedError, UploadAbortedError } from "../api/client";
import type { DownloadProgress, FileEntry, GalleryItem, ShareType } from "../api/client";
import {
  getFileIcon,
  formatFileSize,
  formatExpirationDate,
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
  share_type: ShareType;
  expires_at: number | null;
}

interface ActiveDownload {
  label: string;
  loadedBytes: number;
  totalBytes: number | null;
  pct: number | null;
  status: "downloading" | "cancelled";
}

function fileDownloadKey(path: string) {
  return `file:${path}`;
}

function zipDownloadKey(path: string) {
  return `zip:${path}`;
}

// @tour share-management:90 A visitor opens the link
// The route pulls `token` from params and the in-share subdirectory from `params._splat`,
// and a `phase` state machine walks `loading → password | browsing | error`.
//
// The mount effect calls `api.shareMetadata(token)`; if a password is required it switches
// phase, otherwise it immediately calls `api.shareAuth(token)` and stores `resp.bearer`. A
// later effect deliberately skips listing when `allow_download` is false, mirroring the
// server-side gate.

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
      status: "uploading" | "done" | "error" | "pending" | "cancelled";
      error?: string;
    }>
  >([]);
  const [showUploadProgress, setShowUploadProgress] = useState(false);
  const [zipError, setZipError] = useState("");
  const [activeDownloads, setActiveDownloads] = useState<
    Record<string, ActiveDownload>
  >({});
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  const uploadFileInputRef = useRef<HTMLInputElement>(null);
  const uploadHideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const downloadHideTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(
    new Map(),
  );
  const activeDownloadKeysRef = useRef<Set<string>>(new Set());
  const downloadAbortMapRef = useRef<Map<string, () => void>>(new Map());
  const dragCounterRef = useRef(0);
  const uploadAbortMapRef = useRef<Map<string, () => void>>(new Map());
  const uploadCancelledRef = useRef<Set<string>>(new Set());
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
      meta.share_type !== "gallery" &&
      meta?.allow_download
    ) {
      api
        .shareList(token, bearer, subPath)
        .then((listing) => setEntries(listing.entries))
        .catch((e) => setError(String(e)));
    }
  }, [phase, bearer, token, subPath, meta?.is_directory, meta?.allow_download, meta?.share_type]);

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

  const clearDownloadSoon = useCallback((key: string) => {
    const existing = downloadHideTimersRef.current.get(key);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      setActiveDownloads((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
      activeDownloadKeysRef.current.delete(key);
      downloadHideTimersRef.current.delete(key);
    }, 1200);
    downloadHideTimersRef.current.set(key, timer);
  }, []);

  const updateDownloadProgress = useCallback(
    (key: string, label: string, progress: DownloadProgress) => {
      const existingTimer = downloadHideTimersRef.current.get(key);
      if (existingTimer) {
        clearTimeout(existingTimer);
        downloadHideTimersRef.current.delete(key);
      }
      setActiveDownloads((prev) => ({
        ...prev,
        [key]: {
          label,
          loadedBytes: progress.loaded,
          totalBytes: progress.total,
          pct: progress.pct,
          status: "downloading",
        },
      }));
    },
    [],
  );

  const handleDownload = useCallback(
    async (path: string) => {
      if (!bearer) return;
      const key = fileDownloadKey(path);
      if (activeDownloadKeysRef.current.has(key)) return;
      activeDownloadKeysRef.current.add(key);
      const label = path.split("/").pop() || meta?.name || "Download";
      setActiveDownloads((prev) => ({
        ...prev,
        [key]: {
          label,
          loadedBytes: 0,
          totalBytes: null,
          pct: null,
          status: "downloading",
        },
      }));
      const handle = api.shareDownloadFile(token, bearer, path, (progress) =>
        updateDownloadProgress(key, label, progress),
      );
      downloadAbortMapRef.current.set(key, handle.abort);
      try {
        await handle.promise;
      } catch (err) {
        if (err instanceof DownloadAbortedError) {
          setActiveDownloads((prev) => ({
            ...prev,
            [key]: {
              ...(prev[key] ?? {
                label,
                loadedBytes: 0,
                totalBytes: null,
                pct: null,
              }),
              status: "cancelled",
            },
          }));
          return;
        }
        window.open(api.shareDownloadUrl(token, bearer, path), "_blank");
      } finally {
        downloadAbortMapRef.current.delete(key);
        clearDownloadSoon(key);
      }
    },
    [
      bearer,
      clearDownloadSoon,
      meta?.name,
      token,
      updateDownloadProgress,
    ],
  );

  const handleZipDownload = useCallback(
    async (path: string, label: string) => {
      if (!bearer) return;
      const key = zipDownloadKey(path);
      if (activeDownloadKeysRef.current.has(key)) return;
      activeDownloadKeysRef.current.add(key);
      setZipError("");
      setActiveDownloads((prev) => ({
        ...prev,
        [key]: {
          label,
          loadedBytes: 0,
          totalBytes: null,
          pct: null,
          status: "downloading",
        },
      }));
      const handle = api.shareDownloadZip(token, bearer, [path], (progress) =>
        updateDownloadProgress(key, label, progress),
      );
      downloadAbortMapRef.current.set(key, handle.abort);
      try {
        await handle.promise;
      } catch (err) {
        if (err instanceof DownloadAbortedError) {
          setActiveDownloads((prev) => ({
            ...prev,
            [key]: {
              ...(prev[key] ?? {
                label,
                loadedBytes: 0,
                totalBytes: null,
                pct: null,
              }),
              status: "cancelled",
            },
          }));
          return;
        }
        setZipError(`Failed to download ${label.toLowerCase()}. Please try again.`);
      } finally {
        downloadAbortMapRef.current.delete(key);
        clearDownloadSoon(key);
      }
    },
    [bearer, clearDownloadSoon, token, updateDownloadProgress],
  );

  // Cancel pending upload auto-hide on unmount.
  useEffect(
    () => () => {
      if (uploadHideTimerRef.current) clearTimeout(uploadHideTimerRef.current);
      downloadHideTimersRef.current.forEach((timer) => clearTimeout(timer));
      downloadAbortMapRef.current.forEach((abort) => abort());
      downloadAbortMapRef.current.clear();
      activeDownloadKeysRef.current.clear();
    },
    [],
  );

  const handleCancelDownload = useCallback(
    (key: string) => {
      const abort = downloadAbortMapRef.current.get(key);
      if (!abort) return;
      abort();
      downloadAbortMapRef.current.delete(key);
      setActiveDownloads((prev) => ({
        ...prev,
        [key]: {
          ...(prev[key] ?? {
            label: "Download",
            loadedBytes: 0,
            totalBytes: null,
            pct: null,
          }),
          status: "cancelled",
        },
      }));
      clearDownloadSoon(key);
    },
    [clearDownloadSoon],
  );

  const handleShareUpload = useCallback(
    async (files: File[]) => {
      if (!bearer || files.length === 0) return;
      if (uploadHideTimerRef.current) {
        clearTimeout(uploadHideTimerRef.current);
        uploadHideTimerRef.current = null;
      }

      uploadAbortMapRef.current.clear();
      uploadCancelledRef.current.clear();

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
          if (uploadCancelledRef.current.has(item.id)) {
            setUploadItems((prev) =>
              prev.map((u) =>
                u.id === item.id ? { ...u, status: "cancelled" } : u,
              ),
            );
            return;
          }
          const handle = api.shareUpload(
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
          uploadAbortMapRef.current.set(item.id, handle.abort);
          try {
            await handle.promise;
            uploadAbortMapRef.current.delete(item.id);
            setUploadItems((prev) =>
              prev.map((u) =>
                u.id === item.id ? { ...u, status: "done", progress: 100 } : u,
              ),
            );
          } catch (err) {
            uploadAbortMapRef.current.delete(item.id);
            if (err instanceof UploadAbortedError) {
              setUploadItems((prev) =>
                prev.map((u) =>
                  u.id === item.id ? { ...u, status: "cancelled" } : u,
                ),
              );
            } else {
              const msg = err instanceof Error ? err.message : String(err);
              setUploadItems((prev) =>
                prev.map((u) =>
                  u.id === item.id ? { ...u, status: "error", error: msg } : u,
                ),
              );
            }
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

  const handleCancelUploadItem = useCallback((id: string) => {
    const abortFn = uploadAbortMapRef.current.get(id);
    if (abortFn) {
      abortFn();
    } else {
      uploadCancelledRef.current.add(id);
      setUploadItems((prev) =>
        prev.map((u) => (u.id === id ? { ...u, status: "cancelled" } : u)),
      );
    }
  }, []);

  const handleCancelAllUploads = useCallback(() => {
    uploadAbortMapRef.current.forEach((abort) => abort());
    uploadAbortMapRef.current.clear();
    setUploadItems((prev) => {
      prev
        .filter((u) => u.status === "pending")
        .forEach((u) => uploadCancelledRef.current.add(u.id));
      return prev.map((u) =>
        u.status === "pending" || u.status === "uploading"
          ? { ...u, status: "cancelled" }
          : u,
      );
    });
  }, []);

  const activeUploadItems = useMemo(
    () => uploadItems.filter((u) => u.status !== "cancelled"),
    [uploadItems],
  );

  const uploadProgressOverall = useMemo(
    () =>
      activeUploadItems.length > 0
        ? Math.round(
            activeUploadItems.reduce((s, u) => s + u.progress, 0) /
              activeUploadItems.length,
          )
        : 0,
    [activeUploadItems],
  );

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

  const previewDialog =
    previewTarget && bearer ? (
      <ShareMediaPreviewDialog
        token={token}
        bearer={bearer}
        target={previewTarget}
        activeDownload={activeDownloads[fileDownloadKey(previewTarget.path)]}
        onDownload={() => handleDownload(previewTarget.path)}
        onCancelDownload={() =>
          handleCancelDownload(fileDownloadKey(previewTarget.path))
        }
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
                Expires {formatExpirationDate(meta.expires_at)}
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

  if (meta?.share_type === "gallery" && bearer) {
    return (
      <PublicGalleryView
        token={token}
        bearer={bearer}
        meta={meta}
      />
    );
  }

  // Browsing — single file download
  if (!meta?.is_directory) {
    const singleDownload = activeDownloads[fileDownloadKey("")];
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
                Expires {formatExpirationDate(meta.expires_at)}
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
                disabled={Boolean(singleDownload)}
                style={{
                  ...secondaryButtonStyle,
                  cursor: singleDownload ? "progress" : "pointer",
                  opacity: singleDownload ? 0.75 : 1,
                }}
              >
                <Icon name="download" size={16} />
                {singleDownload ? downloadButtonLabel(singleDownload) : "Download"}
              </button>
            </div>
          )}
          {singleDownload && (
            <div style={{ width: "100%", marginTop: "var(--space-3)" }}>
              <DownloadProgressBar
                download={singleDownload}
                onCancel={() => handleCancelDownload(fileDownloadKey(""))}
              />
            </div>
          )}
        </div>
        {previewDialog}
      </div>
    );
  }

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
              <span> · Expires {formatExpirationDate(meta.expires_at)}</span>
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
        {meta?.allow_download && (
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              {[
                { path: "", label: "Download all" },
                ...(subPath
                  ? [{ path: subPath, label: "Download folder" }]
                  : []),
              ].map((download) => {
                const key = zipDownloadKey(download.path);
                const activeDownload = activeDownloads[key];
                return (
                  <button
                    key={key}
                    disabled={Boolean(activeDownload)}
                    onClick={() =>
                      handleZipDownload(download.path, download.label)
                    }
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "var(--space-2)",
                      padding: "var(--space-2) var(--space-3)",
                      border: "1px solid var(--color-border)",
                      borderRadius: "var(--radius-md)",
                      background: "var(--color-bg)",
                      color: "var(--color-fg)",
                      cursor: activeDownload ? "progress" : "pointer",
                      opacity: activeDownload ? 0.7 : 1,
                      fontWeight: 500,
                      fontSize: "var(--text-sm)",
                      transition: "background var(--duration-fast)",
                    }}
                    onMouseOver={(e) => {
                      if (!activeDownload)
                        e.currentTarget.style.background =
                          "var(--color-bg-muted)";
                    }}
                    onMouseOut={(e) => {
                      e.currentTarget.style.background = "var(--color-bg)";
                    }}
                  >
                    <Icon name="download" size={16} />
                    {activeDownload
                      ? downloadButtonLabel(activeDownload)
                      : download.label}
                  </button>
                );
              })}
            </div>
            {["", ...(subPath ? [subPath] : [])].map((path) => {
              const key = zipDownloadKey(path);
              const activeDownload = activeDownloads[key];
              return activeDownload ? (
                <DownloadProgressBar
                  key={key}
                  download={activeDownload}
                  onCancel={() => handleCancelDownload(key)}
                />
              ) : null;
            })}
            {zipError && (
              <span
                style={{
                  color: "var(--color-danger)",
                  fontSize: "var(--text-xs)",
                }}
              >
                {zipError}
              </span>
            )}
          </div>
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
                const activeFileDownload =
                  activeDownloads[fileDownloadKey(entryPath)];

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
                      cursor: activeFileDownload ? "progress" : "pointer",
                      transition:
                        "background var(--duration-fast) var(--ease-out)",
                    }}
                    onClick={() => {
                      if (activeFileDownload) return;
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
                      {activeFileDownload && (
                        <span
                          style={{
                            fontSize: "var(--text-xs)",
                            color: "var(--color-accent)",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {downloadStatusLabel(activeFileDownload)}
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
                      {activeFileDownload ? (
                        <DownloadProgressBar
                          download={activeFileDownload}
                          compact
                          onCancel={() =>
                            handleCancelDownload(fileDownloadKey(entryPath))
                          }
                        />
                      ) : entry.is_dir ? (
                        "—"
                      ) : (
                        formatFileSize(entry.size)
                      )}
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
              alignItems: "center",
              marginBottom: "var(--space-2)",
              fontSize: "var(--text-sm)",
              fontWeight: 600,
            }}
          >
            <span>
              Uploading{" "}
              {activeUploadItems.filter((u) => u.status === "done").length}/
              {activeUploadItems.length}
            </span>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
              }}
            >
              <span style={{ color: "var(--color-fg-muted)" }}>
                {uploadProgressOverall}%
              </span>
              {uploadItems.some(
                (u) => u.status === "pending" || u.status === "uploading",
              ) && (
                <button
                  onClick={handleCancelAllUploads}
                  title="Cancel all uploads"
                  style={{
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    padding: "0 2px",
                    color: "var(--color-fg-muted)",
                    fontSize: "var(--text-sm)",
                    lineHeight: 1,
                  }}
                >
                  ✕
                </button>
              )}
            </div>
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
                      : item.status === "cancelled"
                        ? "–"
                        : "⋯"}
                </span>
                <span
                  style={{
                    flex: 1,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    color:
                      item.status === "cancelled"
                        ? "var(--color-fg-muted)"
                        : undefined,
                  }}
                >
                  {item.name}
                </span>
                {item.status === "pending" || item.status === "uploading" ? (
                  <button
                    onClick={() => handleCancelUploadItem(item.id)}
                    title="Cancel upload"
                    style={{
                      background: "none",
                      border: "none",
                      cursor: "pointer",
                      padding: "0 2px",
                      color: "var(--color-fg-subtle)",
                      fontSize: "var(--text-xs)",
                      lineHeight: 1,
                    }}
                  >
                    ✕
                  </button>
                ) : item.status === "cancelled" ? (
                  <span style={{ color: "var(--color-fg-subtle)" }}>
                    Cancelled
                  </span>
                ) : item.status !== "error" ? (
                  <span
                    className="tabular-nums"
                    style={{ color: "var(--color-fg-subtle)" }}
                  >
                    {item.progress}%
                  </span>
                ) : null}
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

function downloadButtonLabel(download?: ActiveDownload) {
  if (!download) return "Downloading…";
  if (download.status === "cancelled") return "Cancelled";
  if (download.pct !== null) return `Downloading ${download.pct}%`;
  return "Downloading…";
}

function downloadStatusLabel(download: ActiveDownload) {
  if (download.status === "cancelled") return "Cancelled";
  if (download.pct !== null) return `${download.pct}%`;
  if (download.loadedBytes > 0) return formatFileSize(download.loadedBytes);
  return "Starting";
}

function downloadDetailLabel(download: ActiveDownload) {
  if (download.status === "cancelled") return "Cancelled";
  if (download.totalBytes && download.totalBytes > 0) {
    return `${formatFileSize(download.loadedBytes)} / ${formatFileSize(download.totalBytes)}`;
  }
  if (download.loadedBytes > 0) return formatFileSize(download.loadedBytes);
  return "Starting transfer";
}

function DownloadProgressBar({
  download,
  compact = false,
  dark = false,
  onCancel,
}: {
  download: ActiveDownload;
  compact?: boolean;
  dark?: boolean;
  onCancel?: () => void;
}) {
  const pct = download.pct;
  const hasPct = pct !== null;
  const canCancel =
    download.status === "downloading" && download.pct !== 100 && onCancel;
  return (
    <div style={{ minWidth: compact ? 80 : 0 }}>
      {!compact && (
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            gap: "var(--space-2)",
            marginBottom: 4,
            fontSize: "var(--text-xs)",
            color: dark ? "rgba(255,255,255,0.72)" : "var(--color-fg-muted)",
          }}
        >
          <span
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {download.label}
          </span>
          <span className="tabular-nums" style={{ whiteSpace: "nowrap" }}>
            {downloadDetailLabel(download)}
          </span>
          {canCancel && (
            <button
              type="button"
              onClick={onCancel}
              title="Cancel download"
              style={{
                background: "none",
                border: "none",
                cursor: "pointer",
                padding: "0 2px",
                color: dark ? "rgba(255,255,255,0.76)" : "var(--color-fg-muted)",
                fontSize: "var(--text-xs)",
                lineHeight: 1,
              }}
            >
              ✕
            </button>
          )}
        </div>
      )}
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <div
          style={{
            flex: 1,
            height: compact ? 4 : 5,
            borderRadius: 3,
            background: dark ? "rgba(255,255,255,0.18)" : "var(--color-border)",
            overflow: "hidden",
          }}
        >
          <div
            className={hasPct ? undefined : "operation-progress-indeterminate"}
            style={{
              height: "100%",
              width: hasPct ? `${Math.max(2, Math.min(100, pct))}%` : "42%",
              borderRadius: 3,
              background:
                download.status === "cancelled"
                  ? "var(--color-fg-muted)"
                  : dark
                    ? "#fff"
                    : "var(--color-accent)",
              transition: hasPct ? "width 160ms ease-out" : undefined,
            }}
          />
        </div>
        {compact && canCancel && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onCancel();
            }}
            title="Cancel download"
            style={{
              background: "none",
              border: "none",
              cursor: "pointer",
              padding: 0,
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-xs)",
              lineHeight: 1,
            }}
          >
            ✕
          </button>
        )}
      </div>
    </div>
  );
}

function PublicGalleryView({
  token,
  bearer,
  meta,
}: {
  token: string;
  bearer: string;
  meta: ShareMeta;
}) {
  const [items, setItems] = useState<GalleryItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState<Record<string, boolean>>({});
  const [assetFailures, setAssetFailures] = useState<Record<string, boolean>>({});

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .publicGallery(token, bearer)
      .then((resp) => {
        if (cancelled) return;
        setItems(resp.items);
        setSelectedId(resp.items[0]?.id ?? null);
        setError("");
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [bearer, token]);

  const selected = items.find((item) => item.id === selectedId) ?? items[0] ?? null;
  const markedCount = items.filter((item) => item.marked).length;
  const selectedIndex = selected
    ? items.findIndex((item) => item.id === selected.id)
    : -1;

  const selectOffset = useCallback(
    (delta: number) => {
      if (items.length === 0) return;
      const current = selectedIndex >= 0 ? selectedIndex : 0;
      const next = Math.min(items.length - 1, Math.max(0, current + delta));
      setSelectedId(items[next]?.id ?? null);
    },
    [items, selectedIndex],
  );

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLTextAreaElement) return;
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        selectOffset(-1);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        selectOffset(1);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [selectOffset]);

  const updateItem = useCallback(
    async (item: GalleryItem, patch: { marked?: boolean; note?: string | null }) => {
      const next = {
        ...item,
        marked: patch.marked ?? item.marked,
        note: patch.note === undefined ? item.note : patch.note,
      };
      setItems((prev) => prev.map((entry) => (entry.id === item.id ? next : entry)));
      setSaving((prev) => ({ ...prev, [item.id]: true }));
      try {
        await api.updatePublicGalleryFeedback(token, bearer, item.id, {
          marked: next.marked,
          note: next.note,
        });
      } catch (err) {
        setError(String(err));
        setItems((prev) => prev.map((entry) => (entry.id === item.id ? item : entry)));
      } finally {
        setSaving((prev) => ({ ...prev, [item.id]: false }));
      }
    },
    [bearer, token],
  );

  const markAssetFailed = useCallback((itemId: string, asset: "thumbnail" | "preview") => {
    setAssetFailures((prev) => ({ ...prev, [`${itemId}:${asset}`]: true }));
  }, []);

  return (
    <div
      className="public-gallery"
      style={{
        display: "grid",
        gridTemplateColumns: "minmax(260px, 360px) minmax(0, 1fr)",
        gridTemplateRows: "minmax(0, 1fr)",
        height: "100vh",
        overflow: "hidden",
        background: "var(--color-bg)",
      }}
    >
      <aside
        className="public-gallery-sidebar"
        style={{
          borderRight: "1px solid var(--color-border)",
          display: "flex",
          flexDirection: "column",
          minWidth: 0,
        }}
      >
        <div
          className="public-gallery-header"
          style={{
            padding: "var(--space-4)",
            borderBottom: "1px solid var(--color-border)",
          }}
        >
          <h1 style={{ margin: 0, fontSize: "var(--text-base)", fontWeight: 600 }}>
            {meta.name}
          </h1>
          <div
            style={{
              marginTop: 4,
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-xs)",
            }}
          >
            Shared by {meta.owner_display_name}
            {meta.expires_at && (
              <span> · Expires {formatExpirationDate(meta.expires_at)}</span>
            )}
          </div>
          <div
            style={{
              marginTop: "var(--space-3)",
              display: "flex",
              gap: "var(--space-2)",
              color: "var(--color-fg-muted)",
              fontSize: "var(--text-xs)",
            }}
          >
            <span>{items.length} photos</span>
            <span>{markedCount} marked</span>
          </div>
        </div>

        {error && (
          <div
            style={{
              margin: "var(--space-3)",
              padding: "var(--space-2) var(--space-3)",
              border: "1px solid var(--color-danger)",
              borderRadius: "var(--radius-md)",
              color: "var(--color-danger)",
              fontSize: "var(--text-sm)",
            }}
          >
            {error}
          </div>
        )}

        <div
          className="public-gallery-grid"
          style={{
            overflow: "auto",
            padding: "var(--space-3)",
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(128px, 1fr))",
            gap: "var(--space-2)",
            alignContent: "start",
          }}
        >
          {loading ? (
            <div style={{ color: "var(--color-fg-muted)", fontSize: "var(--text-sm)" }}>
              Loading gallery…
            </div>
          ) : items.length === 0 ? (
            <div style={{ color: "var(--color-fg-muted)", fontSize: "var(--text-sm)" }}>
              No gallery images are ready yet.
            </div>
          ) : (
            items.map((item) => (
              <button
                key={item.id}
                onClick={() => setSelectedId(item.id)}
                style={{
                  border: "1px solid",
                  borderColor:
                    item.id === selected?.id ? "var(--color-accent)" : "var(--color-border)",
                  borderRadius: "var(--radius-md)",
                  background: item.marked
                    ? "var(--color-accent-muted)"
                    : "var(--color-bg)",
                  padding: 4,
                  cursor: "pointer",
                  textAlign: "left",
                  color: "var(--color-fg)",
                }}
              >
                <div
                  style={{
                    aspectRatio: "4/3",
                    borderRadius: "var(--radius-sm)",
                    overflow: "hidden",
                    background: "var(--color-bg-muted)",
                  }}
                >
                  {!assetFailures[`${item.id}:thumbnail`] ? (
                    <img
                      src={api.publicGalleryAssetUrl(token, bearer, item.id, "thumbnail")}
                      alt=""
                      loading="lazy"
                      onError={() => markAssetFailed(item.id, "thumbnail")}
                      style={{ width: "100%", height: "100%", objectFit: "cover" }}
                    />
                  ) : (
                    <div
                      style={{
                        height: "100%",
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "center",
                        color: "var(--color-fg-muted)",
                      }}
                    >
                      <Icon name="file" size={22} />
                    </div>
                  )}
                </div>
                <div
                  style={{
                    marginTop: 4,
                    display: "flex",
                    justifyContent: "space-between",
                    gap: 6,
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-muted)",
                  }}
                >
                  <span>{item.sequence}</span>
                  <span>{item.marked ? "Marked" : ""}</span>
                </div>
              </button>
            ))
          )}
        </div>
      </aside>

      <main
        className="public-gallery-main"
        style={{
          minHeight: 0,
          height: "100%",
          minWidth: 0,
          display: "grid",
          gridTemplateRows: "minmax(0, 1fr) auto",
          overflow: "hidden",
        }}
      >
        {selected ? (
          <>
            <div
              style={{
                minHeight: 0,
                overflow: "hidden",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                background: "var(--color-bg-muted)",
                padding: "var(--space-4)",
                position: "relative",
              }}
            >
              <button
                onClick={() => selectOffset(-1)}
                disabled={selectedIndex <= 0}
                aria-label="Previous picture"
                title="Previous picture"
                style={{
                  ...galleryNavButtonStyle,
                  left: "var(--space-4)",
                  opacity: selectedIndex <= 0 ? 0.35 : 1,
                  cursor: selectedIndex <= 0 ? "default" : "pointer",
                }}
              >
                <Icon name="arrowLeft" size={18} />
                <span>Previous</span>
              </button>
              {!assetFailures[`${selected.id}:preview`] ? (
                <img
                  src={api.publicGalleryAssetUrl(token, bearer, selected.id, "preview")}
                  alt={selected.filename}
                  onError={() => markAssetFailed(selected.id, "preview")}
                  style={{
                    maxWidth: "100%",
                    maxHeight: "100%",
                    objectFit: "contain",
                    borderRadius: "var(--radius-md)",
                  }}
                />
              ) : (
                <div style={{ color: "var(--color-fg-muted)" }}>
                  Preview is not available for this file.
                </div>
              )}
              <button
                onClick={() => selectOffset(1)}
                disabled={selectedIndex >= items.length - 1}
                aria-label="Next picture"
                title="Next picture"
                style={{
                  ...galleryNavButtonStyle,
                  right: "var(--space-4)",
                  opacity: selectedIndex >= items.length - 1 ? 0.35 : 1,
                  cursor:
                    selectedIndex >= items.length - 1 ? "default" : "pointer",
                }}
              >
                <span>Next</span>
                <Icon name="chevronRight" size={18} />
              </button>
            </div>
            <section
              className="public-gallery-detail"
              style={{
                borderTop: "1px solid var(--color-border)",
                padding: "var(--space-4)",
                display: "grid",
                gridTemplateColumns: "minmax(0, 1fr) minmax(260px, 360px)",
                gap: "var(--space-4)",
              }}
            >
              <div style={{ minWidth: 0 }}>
                <h2 style={{ margin: 0, fontSize: "var(--text-base)", fontWeight: 600 }}>
                  {selected.filename}
                </h2>
                <div
                  style={{
                    marginTop: 4,
                    color: "var(--color-fg-muted)",
                    fontSize: "var(--text-sm)",
                    display: "flex",
                    gap: "var(--space-3)",
                    flexWrap: "wrap",
                  }}
                >
                  <span>
                    {selected.sequence} / {items.length}
                  </span>
                  {selected.captured_at && (
                    <span>{new Date(selected.captured_at).toLocaleString()}</span>
                  )}
                  {selected.width && selected.height && (
                    <span>
                      {selected.width} × {selected.height}
                    </span>
                  )}
                  <span>{formatFileSize(selected.source_size)}</span>
                </div>
              </div>
              <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
                {meta.allow_download && (
                  <a
                    href={api.shareDownloadUrl(token, bearer, selected.relative_path)}
                    style={{
                      ...secondaryButtonStyle,
                      justifyContent: "center",
                      textDecoration: "none",
                    }}
                  >
                    <Icon name="download" size={16} />
                    Download Original
                  </a>
                )}
                <button
                  onClick={() => updateItem(selected, { marked: !selected.marked })}
                  disabled={saving[selected.id]}
                  style={{
                    ...primaryButtonStyle,
                    justifyContent: "center",
                    border: selected.marked
                      ? "1px solid var(--color-success)"
                      : primaryButtonStyle.border,
                    background: selected.marked
                      ? "var(--color-success)"
                      : primaryButtonStyle.background,
                    color: selected.marked
                      ? "var(--color-accent-fg)"
                      : primaryButtonStyle.color,
                    opacity: saving[selected.id] ? 0.7 : 1,
                  }}
                >
                  <Icon name={selected.marked ? "check" : "plus"} size={16} />
                  {selected.marked ? "Marked" : "Mark"}
                </button>
                <textarea
                  value={selected.note ?? ""}
                  onChange={(e) =>
                    setItems((prev) =>
                      prev.map((item) =>
                        item.id === selected.id ? { ...item, note: e.target.value } : item,
                      ),
                    )
                  }
                  onBlur={(e) => updateItem(selected, { note: e.target.value })}
                  placeholder="Notes"
                  rows={4}
                  style={{
                    width: "100%",
                    resize: "vertical",
                    boxSizing: "border-box",
                    padding: "var(--space-2) var(--space-3)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    background: "var(--color-bg)",
                    color: "var(--color-fg)",
                    fontSize: "var(--text-sm)",
                    fontFamily: "inherit",
                  }}
                />
              </div>
            </section>
          </>
        ) : (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--color-fg-muted)",
            }}
          >
            Select a photo.
          </div>
        )}
      </main>
    </div>
  );
}

function ShareMediaPreviewDialog({
  token,
  bearer,
  target,
  activeDownload,
  onDownload,
  onCancelDownload,
  onClose,
}: {
  token: string;
  bearer: string;
  target: { entry: FileEntry; path: string };
  activeDownload?: ActiveDownload;
  onDownload: () => void;
  onCancelDownload: () => void;
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
          <button
            type="button"
            onClick={onDownload}
            disabled={Boolean(activeDownload)}
            style={previewIconButtonStyle}
            title={activeDownload ? downloadStatusLabel(activeDownload) : "Download"}
          >
            <Icon name="download" size={16} color="#fff" />
          </button>
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
      {activeDownload && (
        <div
          data-preview-no-close
          style={{
            position: "absolute",
            top: 52,
            right: "var(--space-3)",
            width: 180,
            zIndex: 10,
          }}
        >
          <DownloadProgressBar
            download={activeDownload}
            dark
            onCancel={onCancelDownload}
          />
        </div>
      )}

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

const galleryNavButtonStyle: React.CSSProperties = {
  position: "absolute",
  top: "50%",
  transform: "translateY(-50%)",
  zIndex: 2,
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "var(--space-2) var(--space-3)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "color-mix(in oklch, var(--color-bg) 88%, transparent)",
  color: "var(--color-fg)",
  boxShadow: "var(--shadow-sm)",
  fontSize: "var(--text-sm)",
  fontWeight: 600,
  backdropFilter: "blur(12px)",
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
