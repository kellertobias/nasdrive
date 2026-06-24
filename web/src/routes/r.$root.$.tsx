import {
  createFileRoute,
  useNavigate,
  useParams,
} from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import api, { formatApiError, formatApiErrorDetails } from "../api/client";
import type { FileEntry } from "../api/client";
import type { ExtractMode } from "../api/client";
import { FolderTree } from "../components/FolderTree";
import { FileGrid } from "../components/FileGrid";
import { FileList } from "../components/FileList";
import { ColumnBrowser } from "../components/ColumnBrowser";
import { TopBar } from "../components/TopBar";
import { Breadcrumb } from "../components/Breadcrumb";
import { EmptyState } from "../components/EmptyState";
import { Icon } from "../components/Icon";
import { UploadZone, type UploadZoneHandle } from "../components/UploadZone";
import { CreateFolderDialog } from "../components/CreateFolderDialog";
import { RenameDialog } from "../components/RenameDialog";
import { ContextMenu } from "../components/ContextMenu";
import type { ContextMenuItem } from "../components/ContextMenu";
import { ShareDialog } from "../components/ShareDialog";
import { PreviewPane } from "../components/PreviewPane";
import { DirectoryReadme } from "../components/DirectoryReadme";
import {
  FileDetailsPane,
  type FileDetailsSelection,
} from "../components/FileDetailsPane";
import { ErrorDialog, ErrorToasts } from "../components/ErrorNotice";
import type { ErrorNoticeData } from "../components/ErrorNotice";
import { useViewStore } from "../state/view";
import {
  getExternalDropFiles,
  getFileDragPayload,
  hasExternalFileDrag,
  hasNasfilesDrag,
  isDemoDropTarget,
  isSelfOrDescendantDrop,
  markExternalDropHandled,
} from "../lib/fileDrag";
import { useGlobalDragCleanup } from "../lib/dragState";
import {
  isActiveTransferJob,
  transferJobsForTarget,
} from "../lib/transferJobs";
import { formatFileSize } from "../lib/icons";
import type { DirectoryListing, TransferJob } from "../api/client";

export const Route = createFileRoute("/r/$root/$")({
  component: FileBrowser,
});

interface DeleteJobNotice {
  id: number;
  count: number;
}

const SIDEBAR_WIDTH = { min: 180, max: 420 };
const DEMO_TRANSFER_JOB_STORAGE_KEY = "nasfiles-demo-transfer-job";

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function isEditableShortcutTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;

  const editable = target.closest(
    'input, textarea, select, [role="textbox"], [contenteditable]',
  );
  if (!editable) return false;

  if (
    editable instanceof HTMLElement &&
    editable.getAttribute("contenteditable") === "false"
  ) {
    return false;
  }

  return true;
}

function demoTransferJobsFromLocalStorage(): TransferJob[] {
  if (typeof window === "undefined") return [];

  const raw = window.localStorage.getItem(DEMO_TRANSFER_JOB_STORAGE_KEY);
  if (!raw) return [];

  try {
    const value = JSON.parse(raw) as
      | Partial<TransferJob>
      | Partial<TransferJob>[];
    const values = Array.isArray(value) ? value : [value];
    const now = Date.now();

    return values
      .filter(
        (job) =>
          (job.operation === "move" || job.operation === "copy") &&
          typeof job.source_root === "string" &&
          typeof job.dest_root === "string" &&
          typeof job.dest_path === "string" &&
          Array.isArray(job.paths) &&
          job.paths.every((path) => typeof path === "string"),
      )
      .map((job, index) => ({
        id: typeof job.id === "string" ? job.id : `demo-transfer-${index}`,
        operation: job.operation as "move" | "copy",
        source_root: job.source_root as string,
        dest_root: job.dest_root as string,
        dest_path: job.dest_path as string,
        paths: job.paths as string[],
        status: job.status === "queued" ? "queued" : "running",
        total_bytes:
          typeof job.total_bytes === "number" ? job.total_bytes : 100,
        transferred_bytes:
          typeof job.transferred_bytes === "number"
            ? job.transferred_bytes
            : 35,
        total_entries:
          typeof job.total_entries === "number"
            ? job.total_entries
            : (job.paths?.length ?? 1),
        completed_entries:
          typeof job.completed_entries === "number" ? job.completed_entries : 0,
        error: null,
        created_at: typeof job.created_at === "number" ? job.created_at : now,
        updated_at: typeof job.updated_at === "number" ? job.updated_at : now,
        finished_at: null,
      }));
  } catch {
    return [];
  }
}

function FileBrowser() {
  const { root } = useParams({ from: "/r/$root/$" });
  const params = Route.useParams();
  const path = params._splat || "";
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const {
    viewMode,
    sidebarOpen,
    sidebarWidth,
    selectedPaths,
    setSidebarWidth,
    setViewMode,
  } = useViewStore();
  const uploadZoneRef = useRef<UploadZoneHandle>(null);
  const listingScrollRef = useRef<HTMLDivElement>(null);
  const errorIdRef = useRef(0);
  const deleteJobIdRef = useRef(0);

  // Dialogs
  const [showCreateFolder, setShowCreateFolder] = useState(false);
  const [showRename, setShowRename] = useState(false);
  const [renameTarget, setRenameTarget] = useState<FileEntry | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showShare, setShowShare] = useState(false);
  const [shareTarget, setShareTarget] = useState<{
    path: string;
    is_dir: boolean;
  } | null>(null);
  const [previewTarget, setPreviewTarget] = useState<{
    entry: FileEntry;
    parentPath: string;
  } | null>(null);
  const [dropTargetActive, setDropTargetActive] = useState(false);
  const [columnDisplayPath, setColumnDisplayPath] = useState(path);
  const [columnActiveFolderPath, setColumnActiveFolderPath] = useState(path);
  const [readmeHidden, setReadmeHidden] = useState(false);
  const [pendingTransfer, setPendingTransfer] = useState<{
    sourceRoot: string;
    paths: string[];
    destRoot: string;
    dest: string;
  } | null>(null);
  const [blockingError, setBlockingError] = useState<ErrorNoticeData | null>(
    null,
  );
  const [errorToasts, setErrorToasts] = useState<ErrorNoticeData[]>([]);
  const [deleteJobs, setDeleteJobs] = useState<DeleteJobNotice[]>([]);
  const resetCurrentDropTarget = useCallback(
    () => setDropTargetActive(false),
    [],
  );

  // Context menu
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    entry: FileEntry;
    parentPath: string;
  } | null>(null);

  const { data: user } = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });

  useGlobalDragCleanup(resetCurrentDropTarget);

  const {
    data: listing,
    isLoading,
    error,
  } = useQuery({
    queryKey: ["listing", root, path],
    queryFn: () => api.listDirectory(root, path),
    staleTime: 10_000,
    refetchInterval: 20_000,
    refetchIntervalInBackground: false,
  });

  const { data: transferJobData } = useQuery({
    queryKey: ["transfer-jobs"],
    queryFn: api.transferJobs,
    enabled: Boolean(user),
    refetchInterval: user ? 1000 : false,
    staleTime: 1000,
  });

  const currentRoot = user?.roots.find((r) => r.key === root);
  const caps = currentRoot?.caps || { read: true, write: false, share: false };
  const serverCapabilities = user?.capabilities ?? {
    archive_extraction: true,
    thumbnails: true,
    media_preview_transcoding: true,
    media_metadata_probe: true,
  };
  const demoTransferJobs = useMemo(
    () => demoTransferJobsFromLocalStorage(),
    [],
  );
  const activeTransferJobs = [
    ...(transferJobData?.jobs ?? []).filter(isActiveTransferJob),
    ...demoTransferJobs,
  ];
  const currentFolderTransferJobs = transferJobsForTarget(
    activeTransferJobs,
    root,
    path,
  );
  const isDemoCurrentFolderDropTarget = isDemoDropTarget(root, path);
  const breadcrumbPath = viewMode === "columns" ? columnDisplayPath : path;

  const makeErrorNotice = useCallback(
    (title: string, err: unknown): ErrorNoticeData => {
      const id = errorIdRef.current + 1;
      errorIdRef.current = id;
      const message = formatApiError(err);
      return {
        id,
        title,
        message,
        details: formatApiErrorDetails(err),
      };
    },
    [],
  );

  const showErrorDialog = useCallback(
    (title: string, err: unknown) => {
      setBlockingError(makeErrorNotice(title, err));
    },
    [makeErrorNotice],
  );

  const showErrorToast = useCallback(
    (title: string, err: unknown) => {
      const notice = makeErrorNotice(title, err);
      setErrorToasts((current) => [...current, notice].slice(-4));
      window.setTimeout(() => {
        setErrorToasts((current) =>
          current.filter((toast) => toast.id !== notice.id),
        );
      }, 7000);
    },
    [makeErrorNotice],
  );

  useEffect(() => {
    setColumnDisplayPath(path);
    setReadmeHidden(false);
  }, [path, root, viewMode]);

  // Clear selection when navigating to a different directory so stale paths
  // can't accidentally be targeted by Delete/F2 keyboard shortcuts.
  useEffect(() => {
    useViewStore.getState().clearSelection();
  }, [path, root]);

  const refreshListing = useCallback(
    (targetRoot = root, targetPath = path) => {
      queryClient.invalidateQueries({
        queryKey: ["listing", targetRoot, targetPath],
      });
      queryClient.invalidateQueries({ queryKey: ["tree", targetRoot] });
    },
    [queryClient, root, path],
  );

  const togglePreviewAtPath = useCallback(
    (entry: FileEntry, parentPath: string) => {
      setPreviewTarget((current) => {
        if (
          current &&
          current.parentPath === parentPath &&
          current.entry.name === entry.name
        ) {
          return null;
        }
        return { entry, parentPath };
      });
    },
    [],
  );

  // Global keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (isEditableShortcutTarget(e.target)) return;
      if (previewTarget) return; // PreviewPane handles its own keys

      if (
        (e.key === " " || e.key === "Space" || e.key === "Spacebar") &&
        !e.repeat
      ) {
        e.preventDefault();
        // Open preview for the first selected file
        const { selectedPaths } = useViewStore.getState();
        if (selectedPaths.size === 1 && listing?.entries) {
          const selectedPath = [...selectedPaths][0];
          const name = selectedPath.split("/").pop();
          const entry = listing.entries.find(
            (f) => f.name === name && !f.is_dir,
          );
          if (entry) togglePreviewAtPath(entry, path);
        }
      } else if (e.key === "Delete" || e.key === "Backspace") {
        const { selectedPaths } = useViewStore.getState();
        if (selectedPaths.size > 0) {
          setShowDeleteConfirm(true);
        }
      } else if (e.key === "F2") {
        const { selectedPaths } = useViewStore.getState();
        if (selectedPaths.size === 1 && listing?.entries) {
          const selectedPath = [...selectedPaths][0];
          const name = selectedPath.split("/").pop();
          const entry = listing.entries.find((f) => f.name === name);
          if (entry) {
            setRenameTarget(entry);
            setShowRename(true);
          }
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [listing, path, previewTarget, togglePreviewAtPath]);

  const navigateTo = (entry: FileEntry) => {
    const newPath = path ? `${path}/${entry.name}` : entry.name;
    if (entry.is_dir) {
      navigate({
        to: "/r/$root/$",
        params: { root, _splat: newPath },
      });
    } else {
      setPreviewTarget({ entry, parentPath: path });
    }
  };

  const openEntryAtPath = (entry: FileEntry, parentPath: string) => {
    const newPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;
    if (entry.is_dir) {
      navigate({
        to: "/r/$root/$",
        params: { root, _splat: newPath },
      });
    } else {
      setPreviewTarget({ entry, parentPath });
    }
  };

  const navigateToPath = useCallback(
    (targetPath: string) => {
      navigate({
        to: "/r/$root/$",
        params: { root, _splat: targetPath },
      });
    },
    [navigate, root],
  );

  const navigateToRoot = useCallback(
    (targetRoot: string) => {
      navigate({
        to: "/r/$root/$",
        params: { root: targetRoot, _splat: "" },
      });
    },
    [navigate],
  );

  const handleColumnDisplayPathChange = useCallback((displayPath: string) => {
    setColumnDisplayPath(displayPath);
  }, []);

  const handleColumnActiveFolderPathChange = useCallback(
    (activeFolderPath: string) => {
      setColumnActiveFolderPath(activeFolderPath);
    },
    [],
  );

  const switchViewMode = useCallback(
    (mode: "grid" | "list" | "columns") => {
      if (
        viewMode === "columns" &&
        mode !== "columns" &&
        columnActiveFolderPath !== path
      ) {
        navigateToPath(columnActiveFolderPath);
      }
      setViewMode(mode);
    },
    [columnActiveFolderPath, navigateToPath, path, setViewMode, viewMode],
  );

  const startSidebarResize = useCallback(
    (e: React.PointerEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = sidebarWidth;
      const onMove = (event: PointerEvent) => {
        setSidebarWidth(
          clamp(
            startWidth + event.clientX - startX,
            SIDEBAR_WIDTH.min,
            SIDEBAR_WIDTH.max,
          ),
        );
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [setSidebarWidth, sidebarWidth],
  );

  // ---- Write operation handlers ----

  const handleCreateFolder = async (name: string) => {
    try {
      await api.mkdir(root, path, name);
      setShowCreateFolder(false);
      refreshListing();
    } catch (err) {
      showErrorDialog("Failed to create folder", err);
    }
  };

  const handleRename = async (newName: string) => {
    if (!renameTarget) return;
    const entryPath = path ? `${path}/${renameTarget.name}` : renameTarget.name;
    try {
      await api.rename(root, entryPath, newName);
      setShowRename(false);
      setRenameTarget(null);
      refreshListing();
    } catch (err) {
      showErrorDialog("Failed to rename", err);
    }
  };

  const handleDelete = async () => {
    const selected = useViewStore.getState().selectedPaths;
    const paths = Array.from(selected);
    if (paths.length === 0) return;

    const noticeId = deleteJobIdRef.current + 1;
    deleteJobIdRef.current = noticeId;
    const notice: DeleteJobNotice = { id: noticeId, count: paths.length };

    setShowDeleteConfirm(false);
    setDeleteJobs((current) => [...current, notice]);
    useViewStore.getState().clearSelection();
    removeDeletedPathsFromListings(queryClient, root, paths);

    try {
      await api.deleteEntries(root, paths);
      queryClient.invalidateQueries({ queryKey: ["listing", root] });
      queryClient.invalidateQueries({ queryKey: ["tree", root] });
      queryClient.invalidateQueries({ queryKey: ["roots"] });
    } catch (err) {
      queryClient.invalidateQueries({ queryKey: ["listing", root] });
      queryClient.invalidateQueries({ queryKey: ["tree", root] });
      showErrorDialog("Failed to delete", err);
    } finally {
      setDeleteJobs((current) => current.filter((job) => job.id !== notice.id));
    }
  };

  const executeTransfer = useCallback(
    async (
      sourceRoot: string,
      paths: string[],
      destRoot: string,
      dest: string,
      operation: "move" | "copy",
    ) => {
      try {
        await api.transferEntries(sourceRoot, paths, destRoot, dest, operation);
        useViewStore.getState().clearSelection();
      } catch (err) {
        showErrorDialog(`Failed to ${operation}`, err);
      }
    },
    [showErrorDialog],
  );

  const handleFileDrop = useCallback(
    (targetRoot: string, targetPath: string, e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      resetCurrentDropTarget();

      const targetRootInfo = user?.roots.find((r) => r.key === targetRoot);
      if (!targetRootInfo?.caps.write) {
        showErrorToast(
          "Drop blocked",
          "You do not have permission to write to that share.",
        );
        return;
      }

      const externalFiles = getExternalDropFiles(e.dataTransfer);
      if (externalFiles.length > 0) {
        markExternalDropHandled(e.nativeEvent);
        uploadZoneRef.current?.uploadTo(targetRoot, targetPath, externalFiles);
        return;
      }

      const payload = getFileDragPayload(e.dataTransfer);
      if (!payload || payload.paths.length === 0) return;

      if (isSelfOrDescendantDrop(payload, targetRoot, targetPath)) {
        showErrorToast("Drop blocked", "Cannot move a folder into itself.");
        return;
      }

      if (payload.root === targetRoot) {
        void executeTransfer(
          payload.root,
          payload.paths,
          targetRoot,
          targetPath,
          "move",
        );
        return;
      }

      setPendingTransfer({
        sourceRoot: payload.root,
        paths: payload.paths,
        destRoot: targetRoot,
        dest: targetPath,
      });
    },
    [executeTransfer, resetCurrentDropTarget, showErrorToast, user?.roots],
  );

  const handleContextMenu = (e: React.MouseEvent, entry: FileEntry) => {
    e.preventDefault();
    e.stopPropagation();
    const entryPath = path ? `${path}/${entry.name}` : entry.name;
    useViewStore.getState().select(entryPath);
    setContextMenu({ x: e.clientX, y: e.clientY, entry, parentPath: path });
  };

  const handleContextMenuAtPath = (
    e: React.MouseEvent,
    entry: FileEntry,
    parentPath: string,
  ) => {
    e.preventDefault();
    e.stopPropagation();
    const entryPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;
    useViewStore.getState().select(entryPath);
    setContextMenu({ x: e.clientX, y: e.clientY, entry, parentPath });
  };

  const handleExtractArchive = async (entryPath: string, mode: ExtractMode) => {
    try {
      await api.extractArchive(root, entryPath, mode);
      useViewStore.getState().clearSelection();
      refreshListing();
    } catch (err) {
      showErrorDialog("Extraction failed", err);
    }
  };

  const getContextMenuItems = (
    entry: FileEntry,
    parentPath: string,
  ): ContextMenuItem[] => {
    const entryPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;
    const items: ContextMenuItem[] = [];

    if (entry.is_dir) {
      items.push({
        label: "Open",
        iconName: "folderOpen",
        onClick: () => navigateTo(entry),
      });
      items.push({
        label: "Download as ZIP",
        iconName: "download",
        onClick: () => {
          api
            .downloadZip(root, [entryPath])
            .catch((err) => showErrorDialog("ZIP download failed", err));
        },
      });
    } else {
      items.push({
        label: "Download",
        iconName: "download",
        onClick: () => window.open(api.downloadUrl(root, entryPath), "_blank"),
      });
    }

    if (
      !entry.is_dir &&
      caps.write &&
      serverCapabilities.archive_extraction &&
      isExtractableArchive(entry.name)
    ) {
      items.push({
        label: "",
        iconName: "file",
        onClick: () => {},
        separator: true,
      });
      items.push({
        label: "Extract here",
        iconName: "archive",
        onClick: () => {
          void handleExtractArchive(entryPath, "here");
        },
      });
      items.push({
        label: "Extract into Subfolder",
        iconName: "folder",
        onClick: () => {
          void handleExtractArchive(entryPath, "subfolder");
        },
      });
      items.push({
        label: "Extract and Remove Archive",
        iconName: "archive",
        onClick: () => {
          void handleExtractArchive(entryPath, "here_remove");
        },
      });
    }

    if (caps.write) {
      items.push({
        label: "Rename",
        iconName: "fileText",
        onClick: () => {
          setRenameTarget(entry);
          setShowRename(true);
        },
        separator: false,
      });
    }

    if (caps.share) {
      items.push({
        label: "Share",
        iconName: "share2",
        onClick: () => {
          setShareTarget({ path: entryPath, is_dir: entry.is_dir });
          setShowShare(true);
        },
      });
    }

    if (caps.write) {
      items.push({
        label: "",
        iconName: "file",
        onClick: () => {},
        separator: true,
      });

      items.push({
        label: "Delete",
        iconName: "alertTriangle",
        onClick: () => {
          useViewStore.getState().select(entryPath);
          setShowDeleteConfirm(true);
        },
        danger: true,
      });
    }

    return items;
  };

  // Keyboard shortcuts
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (isEditableShortcutTarget(e.target)) return;

      const selected = useViewStore.getState().selectedPaths;

      // Delete key
      if ((e.key === "Delete" || e.key === "Backspace") && caps.write) {
        if (selected.size > 0) {
          e.preventDefault();
          setShowDeleteConfirm(true);
        }
      }

      // F2 for rename
      if (e.key === "F2" && selected.size === 1 && caps.write) {
        e.preventDefault();
        const selectedPath = Array.from(selected)[0];
        const entry = listing?.entries.find((en) => {
          const entryPath = path ? `${path}/${en.name}` : en.name;
          return entryPath === selectedPath;
        });
        if (entry) {
          setRenameTarget(entry);
          setShowRename(true);
        }
      }
    },
    [caps.write, listing, path],
  );

  const selectedItems = Array.from(selectedPaths);
  const selectedCount = selectedItems.length;

  const [selectionDirSizes, setSelectionDirSizes] = useState<
    Record<string, number>
  >({});

  useEffect(() => {
    if (!listing || selectedCount === 0) {
      setSelectionDirSizes({});
      return;
    }
    const selectedDirPaths = selectedItems.filter((p) => {
      const name = p.split("/").pop() ?? "";
      return listing.entries.some((e) => e.is_dir && e.name === name && (path ? `${path}/${e.name}` : e.name) === p);
    });
    if (!root || selectedDirPaths.length === 0) {
      setSelectionDirSizes({});
      return;
    }
    let cancelled = false;
    api
      .folderSizes(root, selectedDirPaths)
      .then((result) => {
        if (!cancelled) setSelectionDirSizes(result.sizes);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [root, path, selectedItems, listing, selectedCount]);

  const selectionStats = useMemo(() => {
    if (selectedCount === 0 || !listing) return null;
    let totalSize = 0;
    let knownSize = true;
    for (const p of selectedItems) {
      const name = p.split("/").pop() ?? "";
      const entry = listing.entries.find(
        (e) => e.name === name && (path ? `${path}/${e.name}` : e.name) === p,
      );
      if (!entry) continue;
      if (entry.is_dir) {
        const ds = selectionDirSizes[p];
        if (ds != null) {
          totalSize += ds;
        } else {
          knownSize = false;
        }
      } else {
        totalSize += entry.size;
      }
    }
    return { totalSize, knownSize };
  }, [selectedCount, selectedItems, listing, path, selectionDirSizes]);

  const previewEntries = useMemo(() => {
    if (!previewTarget) return [];
    if (previewTarget.parentPath === path) return listing?.entries ?? [];
    return (
      queryClient.getQueryData<DirectoryListing>([
        "listing",
        root,
        previewTarget.parentPath,
      ])?.entries ?? []
    );
  }, [listing?.entries, path, previewTarget, queryClient, root]);
  const selectedDetails: FileDetailsSelection | null = useMemo(() => {
    if (viewMode === "columns" || selectedPaths.size !== 1 || !listing)
      return null;
    const selectedPath = Array.from(selectedPaths)[0];
    const entry = listing.entries.find((candidate) => {
      const candidatePath = path ? `${path}/${candidate.name}` : candidate.name;
      return candidatePath === selectedPath;
    });
    return entry ? { entry, parentPath: path, path: selectedPath } : null;
  }, [listing, path, selectedPaths, viewMode]);
  const hasReadme = Boolean(listing?.entries.some(isReadmeEntry));
  const readmeShown = viewMode !== "columns" && hasReadme && !readmeHidden;
  const canShowReadme = viewMode !== "columns" && hasReadme && !readmeShown;
  const deletePreviewItems = selectedItems.slice(0, 5).map((selectedPath) => {
    const name = selectedPath.split("/").filter(Boolean).pop();
    return { path: selectedPath, name: name || selectedPath };
  });
  const hiddenDeleteCount = Math.max(
    0,
    selectedCount - deletePreviewItems.length,
  );

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        background: "var(--color-bg)",
      }}
      onKeyDown={handleKeyDown}
      tabIndex={-1}
    >
      <TopBar user={user || null} />

      <div
        style={{ display: "flex", flex: 1, minHeight: 0, overflow: "hidden" }}
      >
        {/* Sidebar */}
        {sidebarOpen && viewMode !== "columns" && (
          <aside
            style={{
              position: "relative",
              width: sidebarWidth,
              minWidth: sidebarWidth,
              borderRight: "1px solid var(--color-border)",
              background: "var(--color-sidebar-bg)",
              overflowY: "auto",
              overflowX: "hidden",
              padding: "var(--space-2) 0",
            }}
          >
            {user && (
              <FolderTree
                roots={user.roots}
                activeRoot={root}
                activePath={path}
                onDropFiles={handleFileDrop}
                transferJobs={activeTransferJobs}
                customLinks={user.custom_links}
                onNavigate={(rootKey, folderPath) => {
                  navigate({
                    to: "/r/$root/$",
                    params: { root: rootKey, _splat: folderPath },
                  });
                }}
              />
            )}
            <div
              role="separator"
              aria-orientation="vertical"
              onPointerDown={startSidebarResize}
              style={{
                position: "absolute",
                top: 0,
                right: 0,
                bottom: 0,
                width: 6,
                cursor: "col-resize",
                zIndex: 3,
              }}
            />
          </aside>
        )}

        {/* Main content */}
        <UploadZone
          ref={uploadZoneRef}
          root={root}
          path={path}
          onUploadComplete={refreshListing}
          canUpload={caps.write}
        >
          <main
            style={{
              flex: 1,
              minHeight: 0,
              display: "flex",
              flexDirection: "column",
              overflow: "hidden",
            }}
          >
            {/* Toolbar */}
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
                padding: "var(--space-2) var(--space-4)",
                borderBottom: "1px solid var(--color-border)",
                background: "var(--color-bg)",
              }}
            >
              <Breadcrumb
                root={root}
                rootDisplayName={
                  user?.roots.find((r) => r.key === root)?.display_name || root
                }
                path={breadcrumbPath}
                onNavigate={navigateToPath}
              />

              <div style={{ flex: 1 }} />

              {/* Selection summary */}
              {selectionStats && (
                <div
                  style={{
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-muted)",
                    whiteSpace: "nowrap",
                  }}
                >
                  {selectedCount === 1 ? "1 item" : `${selectedCount} items`}
                  {" · "}
                  {selectionStats.knownSize
                    ? selectionStats.totalSize > 0
                      ? formatFileSize(selectionStats.totalSize)
                      : "0 B"
                    : selectionStats.totalSize > 0
                      ? `~${formatFileSize(selectionStats.totalSize)}`
                      : "Calculating…"}
                </div>
              )}

              {/* Action buttons */}
              {caps.write && (
                <>
                  <button
                    onClick={() => uploadZoneRef.current?.trigger()}
                    title="Upload files"
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "var(--space-1)",
                      padding: "var(--space-1) var(--space-2)",
                      border: "1px solid var(--color-border)",
                      borderRadius: "var(--radius-md)",
                      background: "transparent",
                      color: "var(--color-fg-muted)",
                      cursor: "pointer",
                      fontSize: "var(--text-xs)",
                      fontWeight: 500,
                      transition: "all var(--duration-fast) var(--ease-out)",
                    }}
                    onMouseOver={(e) => {
                      e.currentTarget.style.background =
                        "var(--color-bg-muted)";
                      e.currentTarget.style.color = "var(--color-fg)";
                    }}
                    onMouseOut={(e) => {
                      e.currentTarget.style.background = "transparent";
                      e.currentTarget.style.color = "var(--color-fg-muted)";
                    }}
                  >
                    <Icon name="upload" size={14} />
                    Upload
                  </button>

                  <button
                    onClick={() => setShowCreateFolder(true)}
                    title="New folder"
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "var(--space-1)",
                      padding: "var(--space-1) var(--space-2)",
                      border: "1px solid var(--color-border)",
                      borderRadius: "var(--radius-md)",
                      background: "transparent",
                      color: "var(--color-fg-muted)",
                      cursor: "pointer",
                      fontSize: "var(--text-xs)",
                      fontWeight: 500,
                      transition: "all var(--duration-fast) var(--ease-out)",
                    }}
                    onMouseOver={(e) => {
                      e.currentTarget.style.background =
                        "var(--color-bg-muted)";
                      e.currentTarget.style.color = "var(--color-fg)";
                    }}
                    onMouseOut={(e) => {
                      e.currentTarget.style.background = "transparent";
                      e.currentTarget.style.color = "var(--color-fg-muted)";
                    }}
                  >
                    <Icon name="folder" size={14} />
                    New Folder
                  </button>
                </>
              )}

              {/* Share current folder (shown when inside a subfolder) */}
              {path && caps.share && (
                <button
                  onClick={() => {
                    setShareTarget({ path, is_dir: true });
                    setShowShare(true);
                  }}
                  title="Share this folder"
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-1)",
                    padding: "var(--space-1) var(--space-2)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    background: "transparent",
                    color: "var(--color-fg-muted)",
                    cursor: "pointer",
                    fontSize: "var(--text-xs)",
                    fontWeight: 500,
                    transition: "all var(--duration-fast) var(--ease-out)",
                  }}
                  onMouseOver={(e) => {
                    e.currentTarget.style.background = "var(--color-bg-muted)";
                    e.currentTarget.style.color = "var(--color-fg)";
                  }}
                  onMouseOut={(e) => {
                    e.currentTarget.style.background = "transparent";
                    e.currentTarget.style.color = "var(--color-fg-muted)";
                  }}
                >
                  <Icon name="share2" size={14} />
                  Share
                </button>
              )}

              {/* Delete button (shown when selection) */}
              {selectedCount > 0 && (
                <button
                  onClick={() => setShowDeleteConfirm(true)}
                  title={`Delete ${selectedCount} item(s)`}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-1)",
                    padding: "var(--space-1) var(--space-2)",
                    border: "1px solid var(--color-border)",
                    borderRadius: "var(--radius-md)",
                    background: "transparent",
                    color: "var(--color-fg-muted)",
                    cursor: "pointer",
                    fontSize: "var(--text-xs)",
                    fontWeight: 500,
                    transition:
                      "border-color var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out)",
                  }}
                  onMouseOver={(e) => {
                    e.currentTarget.style.borderColor = "var(--color-danger)";
                    e.currentTarget.style.color = "var(--color-danger)";
                  }}
                  onMouseOut={(e) => {
                    e.currentTarget.style.borderColor = "var(--color-border)";
                    e.currentTarget.style.color = "var(--color-fg-muted)";
                  }}
                >
                  <Icon name="alertTriangle" size={14} />
                  Delete ({selectedCount})
                </button>
              )}

              {/* Separator */}
              <div
                style={{
                  width: 1,
                  height: 20,
                  background: "var(--color-border)",
                  margin: "0 var(--space-1)",
                }}
              />

              {/* View mode toggle */}
              <div
                style={{
                  display: "flex",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-border)",
                  overflow: "hidden",
                }}
              >
                <button
                  onClick={() => switchViewMode("grid")}
                  style={{
                    padding: "var(--space-1) var(--space-2)",
                    background:
                      viewMode === "grid"
                        ? "var(--color-accent-muted)"
                        : "transparent",
                    border: "none",
                    cursor: "pointer",
                    color:
                      viewMode === "grid"
                        ? "var(--color-accent)"
                        : "var(--color-fg-muted)",
                    fontSize: "var(--text-sm)",
                    transition: "all var(--duration-fast) var(--ease-out)",
                    display: "flex",
                    alignItems: "center",
                  }}
                  title="Grid view"
                >
                  <Icon name="grid" size={16} />
                </button>
                <button
                  onClick={() => switchViewMode("list")}
                  style={{
                    padding: "var(--space-1) var(--space-2)",
                    background:
                      viewMode === "list"
                        ? "var(--color-accent-muted)"
                        : "transparent",
                    border: "none",
                    borderLeft: "1px solid var(--color-border)",
                    cursor: "pointer",
                    color:
                      viewMode === "list"
                        ? "var(--color-accent)"
                        : "var(--color-fg-muted)",
                    fontSize: "var(--text-sm)",
                    transition: "all var(--duration-fast) var(--ease-out)",
                    display: "flex",
                    alignItems: "center",
                  }}
                  title="List view"
                >
                  <Icon name="list" size={16} />
                </button>
                <button
                  onClick={() => switchViewMode("columns")}
                  style={{
                    padding: "var(--space-1) var(--space-2)",
                    background:
                      viewMode === "columns"
                        ? "var(--color-accent-muted)"
                        : "transparent",
                    border: "none",
                    borderLeft: "1px solid var(--color-border)",
                    cursor: "pointer",
                    color:
                      viewMode === "columns"
                        ? "var(--color-accent)"
                        : "var(--color-fg-muted)",
                    fontSize: "var(--text-sm)",
                    transition: "all var(--duration-fast) var(--ease-out)",
                    display: "flex",
                    alignItems: "center",
                  }}
                  title="Column view"
                >
                  <Icon name="columns" size={16} />
                </button>
              </div>
            </div>

            {/* File listing */}
            <div style={{ position: "relative", flex: 1, minHeight: 0 }}>
              <div
                ref={listingScrollRef}
                style={{
                  height: "100%",
                  minHeight: 0,
                  overflow: viewMode === "columns" ? "hidden" : "auto",
                  padding: viewMode === "columns" ? 0 : "var(--space-4)",
                  outline:
                    dropTargetActive || isDemoCurrentFolderDropTarget
                      ? "2px dashed var(--color-accent)"
                      : "none",
                  outlineOffset: -8,
                  background:
                    dropTargetActive || isDemoCurrentFolderDropTarget
                      ? "var(--color-accent-muted)"
                      : "transparent",
                }}
                className={
                  viewMode === "columns"
                    ? undefined
                    : "flex flex-col-reverse lg:flex-row gap-6 items-start"
                }
                onDragEnter={(e) => {
                  if (
                    !caps.write ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  e.preventDefault();
                  setDropTargetActive(true);
                }}
                onDragOver={(e) => {
                  if (
                    !caps.write ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = hasExternalFileDrag(
                    e.dataTransfer,
                  )
                    ? "copy"
                    : "move";
                }}
                onDragLeave={(e) => {
                  if (e.currentTarget.contains(e.relatedTarget as Node | null))
                    return;
                  resetCurrentDropTarget();
                }}
                onDrop={(e) => {
                  if (
                    !caps.write ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  resetCurrentDropTarget();
                  handleFileDrop(root, path, e);
                }}
              >
                <div
                  className={
                    viewMode === "columns"
                      ? "flex-1 min-w-0 w-full flex"
                      : "flex-1 min-w-0 w-full"
                  }
                  style={
                    viewMode === "columns"
                      ? { minHeight: 0, height: "100%" }
                      : {
                          minHeight: "100%",
                          display: "flex",
                          flexDirection: "column",
                        }
                  }
                >
                  {viewMode !== "columns" && isLoading && (
                    <div
                      style={{
                        display: "grid",
                        gridTemplateColumns:
                          "repeat(auto-fill, minmax(160px, 1fr))",
                        gap: "var(--space-4)",
                      }}
                    >
                      {Array.from({ length: 12 }).map((_, i) => (
                        <div
                          key={i}
                          className="shimmer"
                          style={{
                            height: 140,
                            borderRadius: "var(--radius-lg)",
                          }}
                        />
                      ))}
                    </div>
                  )}

                  {viewMode !== "columns" && error && (
                    <EmptyState
                      iconName="alertTriangle"
                      title="Failed to load"
                      description={
                        error instanceof Error ? error.message : "Unknown error"
                      }
                    />
                  )}

                  {viewMode !== "columns" &&
                    listing &&
                    listing.entries.length === 0 &&
                    currentFolderTransferJobs.length === 0 && (
                      <EmptyState
                        iconName="folderOpen"
                        title="This folder is empty"
                        description="Drop files here or create a new folder to get started."
                      />
                    )}

                  {viewMode === "columns" && user && (
                    <ColumnBrowser
                      roots={user.roots}
                      activeRoot={root}
                      activePath={path}
                      currentListing={listing}
                      isLoading={isLoading}
                      error={error}
                      canDrop={caps.write}
                      onNavigateRoot={navigateToRoot}
                      onNavigatePath={navigateToPath}
                      onOpenEntry={openEntryAtPath}
                      onPreviewEntry={togglePreviewAtPath}
                      onContextMenu={handleContextMenuAtPath}
                      onDropFiles={handleFileDrop}
                      transferJobs={activeTransferJobs}
                      onDisplayPathChange={handleColumnDisplayPathChange}
                      onActiveFolderPathChange={
                        handleColumnActiveFolderPathChange
                      }
                    />
                  )}

                  {viewMode === "grid" &&
                    listing &&
                    (listing.entries.length > 0 ||
                      currentFolderTransferJobs.length > 0) && (
                      <FileGrid
                        entries={listing.entries}
                        onOpen={navigateTo}
                        root={root}
                        path={path}
                        scrollParentRef={listingScrollRef}
                        onContextMenu={handleContextMenu}
                        onDropFiles={handleFileDrop}
                        transferJobs={activeTransferJobs}
                      />
                    )}

                  {viewMode === "list" &&
                    listing &&
                    (listing.entries.length > 0 ||
                      currentFolderTransferJobs.length > 0) && (
                      <FileList
                        entries={listing.entries}
                        onOpen={navigateTo}
                        root={root}
                        path={path}
                        scrollParentRef={listingScrollRef}
                        onContextMenu={handleContextMenu}
                        onDropFiles={handleFileDrop}
                        transferJobs={activeTransferJobs}
                      />
                    )}

                  {viewMode !== "columns" && listing && currentRoot?.usage && (
                    <FreeSpaceFooter
                      availableBytes={currentRoot.usage.available_bytes}
                      canShowReadme={canShowReadme}
                      onShowReadme={() => {
                        setReadmeHidden(false);
                        useViewStore.getState().clearSelection();
                      }}
                    />
                  )}
                </div>

                {readmeShown && listing && (
                  <DirectoryReadme
                    entries={listing.entries}
                    root={root}
                    path={path}
                    onClose={() => setReadmeHidden(true)}
                  />
                )}
              </div>

              {viewMode !== "columns" && selectedDetails && (
                <div
                  style={{
                    position: "absolute",
                    top: "var(--space-4)",
                    right: "var(--space-4)",
                    bottom: "var(--space-4)",
                    width: "min(400px, calc(100% - var(--space-8)))",
                    zIndex: 15,
                    borderRadius: "var(--radius-lg)",
                    boxShadow: "var(--shadow-lg)",
                  }}
                >
                  <FileDetailsPane
                    root={root}
                    selected={selectedDetails}
                    width="100%"
                    onPreview={togglePreviewAtPath}
                    onClose={() => useViewStore.getState().clearSelection()}
                  />
                </div>
              )}
            </div>
          </main>
        </UploadZone>
      </div>

      {/* Dialogs */}
      <CreateFolderDialog
        open={showCreateFolder}
        onClose={() => setShowCreateFolder(false)}
        onCreate={handleCreateFolder}
      />

      <RenameDialog
        open={showRename}
        currentName={renameTarget?.name || ""}
        onClose={() => {
          setShowRename(false);
          setRenameTarget(null);
        }}
        onRename={handleRename}
      />

      {/* Delete confirmation */}
      {showDeleteConfirm && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            background: "rgba(0,0,0,0.4)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            zIndex: 100,
          }}
          className="fade-in"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowDeleteConfirm(false);
          }}
        >
          <div
            style={{
              background: "var(--color-bg)",
              borderRadius: "var(--radius-xl)",
              boxShadow: "var(--shadow-xl)",
              padding: "var(--space-6)",
              width: 400,
              maxWidth: "90vw",
            }}
            className="slide-in"
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-3)",
                marginBottom: "var(--space-3)",
              }}
            >
              <Icon
                name="alertTriangle"
                size={20}
                color="var(--color-danger)"
              />
              <h2
                style={{
                  margin: 0,
                  fontSize: "var(--text-lg)",
                  fontWeight: 600,
                }}
              >
                Delete
              </h2>
            </div>
            <p
              style={{
                fontSize: "var(--text-sm)",
                color: "var(--color-fg-muted)",
                margin: "0 0 var(--space-3)",
              }}
            >
              Are you sure you want to delete {selectedCount} item(s)? This
              action cannot be undone.
            </p>
            <ul
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--space-1)",
                margin: "0 0 var(--space-4)",
                padding: "var(--space-3)",
                listStyle: "none",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-md)",
                background: "var(--color-bg-muted)",
                fontSize: "var(--text-sm)",
                color: "var(--color-fg)",
              }}
            >
              {deletePreviewItems.map((item) => (
                <li
                  key={item.path}
                  style={{
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                  title={item.path}
                >
                  {item.name}
                </li>
              ))}
              {hiddenDeleteCount > 0 && (
                <li style={{ color: "var(--color-fg-muted)" }}>
                  and {hiddenDeleteCount} more
                </li>
              )}
            </ul>
            <div
              style={{
                display: "flex",
                justifyContent: "flex-end",
                gap: "var(--space-2)",
              }}
            >
              <button
                onClick={() => setShowDeleteConfirm(false)}
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
                Cancel
              </button>
              <button
                onClick={handleDelete}
                style={{
                  padding: "var(--space-2) var(--space-4)",
                  border: "none",
                  borderRadius: "var(--radius-md)",
                  background: "var(--color-danger)",
                  color: "#fff",
                  cursor: "pointer",
                  fontWeight: 500,
                  fontSize: "var(--text-sm)",
                }}
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}

      {pendingTransfer && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            background: "rgba(0,0,0,0.4)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            zIndex: 100,
          }}
          className="fade-in"
          onClick={(e) => {
            if (e.target === e.currentTarget) setPendingTransfer(null);
          }}
        >
          <div
            style={{
              width: 360,
              background: "var(--color-bg)",
              borderRadius: "var(--radius-lg)",
              boxShadow: "var(--shadow-lg)",
              padding: "var(--space-5)",
            }}
          >
            <h3
              style={{
                margin: 0,
                marginBottom: "var(--space-2)",
                fontSize: "var(--text-lg)",
              }}
            >
              Move or copy?
            </h3>
            <p
              style={{
                margin: 0,
                marginBottom: "var(--space-4)",
                color: "var(--color-fg-muted)",
                fontSize: "var(--text-sm)",
              }}
            >
              Drop {pendingTransfer.paths.length} item(s) into a different
              share.
            </p>
            <div
              style={{
                display: "flex",
                gap: "var(--space-2)",
                justifyContent: "flex-end",
              }}
            >
              <button
                onClick={() => setPendingTransfer(null)}
                style={{
                  padding: "var(--space-2) var(--space-3)",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-border)",
                  background: "transparent",
                  color: "var(--color-fg-muted)",
                  cursor: "pointer",
                }}
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  const transfer = pendingTransfer;
                  setPendingTransfer(null);
                  void executeTransfer(
                    transfer.sourceRoot,
                    transfer.paths,
                    transfer.destRoot,
                    transfer.dest,
                    "copy",
                  );
                }}
                style={{
                  padding: "var(--space-2) var(--space-3)",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-border)",
                  background: "transparent",
                  color: "var(--color-fg)",
                  cursor: "pointer",
                }}
              >
                Copy
              </button>
              <button
                onClick={() => {
                  const transfer = pendingTransfer;
                  setPendingTransfer(null);
                  void executeTransfer(
                    transfer.sourceRoot,
                    transfer.paths,
                    transfer.destRoot,
                    transfer.dest,
                    "move",
                  );
                }}
                style={{
                  padding: "var(--space-2) var(--space-3)",
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-accent)",
                  background: "var(--color-accent)",
                  color: "white",
                  cursor: "pointer",
                }}
              >
                Move
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Context menu */}
      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          items={getContextMenuItems(contextMenu.entry, contextMenu.parentPath)}
          onClose={() => setContextMenu(null)}
        />
      )}

      <ErrorToasts
        toasts={errorToasts}
        onDismiss={(id) =>
          setErrorToasts((current) =>
            current.filter((toast) => toast.id !== id),
          )
        }
      />

      <OperationProgressToasts deleteJobs={deleteJobs} />

      <ErrorDialog
        error={blockingError}
        onClose={() => setBlockingError(null)}
      />

      {/* Share dialog */}
      <ShareDialog
        open={showShare}
        root={root}
        path={shareTarget?.path ?? ""}
        isDirectory={shareTarget?.is_dir ?? false}
        onClose={() => {
          setShowShare(false);
          setShareTarget(null);
        }}
      />

      {/* Preview pane */}
      {previewTarget && (
        <PreviewPane
          entry={previewTarget.entry}
          root={root}
          path={previewTarget.parentPath}
          entries={previewEntries}
          mediaPreviewTranscodingEnabled={
            serverCapabilities.media_preview_transcoding
          }
          onClose={() => setPreviewTarget(null)}
          onNavigate={(entry) => {
            const nextPath = previewTarget.parentPath
              ? `${previewTarget.parentPath}/${entry.name}`
              : entry.name;
            useViewStore.getState().select(nextPath);
            setPreviewTarget({ entry, parentPath: previewTarget.parentPath });
          }}
        />
      )}
    </div>
  );
}

function isExtractableArchive(name: string) {
  const lower = name.toLowerCase();
  return (
    lower.endsWith(".zip") ||
    lower.endsWith(".rar") ||
    lower.endsWith(".7z") ||
    lower.endsWith(".7z.001") ||
    lower.endsWith(".tar") ||
    lower.endsWith(".tar.gz") ||
    lower.endsWith(".tgz") ||
    lower.endsWith(".tar.bz2") ||
    lower.endsWith(".tbz") ||
    lower.endsWith(".tbz2") ||
    lower.endsWith(".bz") ||
    lower.endsWith(".bz2") ||
    /\.part\d+\.rar$/.test(lower) ||
    /\.r\d\d$/.test(lower)
  );
}

function removeDeletedPathsFromListings(
  queryClient: ReturnType<typeof useQueryClient>,
  root: string,
  paths: string[],
) {
  const deletedPaths = new Set(paths);
  queryClient.setQueriesData<DirectoryListing>(
    { queryKey: ["listing", root] },
    (listing) => {
      if (!listing) return listing;
      const entries = listing.entries.filter((entry) => {
        const fullPath = listing.path
          ? `${listing.path}/${entry.name}`
          : entry.name;
        return !deletedPaths.has(fullPath);
      });
      return entries.length === listing.entries.length
        ? listing
        : { ...listing, entries };
    },
  );
}

function OperationProgressToasts({
  deleteJobs,
}: {
  deleteJobs: DeleteJobNotice[];
}) {
  if (deleteJobs.length === 0) return null;

  return (
    <div
      aria-live="polite"
      aria-label="File operation progress"
      style={{
        position: "fixed",
        right: "var(--space-4)",
        bottom: "var(--space-4)",
        zIndex: 90,
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-2)",
        width: "min(360px, calc(100vw - 32px))",
        pointerEvents: "none",
      }}
    >
      {deleteJobs.map((job) => (
        <OperationProgressToast
          key={`delete-${job.id}`}
          iconName="alertTriangle"
          title={`Deleting ${formatCount(job.count, "file")}`}
          detail="Removing from this share"
          indeterminate
        />
      ))}
    </div>
  );
}

function OperationProgressToast({
  iconName,
  title,
  detail,
  indeterminate = false,
}: {
  iconName: React.ComponentProps<typeof Icon>["name"];
  title: string;
  detail: string;
  indeterminate?: boolean;
}) {
  return (
    <div
      className="fade-in"
      style={{
        display: "flex",
        gap: "var(--space-3)",
        padding: "var(--space-3)",
        border: "1px solid var(--color-border)",
        borderRadius: "var(--radius-md)",
        background: "var(--color-bg)",
        color: "var(--color-fg)",
        boxShadow: "var(--shadow-lg)",
      }}
    >
      <Icon name={iconName} size={16} color="var(--color-accent)" />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: "var(--space-2)",
            marginBottom: "var(--space-1)",
          }}
        >
          <span
            style={{
              minWidth: 0,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontSize: "var(--text-sm)",
              fontWeight: 600,
            }}
          >
            {title}
          </span>
        </div>
        <div
          style={{
            height: 4,
            borderRadius: 2,
            background: "var(--color-border)",
            overflow: "hidden",
            marginBottom: "var(--space-1)",
          }}
        >
          <div
            className={
              indeterminate ? "operation-progress-indeterminate" : undefined
            }
            style={{
              height: "100%",
              width: indeterminate ? "42%" : "100%",
              borderRadius: 2,
              background: "var(--color-accent)",
            }}
          />
        </div>
        <div
          style={{
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
            color: "var(--color-fg-subtle)",
            fontSize: "var(--text-xs)",
          }}
        >
          {detail}
        </div>
      </div>
    </div>
  );
}

function formatCount(count: number, noun: string) {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

function isReadmeEntry(entry: FileEntry) {
  return (
    !entry.is_dir &&
    ["README.md", "Readme.md", "readme.md"].includes(entry.name)
  );
}

function FreeSpaceFooter({
  availableBytes,
  canShowReadme,
  onShowReadme,
}: {
  availableBytes: number;
  canShowReadme: boolean;
  onShowReadme: () => void;
}) {
  return (
    <div
      className="tabular-nums"
      style={{
        marginTop: "auto",
        paddingTop: "var(--space-4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "flex-end",
        gap: "var(--space-3)",
        color: "var(--color-fg-subtle)",
        fontSize: "var(--text-xs)",
        fontWeight: 500,
      }}
    >
      <span>{formatFileSize(availableBytes)} remaining</span>
      {canShowReadme && (
        <button
          type="button"
          onClick={onShowReadme}
          style={{
            border: "none",
            background: "transparent",
            padding: 0,
            color: "var(--color-accent)",
            cursor: "pointer",
            font: "inherit",
            fontWeight: 600,
          }}
        >
          show readme
        </button>
      )}
    </div>
  );
}
