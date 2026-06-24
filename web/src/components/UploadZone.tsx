import {
  useState,
  useRef,
  useCallback,
  useEffect,
  forwardRef,
  useImperativeHandle,
} from "react";
import api, { UploadAbortedError } from "../api/client";
import { Icon } from "./Icon";
import {
  getExternalDropFiles,
  hasExternalFileDrag,
  hasNasfilesDrag,
  isExternalDropHandled,
  markExternalDropHandled,
} from "../lib/fileDrag";
import { useGlobalDragCleanup } from "../lib/dragState";

interface UploadZoneProps {
  root: string;
  path: string;
  children: React.ReactNode;
  onUploadComplete: (targetRoot: string, targetPath: string) => void;
  canUpload?: boolean;
}

interface UploadItem {
  id: string;
  file: File;
  progress: number;
  status: "pending" | "uploading" | "done" | "error" | "cancelled";
  error?: string;
}

export interface UploadZoneHandle {
  trigger: () => void;
  uploadTo: (targetRoot: string, targetPath: string, files: File[]) => void;
}

export const UploadZone = forwardRef<UploadZoneHandle, UploadZoneProps>(
  ({ root, path, children, onUploadComplete, canUpload = true }, ref) => {
    const [dragStatus, setDragStatus] = useState<"accept" | "reject" | null>(
      null,
    );
    const [uploads, setUploads] = useState<UploadItem[]>([]);
    const [showProgress, setShowProgress] = useState(false);
    const dragCounter = useRef(0);
    const containerRef = useRef<HTMLDivElement>(null);
    const fileInputRef = useRef<HTMLInputElement>(null);
    const hideTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const abortMapRef = useRef<Map<string, () => void>>(new Map());
    const cancelledIdsRef = useRef<Set<string>>(new Set());

    const resetDragState = useCallback(() => {
      dragCounter.current = 0;
      setDragStatus(null);
    }, []);

    useGlobalDragCleanup(resetDragState);

    useEffect(() => {
      if (!canUpload) resetDragState();
    }, [canUpload, resetDragState]);

    // Cancel any pending auto-hide timer on unmount.
    useEffect(() => {
      return () => {
        if (hideTimerRef.current) clearTimeout(hideTimerRef.current);
      };
    }, []);

    const isWithinUploadZone = useCallback((e: DragEvent) => {
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return false;
      return (
        e.clientX >= rect.left &&
        e.clientX <= rect.right &&
        e.clientY >= rect.top &&
        e.clientY <= rect.bottom
      );
    }, []);

    const handleDragEnter = useCallback(
      (e: React.DragEvent) => {
        if (!canUpload) return;
        if (hasNasfilesDrag(e.dataTransfer)) return;
        if (!hasExternalFileDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        dragCounter.current++;
        setDragStatus(canUpload ? "accept" : "reject");
      },
      [canUpload],
    );

    const handleDragLeave = useCallback(
      (e: React.DragEvent) => {
        if (!canUpload) return;
        if (hasNasfilesDrag(e.dataTransfer)) return;
        if (!hasExternalFileDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        dragCounter.current = Math.max(0, dragCounter.current - 1);
        if (dragCounter.current === 0) {
          setDragStatus(null);
        }
      },
      [canUpload],
    );

    const handleDragOver = useCallback(
      (e: React.DragEvent) => {
        if (!canUpload) return;
        if (hasNasfilesDrag(e.dataTransfer)) return;
        if (!hasExternalFileDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "copy";
      },
      [canUpload],
    );

    const uploadFiles = useCallback(
      async (files: File[], targetRoot = root, targetPath = path) => {
        if (files.length === 0) return;

        // Cancel any pending auto-hide from a previous batch.
        if (hideTimerRef.current) {
          clearTimeout(hideTimerRef.current);
          hideTimerRef.current = null;
        }

        abortMapRef.current.clear();
        cancelledIdsRef.current.clear();

        // Assign stable IDs so concurrent batches don't corrupt each other via index.
        const items: UploadItem[] = files.map((f, i) => ({
          id: `${Date.now()}-${i}-${f.name}`,
          file: f,
          progress: 0,
          status: "pending" as const,
        }));
        setUploads(items);
        setShowProgress(true);

        // Upload in batches of 3, using ID-based state updates.
        const batchSize = 3;
        for (let i = 0; i < items.length; i += batchSize) {
          const batch = items.slice(i, i + batchSize);
          await Promise.allSettled(
            batch.map(async (item) => {
              if (cancelledIdsRef.current.has(item.id)) {
                setUploads((prev) =>
                  prev.map((u) =>
                    u.id === item.id ? { ...u, status: "cancelled" } : u,
                  ),
                );
                return;
              }
              setUploads((prev) =>
                prev.map((u) =>
                  u.id === item.id ? { ...u, status: "uploading" } : u,
                ),
              );
              const handle = api.upload(
                targetRoot,
                targetPath,
                [item.file],
                (pct) => {
                  setUploads((prev) =>
                    prev.map((u) =>
                      u.id === item.id ? { ...u, progress: pct } : u,
                    ),
                  );
                },
              );
              abortMapRef.current.set(item.id, handle.abort);
              try {
                await handle.promise;
                abortMapRef.current.delete(item.id);
                setUploads((prev) =>
                  prev.map((u) =>
                    u.id === item.id
                      ? { ...u, status: "done", progress: 100 }
                      : u,
                  ),
                );
              } catch (err) {
                abortMapRef.current.delete(item.id);
                if (err instanceof UploadAbortedError) {
                  setUploads((prev) =>
                    prev.map((u) =>
                      u.id === item.id ? { ...u, status: "cancelled" } : u,
                    ),
                  );
                } else {
                  const msg = err instanceof Error ? err.message : String(err);
                  setUploads((prev) =>
                    prev.map((u) =>
                      u.id === item.id
                        ? { ...u, status: "error", error: msg }
                        : u,
                    ),
                  );
                }
              }
            }),
          );
        }

        onUploadComplete(targetRoot, targetPath);
        hideTimerRef.current = setTimeout(() => {
          setShowProgress(false);
          setUploads([]);
          hideTimerRef.current = null;
        }, 2000);
      },
      [root, path, onUploadComplete],
    );

    const handleCancelItem = useCallback((id: string) => {
      const abortFn = abortMapRef.current.get(id);
      if (abortFn) {
        abortFn();
      } else {
        cancelledIdsRef.current.add(id);
        setUploads((prev) =>
          prev.map((u) => (u.id === id ? { ...u, status: "cancelled" } : u)),
        );
      }
    }, []);

    const handleCancelAll = useCallback(() => {
      abortMapRef.current.forEach((abort) => abort());
      abortMapRef.current.clear();
      setUploads((prev) => {
        prev
          .filter((u) => u.status === "pending")
          .forEach((u) => cancelledIdsRef.current.add(u.id));
        return prev.map((u) =>
          u.status === "pending" || u.status === "uploading"
            ? { ...u, status: "cancelled" }
            : u,
        );
      });
    }, []);

    useEffect(() => {
      if (typeof window === "undefined") return;

      const handleNativeDragOver = (e: DragEvent) => {
        const dataTransfer = e.dataTransfer;
        if (
          !dataTransfer ||
          hasNasfilesDrag(dataTransfer) ||
          !hasExternalFileDrag(dataTransfer)
        )
          return;

        e.preventDefault();
        if (isWithinUploadZone(e) && canUpload) {
          dataTransfer.dropEffect = "copy";
          setDragStatus("accept");
        } else {
          dataTransfer.dropEffect = "none";
          setDragStatus("reject");
        }
      };

      const handleNativeDrop = (e: DragEvent) => {
        const dataTransfer = e.dataTransfer;
        if (
          !dataTransfer ||
          hasNasfilesDrag(dataTransfer) ||
          !hasExternalFileDrag(dataTransfer)
        )
          return;

        e.preventDefault();
        const isAcceptedTarget = isWithinUploadZone(e) && canUpload;
        if (!isAcceptedTarget) {
          e.stopPropagation();
          setDragStatus("reject");
          window.setTimeout(resetDragState, 900);
          return;
        }

        const files = getExternalDropFiles(dataTransfer);
        window.setTimeout(() => {
          if (isExternalDropHandled(e)) return;
          markExternalDropHandled(e);
          if (files.length === 0) {
            setDragStatus("reject");
            window.setTimeout(resetDragState, 900);
            return;
          }
          resetDragState();
          void uploadFiles(files);
        }, 0);
      };

      window.addEventListener("dragover", handleNativeDragOver, true);
      window.addEventListener("drop", handleNativeDrop, true);

      return () => {
        window.removeEventListener("dragover", handleNativeDragOver, true);
        window.removeEventListener("drop", handleNativeDrop, true);
      };
    }, [canUpload, isWithinUploadZone, resetDragState, uploadFiles]);

    useImperativeHandle(
      ref,
      () => ({
        trigger: () => fileInputRef.current?.click(),
        uploadTo: (targetRoot, targetPath, files) => {
          void uploadFiles(files, targetRoot, targetPath);
        },
      }),
      [uploadFiles],
    );

    const handleDrop = useCallback(
      async (e: React.DragEvent) => {
        if (hasNasfilesDrag(e.dataTransfer)) return;
        if (!hasExternalFileDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        markExternalDropHandled(e.nativeEvent);
        resetDragState();
        if (!canUpload) {
          setDragStatus("reject");
          window.setTimeout(resetDragState, 900);
          return;
        }
        const files = getExternalDropFiles(e.dataTransfer);
        if (files.length === 0) {
          setDragStatus("reject");
          window.setTimeout(resetDragState, 900);
          return;
        }
        await uploadFiles(files);
      },
      [canUpload, resetDragState, uploadFiles],
    );

    const handleFileSelect = useCallback(
      async (e: React.ChangeEvent<HTMLInputElement>) => {
        const files = Array.from(e.target.files || []);
        await uploadFiles(files);
        if (fileInputRef.current) fileInputRef.current.value = "";
      },
      [uploadFiles],
    );

    const activeUploads = uploads.filter((u) => u.status !== "cancelled");
    const totalFiles = activeUploads.length;
    const doneFiles = activeUploads.filter((u) => u.status === "done").length;
    const overallProgress =
      totalFiles > 0
        ? Math.round(
            activeUploads.reduce((sum, u) => sum + u.progress, 0) / totalFiles,
          )
        : 0;

    return (
      <div
        ref={containerRef}
        style={{
          position: "relative",
          flex: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        }}
        onDragEnter={handleDragEnter}
        onDragLeave={handleDragLeave}
        onDragOver={handleDragOver}
        onDrop={handleDrop}
      >
        {/* Hidden file input triggered programmatically via ref.trigger() */}
        <input
          ref={fileInputRef}
          type="file"
          multiple
          style={{ display: "none" }}
          onChange={handleFileSelect}
        />

        {children}

        {dragStatus && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              background:
                dragStatus === "accept"
                  ? "rgba(59, 130, 246, 0.08)"
                  : "rgba(239, 68, 68, 0.1)",
              border: `2px dashed ${
                dragStatus === "accept"
                  ? "var(--color-accent)"
                  : "var(--color-danger)"
              }`,
              borderRadius: "var(--radius-lg)",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              justifyContent: "center",
              gap: "var(--space-3)",
              zIndex: 20,
              backdropFilter: "blur(2px)",
            }}
            className="fade-in"
          >
            <Icon
              name={dragStatus === "accept" ? "folder" : "alertTriangle"}
              size={48}
              color={
                dragStatus === "accept"
                  ? "var(--color-accent)"
                  : "var(--color-danger)"
              }
              style={{ opacity: 0.7 }}
            />
            <div
              style={{
                fontSize: "var(--text-lg)",
                fontWeight: 600,
                color:
                  dragStatus === "accept"
                    ? "var(--color-accent)"
                    : "var(--color-danger)",
              }}
            >
              {dragStatus === "accept"
                ? "Drop to upload"
                : "Cannot upload here"}
            </div>
            <div
              style={{
                fontSize: "var(--text-sm)",
                color: "var(--color-fg-muted)",
              }}
            >
              {dragStatus === "accept"
                ? "Files will be uploaded to the current folder"
                : "This location does not accept dropped files"}
            </div>
          </div>
        )}

        {showProgress && (
          <div
            style={{
              position: "absolute",
              bottom: "var(--space-4)",
              right: "var(--space-4)",
              width: 320,
              background: "var(--color-bg)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-lg)",
              boxShadow: "var(--shadow-lg)",
              padding: "var(--space-4)",
              zIndex: 30,
            }}
            className="slide-in"
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                marginBottom: "var(--space-3)",
              }}
            >
              <span style={{ fontWeight: 600, fontSize: "var(--text-sm)" }}>
                Uploading {doneFiles}/{totalFiles}
              </span>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                }}
              >
                <span
                  style={{
                    fontSize: "var(--text-sm)",
                    color: "var(--color-fg-muted)",
                  }}
                >
                  {overallProgress}%
                </span>
                {uploads.some(
                  (u) => u.status === "pending" || u.status === "uploading",
                ) && (
                  <button
                    onClick={handleCancelAll}
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
                  width: `${overallProgress}%`,
                  background: "var(--color-accent)",
                  borderRadius: 2,
                  transition: "width 200ms ease-out",
                }}
              />
            </div>

            <div style={{ maxHeight: 140, overflowY: "auto" }}>
              {uploads.slice(0, 10).map((item) => (
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
                            : "var(--color-fg)",
                      }}
                    >
                      {item.file.name}
                    </span>
                    {item.status === "pending" ||
                    item.status === "uploading" ? (
                      <button
                        onClick={() => handleCancelItem(item.id)}
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
          </div>
        )}
      </div>
    );
  },
);

UploadZone.displayName = "UploadZone";
