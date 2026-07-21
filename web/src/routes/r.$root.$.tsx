import {
  createFileRoute,
  useNavigate,
  useParams,
} from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
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
import { FileIcon, Icon } from "../components/Icon";
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
import { ThumbnailImage } from "../components/ThumbnailImage";
import { TransferProgressIndicator } from "../components/TransferProgressIndicator";
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
import { formatFileSize, formatModifiedDate, getFileIcon } from "../lib/icons";
import type { DirectoryListing, TransferJob } from "../api/client";

export const Route = createFileRoute("/r/$root/$")({
  component: FileBrowser,
});

interface DeleteJobNotice {
  id: number;
  count: number;
}

type MobileDrawerState = "closed" | "half" | "full";

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
    sortField,
    sortDirection,
    setSidebarWidth,
    setViewMode,
  } = useViewStore();
  const uploadZoneRef = useRef<UploadZoneHandle>(null);
  const mobileUploadZoneRef = useRef<UploadZoneHandle>(null);
  const mobileListingScrollRef = useRef<HTMLDivElement>(null);
  const listingScrollRef = useRef<HTMLDivElement>(null);
  const errorIdRef = useRef(0);
  const deleteJobIdRef = useRef(0);
  const lastDirectoryRef = useRef({ root, path });

  // Dialogs
  const [showCreateFolder, setShowCreateFolder] = useState(false);
  const [showRename, setShowRename] = useState(false);
  const [renameTarget, setRenameTarget] = useState<FileEntry | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showShare, setShowShare] = useState(false);
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const [mobileCreateMenuOpen, setMobileCreateMenuOpen] = useState(false);
  const [mobileSortMenuOpen, setMobileSortMenuOpen] = useState(false);
  const [mobileDrawerState, setMobileDrawerState] =
    useState<MobileDrawerState>("closed");
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
  // can't accidentally be targeted by Delete/F2 keyboard shortcuts. Column
  // view preserves selection while committing focused folders into the URL.
  useEffect(() => {
    const previous = lastDirectoryRef.current;
    lastDirectoryRef.current = { root, path };
    if (viewMode === "columns" && previous.root === root) return;
    useViewStore.getState().clearSelection();
  }, [path, root, viewMode]);

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
    (targetPath: string, options?: { replace?: boolean }) => {
      navigate({
        to: "/r/$root/$",
        params: { root, _splat: targetPath },
        replace: options?.replace,
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
      if (viewMode === "columns" && activeFolderPath !== path) {
        navigateToPath(activeFolderPath, { replace: true });
      }
    },
    [navigateToPath, path, viewMode],
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

  const setSortOption = useCallback(
    (field: "name" | "size" | "modified_at", direction: "asc" | "desc") => {
      const state = useViewStore.getState();
      if (state.sortField !== field) state.setSortField(field);
      if (useViewStore.getState().sortDirection !== direction) {
        state.toggleSortDirection();
      }
    },
    [],
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

  // @tour file-transfers:30 Copy or move is a UI decision
  // Before this runs, `handleFileDrop` refuses a folder dropped into itself, then branches:
  // a same-root drop is unambiguously a move and calls straight through, while a cross-root
  // drop stores a `pendingTransfer` and renders a modal whose Copy and Move buttons each
  // call here.
  //
  // `executeTransfer` itself is deliberately thin, and notably invalidates *no* query cache
  // — the request only creates a job, so there is nothing to re-read yet.

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

  // @tour file-transfers:20 The drop handler
  // The single funnel for every drop target in the route — the list, the grid, the column
  // browser and the tree all pass it as `onDropFiles`.
  //
  // It resets the drop highlight, does a client-side permission pre-check against
  // `user?.roots` and bails with a toast when the target root is not writable, and branches
  // external OS files off to the upload path first. Only in-app drags reach the transfer
  // logic.

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
        iconName: "trash",
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
  const selectedItemsKey = selectedItems.join("\n");

  useEffect(() => {
    setMobileDrawerState("closed");
  }, [selectedItemsKey]);

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
      return listing.entries.some(
        (e) =>
          e.is_dir &&
          e.name === name &&
          (path ? `${path}/${e.name}` : e.name) === p,
      );
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
    if (selectedPaths.size !== 1 || !listing) return null;
    const selectedPath = Array.from(selectedPaths)[0];
    const entry = listing.entries.find((candidate) => {
      const candidatePath = path ? `${path}/${candidate.name}` : candidate.name;
      return candidatePath === selectedPath;
    });
    return entry ? { entry, parentPath: path, path: selectedPath } : null;
  }, [listing, path, selectedPaths]);
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
      <TopBar
        user={user || null}
        onMobileSidebarToggle={() => setMobileSidebarOpen((open) => !open)}
      />

      <div
        className="mobile-file-browser"
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "hidden",
          display: "none",
        }}
      >
        <UploadZone
          ref={mobileUploadZoneRef}
          root={root}
          path={path}
          onUploadComplete={refreshListing}
          canUpload={caps.write}
        >
          <main
            style={{
              height: "100%",
              display: "flex",
              flexDirection: "column",
              minHeight: 0,
              background: "var(--color-bg)",
            }}
          >
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "var(--space-2)",
                padding: "var(--space-2)",
                borderBottom: "1px solid var(--color-border)",
                background: "var(--color-bg)",
              }}
            >
              {path && (
                <MobileIconButton
                  iconName="arrowLeft"
                  label="Back"
                  onClick={() => {
                    const parent = path.split("/").slice(0, -1).join("/");
                    navigateToPath(parent);
                  }}
                />
              )}

              <MobilePathBar
                rootDisplayName={currentRoot?.display_name || root}
                path={path}
                onNavigate={navigateToPath}
              />

              <span
                style={{
                  color: "var(--color-fg-subtle)",
                  fontSize: "var(--text-xs)",
                  whiteSpace: "nowrap",
                }}
              >
                {listing
                  ? formatCount(listing.entries.length, "item")
                  : isLoading
                    ? "Loading"
                    : ""}
              </span>

              {caps.write && (
                <MobileMenuButton
                  iconName="plus"
                  label="Create"
                  open={mobileCreateMenuOpen}
                  onToggle={() => {
                    setMobileCreateMenuOpen((open) => !open);
                    setMobileSortMenuOpen(false);
                  }}
                  items={[
                    {
                      iconName: "upload",
                      label: "Upload",
                      onClick: () => mobileUploadZoneRef.current?.trigger(),
                    },
                    {
                      iconName: "folder",
                      label: "New folder",
                      onClick: () => setShowCreateFolder(true),
                    },
                  ]}
                />
              )}

              <MobileSortMenu
                open={mobileSortMenuOpen}
                viewMode={viewMode === "grid" ? "grid" : "list"}
                sortField={sortField}
                sortDirection={sortDirection}
                onToggle={() => {
                  setMobileSortMenuOpen((open) => !open);
                  setMobileCreateMenuOpen(false);
                }}
                onSelect={(field, direction) => {
                  setSortOption(field, direction);
                  setMobileSortMenuOpen(false);
                }}
                onViewModeChange={(mode) => {
                  switchViewMode(mode);
                  setMobileSortMenuOpen(false);
                }}
              />
            </div>

            <div
              ref={mobileListingScrollRef}
              style={{
                position: "relative",
                flex: 1,
                minHeight: 0,
                overflowY: "auto",
                padding: `var(--space-2) var(--space-2) ${mobileListBottomPadding(
                  selectedCount,
                  mobileDrawerState,
                )}`,
                outline:
                  dropTargetActive || isDemoCurrentFolderDropTarget
                    ? "2px dashed var(--color-accent)"
                    : "none",
                outlineOffset: -6,
                background:
                  dropTargetActive || isDemoCurrentFolderDropTarget
                    ? "var(--color-accent-muted)"
                    : "transparent",
              }}
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
                e.dataTransfer.dropEffect = hasExternalFileDrag(e.dataTransfer)
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
              {isLoading && <MobileLoadingList />}
              {error && (
                <EmptyState
                  iconName="alertTriangle"
                  title="Failed to load"
                  description={
                    error instanceof Error ? error.message : "Unknown error"
                  }
                />
              )}
              {listing &&
                listing.entries.length === 0 &&
                currentFolderTransferJobs.length === 0 && (
                  <EmptyState
                    iconName="folderOpen"
                    title="This folder is empty"
                    description="Upload files or create a folder to get started."
                  />
                )}
              {listing && (
                <MobileFileList
                  viewMode={viewMode === "grid" ? "grid" : "list"}
                  entries={listing.entries}
                  root={root}
                  path={path}
                  selectedPaths={selectedPaths}
                  transferJobs={activeTransferJobs}
                  scrollParentRef={mobileListingScrollRef}
                  onOpen={navigateTo}
                  onSelect={(entryPath) =>
                    useViewStore.getState().select(entryPath)
                  }
                  onToggleSelect={(entryPath) =>
                    useViewStore.getState().toggleSelect(entryPath)
                  }
                  onShowActions={(entry, x, y) => {
                    const entryPath = path
                      ? `${path}/${entry.name}`
                      : entry.name;
                    useViewStore.getState().select(entryPath);
                    setContextMenu({ x, y, entry, parentPath: path });
                  }}
                />
              )}
              {listing && currentRoot?.usage && (
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

            {selectedCount > 0 && (
              <MobileSelectionDrawer
                root={root}
                rootDisplayName={currentRoot?.display_name || root}
                selected={selectedDetails}
                selectedCount={selectedCount}
                state={mobileDrawerState}
                onStateChange={setMobileDrawerState}
                selectionStats={selectionStats}
                canWrite={caps.write}
                canShare={caps.share && Boolean(selectedDetails)}
                onClear={() => useViewStore.getState().clearSelection()}
                onShare={() => {
                  if (!selectedDetails) return;
                  setShareTarget({
                    path: selectedDetails.path,
                    is_dir: selectedDetails.entry.is_dir,
                  });
                  setShowShare(true);
                }}
                onPreview={(entry, parentPath) =>
                  setPreviewTarget({ entry, parentPath })
                }
                onDelete={() => setShowDeleteConfirm(true)}
              />
            )}
          </main>
        </UploadZone>

        {mobileSidebarOpen && user && (
          <MobileSidebarDrawer
            roots={user.roots}
            activeRoot={root}
            activePath={path}
            customLinks={user.custom_links}
            transferJobs={activeTransferJobs}
            onClose={() => setMobileSidebarOpen(false)}
            onDropFiles={handleFileDrop}
            onNavigate={(rootKey, folderPath) => {
              setMobileSidebarOpen(false);
              navigate({
                to: "/r/$root/$",
                params: { root: rootKey, _splat: folderPath },
              });
            }}
          />
        )}
      </div>

      <div
        className="desktop-file-browser"
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
                  <Icon name="trash" size={14} />
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
              <Icon name="trash" size={20} color="var(--color-danger)" />
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

function MobileSidebarDrawer({
  roots,
  activeRoot,
  activePath,
  customLinks,
  transferJobs,
  onNavigate,
  onDropFiles,
  onClose,
}: {
  roots: React.ComponentProps<typeof FolderTree>["roots"];
  activeRoot: string;
  activePath: string;
  customLinks: React.ComponentProps<typeof FolderTree>["customLinks"];
  transferJobs: TransferJob[];
  onNavigate: (rootKey: string, folderPath: string) => void;
  onDropFiles: React.ComponentProps<typeof FolderTree>["onDropFiles"];
  onClose: () => void;
}) {
  return (
    <div
      className="fade-in"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 70,
        background: "rgba(0,0,0,0.42)",
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <aside
        aria-label="Shares"
        style={{
          width: "min(84vw, 320px)",
          height: "100%",
          padding: "var(--space-2) 0",
          borderRight: "1px solid var(--color-border)",
          background: "var(--color-sidebar-bg)",
          boxShadow: "var(--shadow-lg)",
          overflowY: "auto",
        }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            padding: "0 var(--space-2) var(--space-1)",
          }}
        >
          <MobileIconButton iconName="x" label="Close" onClick={onClose} />
        </div>
        <FolderTree
          roots={roots}
          activeRoot={activeRoot}
          activePath={activePath}
          onDropFiles={onDropFiles}
          transferJobs={transferJobs}
          customLinks={customLinks}
          onNavigate={onNavigate}
        />
      </aside>
    </div>
  );
}

function MobilePathBar({
  rootDisplayName,
  path,
  onNavigate,
}: {
  rootDisplayName: string;
  path: string;
  onNavigate: (path: string) => void;
}) {
  const segments = path ? path.split("/").filter(Boolean) : [];

  return (
    <div
      style={{
        display: "flex",
        gap: "var(--space-1)",
        flex: 1,
        minWidth: 0,
        overflowX: "auto",
        paddingBottom: 2,
      }}
      aria-label="Current path"
    >
      <button
        type="button"
        onClick={() => onNavigate("")}
        style={mobilePathButtonStyle(!path)}
      >
        {rootDisplayName}
      </button>
      {segments.map((segment, index) => {
        const segmentPath = segments.slice(0, index + 1).join("/");
        const isLast = index === segments.length - 1;
        return (
          <button
            key={segmentPath}
            type="button"
            onClick={() => onNavigate(segmentPath)}
            style={mobilePathButtonStyle(isLast)}
          >
            {segment}
          </button>
        );
      })}
    </div>
  );
}

function mobilePathButtonStyle(active: boolean): React.CSSProperties {
  return {
    maxWidth: 180,
    flexShrink: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    minHeight: 32,
    border: `1px solid ${active ? "var(--color-accent)" : "var(--color-border)"}`,
    borderRadius: "var(--radius-md)",
    background: active ? "var(--color-accent-muted)" : "transparent",
    color: active ? "var(--color-accent)" : "var(--color-fg-muted)",
    padding: "0 var(--space-2)",
    fontSize: "var(--text-sm)",
    fontWeight: active ? 600 : 500,
  };
}

function MobileMenuButton({
  iconName,
  label,
  open,
  onToggle,
  items,
}: {
  iconName: React.ComponentProps<typeof Icon>["name"];
  label: string;
  open: boolean;
  onToggle: () => void;
  items: Array<{
    iconName: React.ComponentProps<typeof Icon>["name"];
    label: string;
    onClick: () => void;
  }>;
}) {
  return (
    <div style={{ position: "relative" }}>
      <MobileIconButton iconName={iconName} label={label} onClick={onToggle} />
      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            right: 0,
            top: "calc(100% + var(--space-1))",
            zIndex: 50,
            minWidth: 180,
            padding: "var(--space-1)",
            border: "1px solid var(--color-border)",
            borderRadius: "var(--radius-md)",
            background: "var(--color-bg)",
            boxShadow: "var(--shadow-lg)",
          }}
        >
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              onClick={() => {
                item.onClick();
                onToggle();
              }}
              style={mobileMenuItemStyle}
            >
              <Icon name={item.iconName} size={16} />
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function MobileSortMenu({
  open,
  viewMode,
  sortField,
  sortDirection,
  onToggle,
  onSelect,
  onViewModeChange,
}: {
  open: boolean;
  viewMode: "grid" | "list";
  sortField: "name" | "size" | "modified_at";
  sortDirection: "asc" | "desc";
  onToggle: () => void;
  onSelect: (
    field: "name" | "size" | "modified_at",
    direction: "asc" | "desc",
  ) => void;
  onViewModeChange: (mode: "grid" | "list") => void;
}) {
  const options: Array<{
    label: string;
    field: "name" | "size" | "modified_at";
    direction: "asc" | "desc";
  }> = [
    { label: "Name A-Z", field: "name", direction: "asc" },
    { label: "Name Z-A", field: "name", direction: "desc" },
    { label: "Newest first", field: "modified_at", direction: "desc" },
    { label: "Oldest first", field: "modified_at", direction: "asc" },
    { label: "Largest first", field: "size", direction: "desc" },
    { label: "Smallest first", field: "size", direction: "asc" },
  ];

  return (
    <div style={{ position: "relative" }}>
      <MobileIconButton iconName="sliders" label="Sort" onClick={onToggle} />
      {open && (
        <div
          role="menu"
          style={{
            position: "absolute",
            right: 0,
            top: "calc(100% + var(--space-1))",
            zIndex: 50,
            minWidth: 190,
            padding: "var(--space-1)",
            border: "1px solid var(--color-border)",
            borderRadius: "var(--radius-md)",
            background: "var(--color-bg)",
            boxShadow: "var(--shadow-lg)",
          }}
        >
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "1fr 1fr",
              gap: "var(--space-1)",
              paddingBottom: "var(--space-1)",
              marginBottom: "var(--space-1)",
              borderBottom: "1px solid var(--color-border)",
            }}
          >
            {(["list", "grid"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                onClick={() => onViewModeChange(mode)}
                style={{
                  ...mobileMenuItemStyle,
                  justifyContent: "center",
                  background:
                    viewMode === mode
                      ? "var(--color-accent-muted)"
                      : "transparent",
                  color:
                    viewMode === mode
                      ? "var(--color-accent)"
                      : "var(--color-fg)",
                  fontWeight: viewMode === mode ? 700 : 500,
                }}
              >
                <Icon name={mode === "grid" ? "grid" : "list"} size={16} />
                {mode === "grid" ? "Thumbnails" : "List"}
              </button>
            ))}
          </div>
          {options.map((option) => {
            const active =
              sortField === option.field && sortDirection === option.direction;
            return (
              <button
                key={`${option.field}:${option.direction}`}
                type="button"
                role="menuitemradio"
                aria-checked={active}
                onClick={() => onSelect(option.field, option.direction)}
                style={{
                  ...mobileMenuItemStyle,
                  color: active ? "var(--color-accent)" : "var(--color-fg)",
                  fontWeight: active ? 700 : 500,
                }}
              >
                <Icon name={active ? "check" : "file"} size={16} />
                {option.label}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

const mobileMenuItemStyle: React.CSSProperties = {
  width: "100%",
  minHeight: 38,
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "0 var(--space-2)",
  border: "none",
  borderRadius: "var(--radius-sm)",
  background: "transparent",
  color: "var(--color-fg)",
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  textAlign: "left",
};

function MobileSelectionDrawer({
  root,
  rootDisplayName,
  selected,
  selectedCount,
  state,
  onStateChange,
  selectionStats,
  canWrite,
  canShare,
  onClear,
  onShare,
  onPreview,
  onDelete,
}: {
  root: string;
  rootDisplayName: string;
  selected: FileDetailsSelection | null;
  selectedCount: number;
  state: MobileDrawerState;
  onStateChange: (state: MobileDrawerState) => void;
  selectionStats: { totalSize: number; knownSize: boolean } | null;
  canWrite: boolean;
  canShare: boolean;
  onClear: () => void;
  onShare: () => void;
  onPreview: (entry: FileEntry, parentPath: string) => void;
  onDelete: () => void;
}) {
  const dragStartRef = useRef<{ y: number; state: MobileDrawerState } | null>(
    null,
  );
  const fullPath = selected?.path
    ? `${rootDisplayName}/${selected.path}`
    : rootDisplayName;
  const title =
    selected?.entry.name ?? `${formatCount(selectedCount, "item")} selected`;
  const sizeLabel = selectionStats
    ? selectionStats.knownSize
      ? formatFileSize(selectionStats.totalSize)
      : selectionStats.totalSize > 0
        ? `~${formatFileSize(selectionStats.totalSize)}`
        : "Calculating"
    : "";

  return (
    <div
      style={{
        position: "fixed",
        left: 0,
        right: 0,
        bottom: 0,
        zIndex: 40,
        display: "flex",
        flexDirection: "column",
        height:
          state === "full"
            ? "100dvh"
            : state === "half"
              ? "min(56vh, 360px)"
              : 76,
        maxHeight: state === "full" ? "100dvh" : undefined,
        borderTop: "1px solid var(--color-border)",
        borderTopLeftRadius: state === "full" ? 0 : "var(--radius-lg)",
        borderTopRightRadius: state === "full" ? 0 : "var(--radius-lg)",
        background: "var(--color-bg)",
        boxShadow: "var(--shadow-lg)",
        overflow: "hidden",
        transition: "height var(--duration-normal) var(--ease-out)",
      }}
    >
      <button
        type="button"
        onTouchStart={(event) => {
          dragStartRef.current = { y: event.touches[0].clientY, state };
        }}
        onTouchEnd={(event) => {
          const start = dragStartRef.current;
          dragStartRef.current = null;
          if (!start) return;
          const delta = event.changedTouches[0].clientY - start.y;
          if (Math.abs(delta) < 44) return;
          event.preventDefault();
          const states: MobileDrawerState[] = ["closed", "half", "full"];
          const index = states.indexOf(start.state);
          onStateChange(states[clamp(index + (delta > 0 ? -1 : 1), 0, 2)]);
        }}
        aria-label={
          state === "closed"
            ? "Open details"
            : state === "half"
              ? "Expand details"
              : "Collapse details"
        }
        onClick={() => {
          onStateChange(
            state === "closed" ? "half" : state === "half" ? "full" : "half",
          );
        }}
        style={{
          height: 18,
          border: "none",
          background: "transparent",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          cursor: "pointer",
          flexShrink: 0,
          touchAction: "none",
        }}
      >
        <span
          style={{
            width: 44,
            height: 5,
            borderRadius: 999,
            background: "var(--color-border)",
          }}
        />
      </button>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "minmax(0, 1fr) auto auto auto",
          alignItems: "center",
          gap: "var(--space-1)",
          padding: "0 var(--space-2) var(--space-2)",
          borderBottom: "1px solid var(--color-border)",
        }}
      >
        <div style={{ minWidth: 0 }}>
          <div
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontSize:
                state === "closed" ? "var(--text-sm)" : "var(--text-xs)",
              fontWeight: state === "closed" ? 700 : 500,
              color:
                state === "closed"
                  ? "var(--color-fg)"
                  : "var(--color-fg-subtle)",
            }}
            title={fullPath}
          >
            {state === "closed" ? title : selected ? fullPath : title}
          </div>
          {state !== "closed" && sizeLabel && (
            <div
              className="tabular-nums"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: "var(--color-fg-muted)",
                fontSize: "var(--text-xs)",
                marginTop: 2,
              }}
            >
              {sizeLabel}
            </div>
          )}
        </div>
        {canShare && (
          <MobileIconButton iconName="share2" label="Share" onClick={onShare} />
        )}
        {canWrite && (
          <MobileIconButton
            iconName="trash"
            label="Delete"
            danger
            onClick={onDelete}
          />
        )}
        <MobileIconButton
          iconName="x"
          label={state === "closed" ? "Unselect" : "Close"}
          onClick={() => {
            if (state === "closed") onClear();
            else onStateChange("closed");
          }}
        />
      </div>
      {state !== "closed" && (
        <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
          {selected ? (
            <FileDetailsPane
              root={root}
              selected={selected}
              width="100%"
              flush
              title="Selected"
              onPreview={onPreview}
            />
          ) : (
            <div
              style={{
                padding: "var(--space-5)",
                color: "var(--color-fg-muted)",
                fontSize: "var(--text-sm)",
              }}
            >
              {formatCount(selectedCount, "item")} selected
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function mobileListBottomPadding(
  selectedCount: number,
  state: MobileDrawerState,
): string {
  if (selectedCount === 0) return "92px";
  if (state === "closed") return "96px";
  if (state === "half") return "360px";
  return "92px";
}

function MobileLoadingList() {
  return (
    <div
      style={{
        display: "grid",
        gap: "var(--space-2)",
      }}
    >
      {Array.from({ length: 8 }).map((_, index) => (
        <div
          key={index}
          className="shimmer"
          style={{
            height: 64,
            borderRadius: "var(--radius-md)",
          }}
        />
      ))}
    </div>
  );
}

function MobileFileList({
  viewMode,
  entries,
  root,
  path,
  selectedPaths,
  transferJobs,
  scrollParentRef,
  onOpen,
  onSelect,
  onToggleSelect,
  onShowActions,
}: {
  viewMode: "grid" | "list";
  entries: FileEntry[];
  root: string;
  path: string;
  selectedPaths: Set<string>;
  transferJobs: TransferJob[];
  scrollParentRef: React.RefObject<HTMLElement | null>;
  onOpen: (entry: FileEntry) => void;
  onSelect: (path: string) => void;
  onToggleSelect: (path: string) => void;
  onShowActions: (entry: FileEntry, x: number, y: number) => void;
}) {
  const { sortField, sortDirection } = useViewStore();
  const sortedEntries = useMemo(
    () =>
      [...entries].sort((a, b) => {
        if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
        let cmp = 0;
        if (sortField === "name") {
          cmp = a.name.localeCompare(b.name, undefined, {
            sensitivity: "base",
          });
        } else if (sortField === "size") {
          cmp = a.size - b.size;
        } else {
          cmp = a.modified_at - b.modified_at;
        }
        return sortDirection === "asc" ? cmp : -cmp;
      }),
    [entries, sortDirection, sortField],
  );
  const columnCount = viewMode === "grid" ? 2 : 1;
  const rowCount = Math.ceil(sortedEntries.length / columnCount);
  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollParentRef.current,
    estimateSize: () => (viewMode === "grid" ? 230 : 68),
    overscan: viewMode === "grid" ? 3 : 8,
  });

  return (
    <div
      role="list"
      aria-label="Files"
      style={{
        position: "relative",
        height: rowVirtualizer.getTotalSize(),
      }}
    >
      {rowVirtualizer.getVirtualItems().map((virtualRow) => (
        <div
          key={virtualRow.key}
          ref={rowVirtualizer.measureElement}
          data-index={virtualRow.index}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            width: "100%",
            transform: `translateY(${virtualRow.start}px)`,
            display: "grid",
            gridTemplateColumns:
              viewMode === "grid" ? "repeat(2, minmax(0, 1fr))" : "1fr",
            gap: viewMode === "grid" ? "var(--space-2)" : "var(--space-1)",
            paddingBottom:
              viewMode === "grid" ? "var(--space-2)" : "var(--space-1)",
          }}
        >
          {sortedEntries
            .slice(
              virtualRow.index * columnCount,
              virtualRow.index * columnCount + columnCount,
            )
            .map((entry) => {
              const filePath = path ? `${path}/${entry.name}` : entry.name;
              const selected = selectedPaths.has(filePath);
              const icon = getFileIcon(entry);
              const entryTransferJobs = entry.is_dir
                ? transferJobsForTarget(transferJobs, root, filePath)
                : [];

              return (
                <div
                  key={entry.name}
                  role="listitem"
                  aria-selected={selected}
            style={{
              display: "grid",
              gridTemplateColumns:
                viewMode === "grid"
                  ? "minmax(0, 1fr) 40px"
                  : "44px minmax(0, 1fr) 40px",
              alignItems: "center",
              minHeight: 64,
              gap: "var(--space-1)",
              border: `1px solid ${selected ? "var(--color-accent)" : "transparent"}`,
              borderRadius: "var(--radius-md)",
              background: selected
                ? "var(--color-accent-muted)"
                : "transparent",
            }}
          >
            <button
              type="button"
              onClick={() => onToggleSelect(filePath)}
              aria-label={selected ? "Deselect item" : "Select item"}
              style={{
                width: 44,
                height: 56,
                border: "none",
                background: "transparent",
                color: selected
                  ? "var(--color-accent)"
                  : "var(--color-fg-subtle)",
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                ...(viewMode === "grid" ? { display: "none" } : {}),
              }}
            >
              <Icon name={selected ? "checkCircle" : "file"} size={18} />
            </button>

            <button
              type="button"
              onClick={() => {
                if (entry.is_dir) {
                  onOpen(entry);
                  return;
                }
                onSelect(filePath);
              }}
              style={{
                minWidth: 0,
                minHeight: 62,
                display: "grid",
                gridTemplateColumns:
                  viewMode === "grid"
                    ? "minmax(0, 1fr)"
                    : "32px minmax(0, 1fr)",
                alignItems: "center",
                gap: "var(--space-2)",
                border: "none",
                background: "transparent",
                color: "var(--color-fg)",
                textAlign: "left",
              }}
            >
              <span
                style={{
                  position: "relative",
                  display: "inline-flex",
                  minWidth: 0,
                }}
              >
                {viewMode === "grid" && entry.has_thumbnail ? (
                  <ThumbnailImage
                    root={root}
                    path={path}
                    entry={entry}
                    width={360}
                  />
                ) : (
                  <FileIcon
                    svg={icon.svg}
                    color={icon.color}
                    size={viewMode === "grid" ? 48 : 28}
                  />
                )}
                <span style={{ position: "absolute", right: -8, top: -7 }}>
                  <TransferProgressIndicator jobs={entryTransferJobs} compact />
                </span>
              </span>
              <span style={{ minWidth: 0 }}>
                <span
                  style={{
                    display: "block",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    fontSize: "var(--text-sm)",
                    fontWeight: entry.is_dir ? 600 : 500,
                    textAlign: viewMode === "grid" ? "center" : "left",
                  }}
                >
                  {entry.name}
                </span>
                <span
                  className="tabular-nums"
                  style={{
                    display: "block",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    marginTop: 2,
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-subtle)",
                    textAlign: viewMode === "grid" ? "center" : "left",
                  }}
                >
                  {entry.is_dir
                    ? entry.item_count != null
                      ? formatCount(entry.item_count, "item")
                      : "Folder"
                    : formatFileSize(entry.size)}
                  {" · "}
                  {formatModifiedDate(entry.modified_at)}
                </span>
              </span>
            </button>

            <button
              type="button"
              aria-label="More actions"
              onClick={(e) => {
                const rect = e.currentTarget.getBoundingClientRect();
                onShowActions(entry, rect.right - 220, rect.bottom + 4);
              }}
              style={{
                width: 40,
                height: 56,
                border: "none",
                background: "transparent",
                color: "var(--color-fg-subtle)",
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
              }}
            >
              <Icon name="moreVertical" size={16} />
            </button>
                </div>
              );
            })}
        </div>
      ))}
    </div>
  );
}

function MobileIconButton({
  iconName,
  label,
  danger = false,
  onClick,
}: {
  iconName: React.ComponentProps<typeof Icon>["name"];
  label: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      style={{
        width: 42,
        height: 42,
        border: "1px solid var(--color-border)",
        borderRadius: "var(--radius-md)",
        background: "transparent",
        color: danger ? "var(--color-danger)" : "var(--color-fg)",
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Icon name={iconName} size={17} />
    </button>
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
          iconName="trash"
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
