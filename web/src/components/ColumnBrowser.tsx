import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useQueries } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import api from "../api/client";
import type {
  DirectoryListing,
  FileEntry,
  Root,
  TransferJob,
} from "../api/client";
import { FileIcon, Icon } from "./Icon";
import { MiddleEllipsis } from "./MiddleEllipsis";
import { UsageRing } from "./UsageRing";
import { FileDetailsPane } from "./FileDetailsPane";
import {
  entryPath,
  hasNasfilesDrag,
  setFileDragPayload,
} from "../lib/fileDrag";
import { useGlobalDragCleanup } from "../lib/dragState";
import { getFileIcon } from "../lib/icons";
import { useViewStore } from "../state/view";
import { TransferProgressIndicator } from "./TransferProgressIndicator";
import {
  incomingTransferPlaceholders,
  moveJobsForSourcePath,
  transferJobsForTarget,
  transferProgressPercent,
} from "../lib/transferJobs";

const SHARE_WIDTH = { min: 180, max: 360 };
const FOLDER_WIDTH = { min: 220, max: 480 };
const INFO_WIDTH = { min: 260, max: 520 };
const COLUMN_ROW_HEIGHT = 32;

type FocusTarget =
  | { kind: "share"; index: number }
  | { kind: "folder"; columnIndex: number; rowIndex: number };

interface ColumnBrowserProps {
  roots: Root[];
  activeRoot: string;
  activePath: string;
  currentListing?: DirectoryListing;
  isLoading: boolean;
  error: unknown;
  onNavigateRoot: (root: string) => void;
  onNavigatePath: (path: string) => void;
  onOpenEntry: (entry: FileEntry, parentPath: string) => void;
  onPreviewEntry: (entry: FileEntry, parentPath: string) => void;
  onContextMenu: (
    e: React.MouseEvent,
    entry: FileEntry,
    parentPath: string,
  ) => void;
  onDropFiles: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  canDrop: boolean;
  transferJobs?: TransferJob[];
  onDisplayPathChange?: (path: string) => void;
  onActiveFolderPathChange?: (path: string) => void;
}

interface FolderColumn {
  path: string;
  title: string;
  entries: FileEntry[];
  isLoading: boolean;
  error: unknown;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function pathParts(path: string) {
  return path ? path.split("/").filter(Boolean) : [];
}

function parentColumnPaths(path: string) {
  const parts = pathParts(path);
  const paths = [""];
  for (let i = 0; i < parts.length; i += 1) {
    paths.push(parts.slice(0, i + 1).join("/"));
  }
  return paths;
}

function basename(path: string) {
  if (!path) return "Root";
  const parts = pathParts(path);
  return parts[parts.length - 1] || "Root";
}

function entryAt(entries: FileEntry[], index: number) {
  if (entries.length === 0) return null;
  return entries[clamp(index, 0, entries.length - 1)] ?? null;
}

function selectedPathValue() {
  const selected = useViewStore.getState().selectedPaths;
  return selected.size === 1 ? Array.from(selected)[0] : "";
}

function isSpaceKey(key: string) {
  return key === " " || key === "Space" || key === "Spacebar";
}

export function ColumnBrowser({
  roots,
  activeRoot,
  activePath,
  currentListing,
  isLoading,
  error,
  onNavigateRoot,
  onNavigatePath,
  onOpenEntry,
  onPreviewEntry,
  onContextMenu,
  onDropFiles,
  canDrop,
  transferJobs = [],
  onDisplayPathChange,
  onActiveFolderPathChange,
}: ColumnBrowserProps) {
  const {
    selectedPaths,
    select,
    toggleSelect,
    selectAll,
    clearSelection,
    shareColumnWidth,
    folderColumnWidth,
    infoColumnWidth,
    setShareColumnWidth,
    setInfoColumnWidth,
  } = useViewStore();
  const activeShareName =
    roots.find((root) => root.key === activeRoot)?.display_name ?? activeRoot;
  const [folderColumnWidths, setFolderColumnWidths] = useState<
    Record<string, number>
  >({});
  const [focus, setFocus] = useState<FocusTarget>(() => ({
    kind: "folder",
    columnIndex: Math.max(0, parentColumnPaths(activePath).length - 1),
    rowIndex: -1,
  }));
  const [revealedPath, setRevealedPath] = useState(activePath);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const columnRefs = useRef<Array<HTMLDivElement | null>>([]);
  const selectionAnchorRef = useRef<{
    columnPath: string;
    rowIndex: number;
  } | null>(null);

  const columnPaths = useMemo(
    () => parentColumnPaths(revealedPath),
    [revealedPath],
  );
  const columnQueries = useQueries({
    queries: columnPaths.map((columnPath) => ({
      queryKey: ["listing", activeRoot, columnPath],
      queryFn: () => api.listDirectory(activeRoot, columnPath),
      staleTime: 10_000,
      enabled: columnPath !== activePath || currentListing === undefined,
    })),
  });

  const folderColumns = columnQueries.map(
    (column, index): FolderColumn => ({
      path: columnPaths[index],
      title: index === 0 ? activeShareName : basename(columnPaths[index]),
      entries:
        (
          (columnPaths[index] === activePath ? currentListing : undefined) ??
          column.data
        )?.entries ?? [],
      isLoading:
        columnPaths[index] === activePath ? isLoading : column.isLoading,
      error: columnPaths[index] === activePath ? error : column.error,
    }),
  );

  const widthForColumn = useCallback(
    (columnPath: string) => folderColumnWidths[columnPath] ?? folderColumnWidth,
    [folderColumnWidth, folderColumnWidths],
  );

  const setWidthForColumn = useCallback((columnPath: string, width: number) => {
    setFolderColumnWidths((current) => ({ ...current, [columnPath]: width }));
  }, []);

  const selectedPath =
    selectedPaths.size === 1 ? Array.from(selectedPaths)[0] : "";
  let activeFolderPath = revealedPath || activePath;
  if (focus.kind === "folder") {
    const focusedColumn = folderColumns[focus.columnIndex];
    const focusedEntry = focusedColumn
      ? entryAt(focusedColumn.entries, focus.rowIndex)
      : null;
    if (focusedColumn) {
      activeFolderPath = focusedEntry?.is_dir
        ? entryPath(focusedColumn.path, focusedEntry.name)
        : focusedColumn.path;
    }
  }
  let selectedInfo: {
    entry: FileEntry;
    parentPath: string;
    path: string;
  } | null = null;
  if (selectedPath) {
    for (const column of folderColumns) {
      const match = column.entries.find(
        (entry) => entryPath(column.path, entry.name) === selectedPath,
      );
      if (match) {
        selectedInfo = {
          entry: match,
          parentPath: column.path,
          path: selectedPath,
        };
        break;
      }
    }
  }

  useEffect(() => {
    onDisplayPathChange?.(selectedPath || revealedPath || activePath);
  }, [activePath, onDisplayPathChange, revealedPath, selectedPath]);

  useEffect(() => {
    onActiveFolderPathChange?.(activeFolderPath);
  }, [activeFolderPath, onActiveFolderPathChange]);

  useEffect(() => {
    setRevealedPath(activePath);
    setFocus({
      kind: "folder",
      columnIndex: Math.max(0, parentColumnPaths(activePath).length - 1),
      rowIndex: -1,
    });
  }, [activeRoot, activePath]);

  useEffect(() => {
    if (focus.kind !== "folder") return;
    if (
      activePath &&
      revealedPath !== activePath &&
      !revealedPath.startsWith(`${activePath}/`)
    )
      return;
    if (
      activePath &&
      revealedPath === activePath &&
      focus.columnIndex < parentColumnPaths(activePath).length - 1
    )
      return;
    const nextColumnIndex = Math.min(
      focus.columnIndex,
      Math.max(0, folderColumns.length - 1),
    );
    const column = folderColumns[nextColumnIndex];
    const entries = column?.entries ?? [];
    const nextRowIndex =
      entries.length > 0 ? clamp(focus.rowIndex, 0, entries.length - 1) : -1;
    if (
      focus.columnIndex === nextColumnIndex &&
      focus.rowIndex === nextRowIndex
    )
      return;
    setFocus({
      kind: "folder",
      columnIndex: nextColumnIndex,
      rowIndex: nextRowIndex,
    });
    const entry = entryAt(entries, nextRowIndex);
    if (entry && column) {
      const nextPath = entryPath(column.path, entry.name);
      select(nextPath);
      setRevealedPath(entry.is_dir ? nextPath : column.path);
    }
  }, [activePath, folderColumns, focus, revealedPath, select]);

  useEffect(() => {
    if (useViewStore.getState().selectedPaths.size > 0) return;
    const activeColumn = folderColumns[folderColumns.length - 1];
    const firstEntry = activeColumn?.entries[0];
    if (firstEntry) select(entryPath(activeColumn.path, firstEntry.name));
  }, [folderColumns, select]);

  useEffect(() => {
    const lastColumn = columnRefs.current[folderColumns.length - 1];
    lastColumn?.scrollIntoView({ block: "nearest", inline: "end" });
  }, [folderColumns.length]);

  const startResize = useCallback(
    (
      e: React.PointerEvent,
      width: number,
      limits: { min: number; max: number },
      setter: (width: number) => void,
      direction: 1 | -1 = 1,
    ) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = width;
      const onMove = (event: PointerEvent) => {
        setter(
          clamp(
            startWidth + (event.clientX - startX) * direction,
            limits.min,
            limits.max,
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
    [],
  );

  const focusFolderEntry = useCallback(
    (columnIndex: number, rowIndex: number) => {
      const column = folderColumns[columnIndex];
      const entry = entryAt(column?.entries ?? [], rowIndex);
      const nextRowIndex = entry
        ? clamp(rowIndex, 0, column.entries.length - 1)
        : -1;
      setFocus({ kind: "folder", columnIndex, rowIndex: nextRowIndex });
      if (entry && column) {
        const nextPath = entryPath(column.path, entry.name);
        selectionAnchorRef.current = {
          columnPath: column.path,
          rowIndex: nextRowIndex,
        };
        select(nextPath);
        setRevealedPath(entry.is_dir ? nextPath : column.path);
      }
    },
    [folderColumns, select],
  );

  const selectFolderRange = useCallback(
    (columnIndex: number, rowIndex: number) => {
      const column = folderColumns[columnIndex];
      const entry = entryAt(column?.entries ?? [], rowIndex);
      if (!column || !entry) return;

      const nextRowIndex = clamp(rowIndex, 0, column.entries.length - 1);
      const anchor =
        selectionAnchorRef.current?.columnPath === column.path
          ? selectionAnchorRef.current
          : {
              columnPath: column.path,
              rowIndex:
                focus.kind === "folder" && focus.columnIndex === columnIndex
                  ? focus.rowIndex
                  : nextRowIndex,
            };
      const anchorRowIndex = clamp(
        anchor.rowIndex,
        0,
        column.entries.length - 1,
      );
      const lo = Math.min(anchorRowIndex, nextRowIndex);
      const hi = Math.max(anchorRowIndex, nextRowIndex);
      const paths = column.entries
        .slice(lo, hi + 1)
        .map((rangeEntry) => entryPath(column.path, rangeEntry.name));

      selectionAnchorRef.current = {
        columnPath: column.path,
        rowIndex: anchorRowIndex,
      };
      setFocus({ kind: "folder", columnIndex, rowIndex: nextRowIndex });
      selectAll(paths);
      setRevealedPath(column.path);
    },
    [focus, folderColumns, selectAll],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey) return;

      if (focus.kind === "share") {
        const nextShare = (delta: number) => {
          const index = clamp(focus.index + delta, 0, roots.length - 1);
          setFocus({ kind: "share", index });
          const nextRoot = roots[index];
          if (nextRoot && nextRoot.key !== activeRoot)
            onNavigateRoot(nextRoot.key);
        };

        if (e.key === "ArrowDown") {
          e.preventDefault();
          nextShare(1);
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          nextShare(-1);
        }
        if (e.key === "ArrowRight") {
          e.preventDefault();
          setFocus({ kind: "folder", columnIndex: 0, rowIndex: 0 });
        }
        if (e.key === "Enter") {
          e.preventDefault();
          onNavigateRoot(roots[focus.index]?.key ?? activeRoot);
        }
        return;
      }

      const column = folderColumns[focus.columnIndex];
      const currentEntry = column
        ? entryAt(column.entries, focus.rowIndex)
        : null;

      if (e.key === "ArrowDown") {
        e.preventDefault();
        if (e.shiftKey) {
          selectFolderRange(focus.columnIndex, focus.rowIndex + 1);
          return;
        }
        focusFolderEntry(focus.columnIndex, focus.rowIndex + 1);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        if (e.shiftKey) {
          selectFolderRange(focus.columnIndex, focus.rowIndex - 1);
          return;
        }
        focusFolderEntry(focus.columnIndex, focus.rowIndex - 1);
        return;
      }
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        if (focus.columnIndex <= 0) {
          setFocus({
            kind: "share",
            index: Math.max(
              0,
              roots.findIndex((root) => root.key === activeRoot),
            ),
          });
        } else {
          const parentColumnIndex = focus.columnIndex - 1;
          const parentColumn = folderColumns[parentColumnIndex];
          const childName = basename(column?.path ?? "");
          const parentRow = parentColumn.entries.findIndex(
            (entry) => entry.name === childName,
          );
          focusFolderEntry(parentColumnIndex, parentRow >= 0 ? parentRow : 0);
        }
        return;
      }
      if (e.key === "ArrowRight") {
        e.preventDefault();
        if (currentEntry?.is_dir && column) {
          const nextPath = entryPath(column.path, currentEntry.name);
          setRevealedPath(nextPath);
          onNavigatePath(nextPath);
          setFocus({
            kind: "folder",
            columnIndex: focus.columnIndex + 1,
            rowIndex: 0,
          });
        }
        return;
      }
      if (e.key === "Enter") {
        if (currentEntry && column) {
          e.preventDefault();
          onOpenEntry(currentEntry, column.path);
        }
        return;
      }
      if (isSpaceKey(e.key) && !e.repeat) {
        if (currentEntry && !currentEntry.is_dir && column) {
          e.preventDefault();
          onPreviewEntry(currentEntry, column.path);
          return;
        }
        const selected = selectedPathValue();
        for (const selectedColumn of folderColumns) {
          const selectedEntry = selectedColumn.entries.find(
            (entry) => entryPath(selectedColumn.path, entry.name) === selected,
          );
          if (selectedEntry && !selectedEntry.is_dir) {
            e.preventDefault();
            onPreviewEntry(selectedEntry, selectedColumn.path);
            return;
          }
        }
      }
    },
    [
      activeRoot,
      focus,
      focusFolderEntry,
      folderColumns,
      onNavigatePath,
      onNavigateRoot,
      onOpenEntry,
      onPreviewEntry,
      roots,
      selectFolderRange,
    ],
  );

  return (
    <div
      role="application"
      aria-label="Column file browser"
      tabIndex={0}
      onKeyDown={handleKeyDown}
      style={{
        display: "grid",
        gridTemplateColumns: `${shareColumnWidth}px minmax(0, 1fr) ${infoColumnWidth}px`,
        flex: 1,
        height: "100%",
        minHeight: 0,
        overflow: "hidden",
        outline: "none",
        position: "relative",
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) clearSelection();
      }}
    >
      <aside
        aria-label="Shares"
        style={{
          position: "sticky",
          left: 0,
          zIndex: 2,
          display: "flex",
          flexDirection: "column",
          height: "100%",
          minHeight: 0,
          minWidth: 0,
          borderRight: "1px solid var(--color-border)",
          background: "var(--color-sidebar-bg)",
          overflowY: "auto",
        }}
      >
        <div style={columnTitleStyle}>Shares</div>
        {roots.map((share, index) => {
          const active = share.key === activeRoot;
          const focused = focus.kind === "share" && focus.index === index;
          // Roots arrive ordered with each group's members contiguous, so a
          // non-interactive label is emitted whenever the group changes. The
          // share buttons below stay one-per-root in array order, keeping the
          // index-based keyboard navigation intact.
          const prevGroup = index > 0 ? roots[index - 1].group : null;
          const showGroupHeader = !!share.group && share.group !== prevGroup;
          return (
            <Fragment key={share.key}>
              {showGroupHeader && (
                <div style={shareGroupHeaderStyle} aria-hidden="true">
                  <Icon
                    name="folders"
                    size={13}
                    color="var(--color-fg-subtle)"
                  />
                  <span style={truncateStyle}>{share.group}</span>
                </div>
              )}
              <button
                onClick={() => {
                  setFocus({ kind: "share", index });
                  clearSelection();
                  onNavigateRoot(share.key);
                }}
                onFocus={() => setFocus({ kind: "share", index })}
                style={{
                  ...shareRowStyle,
                  background: active
                    ? "var(--color-sidebar-active)"
                    : "transparent",
                  color: active
                    ? "var(--color-accent)"
                    : "var(--color-sidebar-fg)",
                  boxShadow: focused
                    ? "inset 0 0 0 2px var(--color-accent)"
                    : "none",
                }}
              >
                <Icon
                  name={share.kind === "home" ? "home" : "folder"}
                  size={16}
                />
                <span style={truncateStyle}>{share.display_name}</span>
                <UsageRing usage={share.usage} />
              </button>
            </Fragment>
          );
        })}
      </aside>

      <ResizeHandle
        left={shareColumnWidth - 3}
        onPointerDown={(e) =>
          startResize(e, shareColumnWidth, SHARE_WIDTH, setShareColumnWidth)
        }
      />

      <div
        ref={scrollerRef}
        style={{
          display: "flex",
          height: "100%",
          minHeight: 0,
          minWidth: 0,
          overflowX: "auto",
          overflowY: "hidden",
          background: "var(--color-bg)",
        }}
      >
        {folderColumns.map((column, columnIndex) => {
          const width = widthForColumn(column.path);
          return (
            <FolderColumnView
              key={`${activeRoot}:${column.path}`}
              refCallback={(element) => {
                columnRefs.current[columnIndex] = element;
              }}
              column={column}
              root={activeRoot}
              width={width}
              selectedPaths={selectedPaths}
              focusedRow={
                focus.kind === "folder" && focus.columnIndex === columnIndex
                  ? focus.rowIndex
                  : -1
              }
              canDrop={canDrop}
              transferJobs={transferJobs}
              onResizeStart={(e) =>
                startResize(e, width, FOLDER_WIDTH, (nextWidth) =>
                  setWidthForColumn(column.path, nextWidth),
                )
              }
              onSelect={(entry, rowIndex) => {
                setFocus({ kind: "folder", columnIndex, rowIndex });
                const nextPath = entryPath(column.path, entry.name);
                selectionAnchorRef.current = {
                  columnPath: column.path,
                  rowIndex,
                };
                select(nextPath);
                setRevealedPath(entry.is_dir ? nextPath : column.path);
              }}
              onToggleSelect={(entry, rowIndex) => {
                setFocus({ kind: "folder", columnIndex, rowIndex });
                const nextPath = entryPath(column.path, entry.name);
                selectionAnchorRef.current = {
                  columnPath: column.path,
                  rowIndex,
                };
                toggleSelect(nextPath);
                setRevealedPath(column.path);
              }}
              onRangeSelect={(_, rowIndex) => {
                selectFolderRange(columnIndex, rowIndex);
              }}
              onOpen={(entry) => onOpenEntry(entry, column.path)}
              onPreview={(entry) => onPreviewEntry(entry, column.path)}
              onContextMenu={(e, entry) => onContextMenu(e, entry, column.path)}
              onDropFiles={onDropFiles}
            />
          );
        })}
      </div>

      <ResizeHandle
        right={infoColumnWidth - 3}
        onPointerDown={(e) =>
          startResize(e, infoColumnWidth, INFO_WIDTH, setInfoColumnWidth, -1)
        }
      />

      <FileDetailsPane
        width={infoColumnWidth}
        sticky
        root={activeRoot}
        selected={selectedInfo}
        onPreview={(entry, parentPath) => onPreviewEntry(entry, parentPath)}
      />
    </div>
  );
}

function FolderColumnView({
  refCallback,
  column,
  root,
  width,
  selectedPaths,
  focusedRow,
  canDrop,
  transferJobs,
  onSelect,
  onToggleSelect,
  onRangeSelect,
  onOpen,
  onPreview,
  onContextMenu,
  onDropFiles,
  onResizeStart,
}: {
  refCallback: (element: HTMLDivElement | null) => void;
  column: FolderColumn;
  root: string;
  width: number;
  selectedPaths: Set<string>;
  focusedRow: number;
  canDrop: boolean;
  transferJobs: TransferJob[];
  onSelect: (entry: FileEntry, rowIndex: number) => void;
  onToggleSelect: (entry: FileEntry, rowIndex: number) => void;
  onRangeSelect: (entry: FileEntry, rowIndex: number) => void;
  onOpen: (entry: FileEntry) => void;
  onPreview: (entry: FileEntry) => void;
  onContextMenu: (e: React.MouseEvent, entry: FileEntry) => void;
  onDropFiles: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  onResizeStart: (e: React.PointerEvent) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [isDropTarget, setIsDropTarget] = useState(false);
  const resetDropTarget = useCallback(() => setIsDropTarget(false), []);
  const columnTransferJobs = transferJobsForTarget(
    transferJobs,
    root,
    column.path,
  );
  const transferPlaceholders = incomingTransferPlaceholders(
    transferJobs,
    root,
    column.path,
    column.entries.map((entry) => entry.name),
  );
  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: column.entries.length + transferPlaceholders.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => COLUMN_ROW_HEIGHT,
    overscan: 12,
  });

  useEffect(() => {
    if (focusedRow < 0) return;
    rowVirtualizer.scrollToIndex(focusedRow, { align: "auto" });
    window.requestAnimationFrame(() => {
      scrollRef.current
        ?.querySelector<HTMLButtonElement>(
          `[data-column-row-index="${focusedRow}"]`,
        )
        ?.focus({ preventScroll: true });
    });
  }, [focusedRow, rowVirtualizer]);

  useGlobalDragCleanup(resetDropTarget);

  return (
    <section
      ref={refCallback}
      aria-label={column.title}
      onDragEnter={(e) => {
        if (!canDrop || !hasNasfilesDrag(e.dataTransfer)) return;
        e.preventDefault();
        setIsDropTarget(true);
      }}
      onDragOver={(e) => {
        if (!canDrop || !hasNasfilesDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect =
          e.dataTransfer.effectAllowed === "copy" ? "copy" : "move";
      }}
      onDragLeave={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
        resetDropTarget();
      }}
      onDrop={(e) => {
        if (!canDrop || !hasNasfilesDrag(e.dataTransfer)) return;
        e.preventDefault();
        e.stopPropagation();
        resetDropTarget();
        onDropFiles(root, column.path, e);
      }}
      style={{
        width,
        minWidth: width,
        maxWidth: width,
        height: "100%",
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        borderRight: "1px solid var(--color-border)",
        outline: isDropTarget ? "2px solid var(--color-accent)" : "none",
        outlineOffset: -2,
        background: isDropTarget ? "var(--color-accent-muted)" : "transparent",
        position: "relative",
      }}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        onPointerDown={onResizeStart}
        style={{
          position: "absolute",
          top: 0,
          right: -3,
          bottom: 0,
          width: 6,
          zIndex: 3,
          cursor: "col-resize",
        }}
      />
      <div
        style={{
          ...columnTitleStyle,
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
        }}
      >
        <span style={truncateStyle}>{column.title}</span>
        <TransferProgressIndicator jobs={columnTransferJobs} compact />
      </div>
      <div
        ref={scrollRef}
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          padding: "var(--space-1)",
        }}
      >
        {column.isLoading && (
          <div
            style={{
              display: "grid",
              gap: "var(--space-1)",
              padding: "var(--space-2)",
            }}
          >
            {Array.from({ length: 8 }).map((_, index) => (
              <div
                key={index}
                className="shimmer"
                style={{ height: 28, borderRadius: "var(--radius-sm)" }}
              />
            ))}
          </div>
        )}
        {Boolean(column.error) && (
          <div style={emptyColumnStyle}>Failed to load</div>
        )}
        {!column.isLoading &&
          !column.error &&
          column.entries.length === 0 &&
          transferPlaceholders.length === 0 && (
            <div style={emptyColumnStyle}>Empty folder</div>
          )}
        {!column.isLoading &&
          !column.error &&
          (column.entries.length > 0 || transferPlaceholders.length > 0) && (
            <div
              style={{
                position: "relative",
                height: rowVirtualizer.getTotalSize(),
              }}
            >
              {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                const rowIndex = virtualRow.index;

                if (rowIndex >= column.entries.length) {
                  const placeholder =
                    transferPlaceholders[rowIndex - column.entries.length];
                  return (
                    <ColumnTransferPlaceholderRow
                      key={placeholder.key}
                      name={placeholder.name}
                      percent={transferProgressPercent([placeholder.job])}
                      style={{
                        position: "absolute",
                        top: 0,
                        left: 0,
                        height: COLUMN_ROW_HEIGHT,
                        transform: `translateY(${virtualRow.start}px)`,
                      }}
                    />
                  );
                }

                const entry = column.entries[rowIndex];
                const fullPath = entryPath(column.path, entry.name);
                return (
                  <ColumnEntryRow
                    key={entry.name}
                    rowIndex={rowIndex}
                    entry={entry}
                    root={root}
                    path={column.path}
                    fullPath={fullPath}
                    selected={selectedPaths.has(fullPath)}
                    focused={focusedRow === rowIndex}
                    canDrop={canDrop}
                    transferJobs={transferJobs}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      height: COLUMN_ROW_HEIGHT,
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                    onClick={() => onSelect(entry, rowIndex)}
                    onToggleClick={() => onToggleSelect(entry, rowIndex)}
                    onRangeClick={() => onRangeSelect(entry, rowIndex)}
                    onDoubleClick={() => onOpen(entry)}
                    onPreview={() => onPreview(entry)}
                    onContextMenu={(e) => onContextMenu(e, entry)}
                    onDropFiles={onDropFiles}
                  />
                );
              })}
            </div>
          )}
      </div>
    </section>
  );
}

function ColumnTransferPlaceholderRow({
  name,
  percent,
  style,
}: {
  name: string;
  percent: number;
  style?: React.CSSProperties;
}) {
  const icon = getFileIcon({
    name,
    size: 0,
    modified_at: 0,
    is_dir: false,
    mime_type: null,
    has_thumbnail: false,
  });

  return (
    <div
      role="row"
      aria-disabled="true"
      style={{
        ...style,
        display: "grid",
        gridTemplateColumns: "20px minmax(0, 1fr) 18px",
        alignItems: "center",
        gap: "var(--space-2)",
        width: "100%",
        minHeight: 30,
        padding: "var(--space-1) var(--space-2)",
        borderRadius: "var(--radius-sm)",
        color: "var(--color-fg-muted)",
        fontSize: "var(--text-sm)",
        opacity: 0.58,
        pointerEvents: "none",
      }}
    >
      <FileIcon svg={icon.svg} color="var(--color-fg-subtle)" size={18} />
      <span style={{ minWidth: 0, fontWeight: 400, overflow: "hidden" }}>
        <MiddleEllipsis text={name} maxWidth={Number.MAX_SAFE_INTEGER} />
      </span>
      <span
        title={`${percent}%`}
        style={{
          width: 16,
          height: 16,
          borderRadius: "50%",
          background: `conic-gradient(var(--color-accent) ${percent}%, var(--color-border) 0)`,
          boxShadow:
            "inset 0 0 0 1px color-mix(in oklch, var(--color-border), transparent 25%)",
        }}
      />
    </div>
  );
}

function ColumnEntryRow({
  rowIndex,
  entry,
  root,
  path,
  fullPath,
  selected,
  focused,
  canDrop,
  transferJobs,
  style,
  onClick,
  onToggleClick,
  onRangeClick,
  onDoubleClick,
  onPreview,
  onContextMenu,
  onDropFiles,
}: {
  rowIndex: number;
  entry: FileEntry;
  root: string;
  path: string;
  fullPath: string;
  selected: boolean;
  focused: boolean;
  canDrop: boolean;
  transferJobs: TransferJob[];
  style?: React.CSSProperties;
  onClick: () => void;
  onToggleClick: () => void;
  onRangeClick: () => void;
  onDoubleClick: () => void;
  onPreview: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onDropFiles: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
}) {
  const [isDropTarget, setIsDropTarget] = useState(false);
  const resetDropTarget = useCallback(() => setIsDropTarget(false), []);
  const icon = getFileIcon(entry);
  const rowTransferJobs = entry.is_dir
    ? transferJobsForTarget(transferJobs, root, fullPath)
    : [];
  const sourceMoveJobs = entry.is_dir
    ? moveJobsForSourcePath(transferJobs, root, fullPath)
    : [];
  const isBeingMoved = sourceMoveJobs.length > 0;
  const displayedTransferJobs = isBeingMoved ? sourceMoveJobs : rowTransferJobs;

  useGlobalDragCleanup(resetDropTarget);

  return (
    <button
      type="button"
      data-column-entry="true"
      data-column-row-index={rowIndex}
      draggable
      aria-selected={selected}
      aria-busy={isBeingMoved || undefined}
      onClick={(e) => {
        if (e.shiftKey) {
          onRangeClick();
          return;
        }
        if (e.metaKey || e.ctrlKey) {
          onToggleClick();
          return;
        }
        onClick();
      }}
      onDoubleClick={(e) => {
        if (e.shiftKey) {
          e.preventDefault();
          return;
        }
        onDoubleClick();
      }}
      onContextMenu={onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          e.stopPropagation();
          onDoubleClick();
        }
        if (isSpaceKey(e.key) && !e.repeat && !entry.is_dir) {
          e.preventDefault();
          e.stopPropagation();
          onPreview();
        }
      }}
      onDragStart={(e) => {
        const selectedPaths = useViewStore.getState().selectedPaths;
        const paths = selectedPaths.has(fullPath)
          ? Array.from(selectedPaths)
          : [fullPath];
        if (!selectedPaths.has(fullPath))
          useViewStore.getState().select(fullPath);
        setFileDragPayload(
          e.dataTransfer,
          { root, paths },
          {
            label: entry.name,
            iconSvg: icon.svg,
            iconColor: icon.color,
          },
        );
      }}
      onDragEnd={resetDropTarget}
      onDragEnter={(e) => {
        if (!entry.is_dir || !canDrop || !hasNasfilesDrag(e.dataTransfer))
          return;
        e.preventDefault();
        e.stopPropagation();
        setIsDropTarget(true);
      }}
      onDragOver={(e) => {
        if (!entry.is_dir || !canDrop || !hasNasfilesDrag(e.dataTransfer))
          return;
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect =
          e.dataTransfer.effectAllowed === "copy" ? "copy" : "move";
      }}
      onDragLeave={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
        resetDropTarget();
      }}
      onDrop={(e) => {
        if (!entry.is_dir || !canDrop || !hasNasfilesDrag(e.dataTransfer))
          return;
        e.preventDefault();
        e.stopPropagation();
        resetDropTarget();
        onDropFiles(root, entryPath(path, entry.name), e);
      }}
      style={{
        ...style,
        display: "grid",
        gridTemplateColumns: "20px minmax(0, 1fr) auto 24px",
        alignItems: "center",
        gap: "var(--space-2)",
        width: "100%",
        minHeight: 30,
        padding: "var(--space-1) var(--space-2)",
        border: "none",
        borderRadius: "var(--radius-sm)",
        background:
          isDropTarget || selected
            ? "var(--color-accent-muted)"
            : focused
              ? "var(--color-bg-muted)"
              : "transparent",
        color: isBeingMoved
          ? "var(--color-fg-muted)"
          : selected
            ? "var(--color-accent)"
            : "var(--color-fg)",
        cursor: isBeingMoved ? "progress" : "pointer",
        fontSize: "var(--text-sm)",
        textAlign: "left",
        outline: focused ? "1px solid var(--color-accent)" : "none",
        outlineOffset: -1,
        opacity: isBeingMoved ? 0.48 : 1,
      }}
    >
      <FileIcon
        svg={icon.svg}
        color={isBeingMoved ? "var(--color-fg-subtle)" : icon.color}
        size={18}
      />
      <span
        style={{
          minWidth: 0,
          fontWeight: entry.is_dir ? 500 : 400,
          overflow: "hidden",
        }}
      >
        <MiddleEllipsis text={entry.name} maxWidth={Number.MAX_SAFE_INTEGER} />
      </span>
      {isBeingMoved ? (
        <span
          style={{
            fontSize: "var(--text-xs)",
            color: "var(--color-fg-subtle)",
          }}
        >
          Moving...
        </span>
      ) : (
        <TransferProgressIndicator jobs={displayedTransferJobs} compact />
      )}
      <span
        style={{
          justifySelf: "end",
          visibility: entry.is_dir ? "visible" : "hidden",
        }}
      >
        <Icon name="chevronRight" size={14} color="var(--color-fg-subtle)" />
      </span>
    </button>
  );
}

function ResizeHandle({
  left,
  right,
  onPointerDown,
}: {
  left?: number;
  right?: number;
  onPointerDown: (e: React.PointerEvent) => void;
}) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      onPointerDown={onPointerDown}
      style={{
        position: "absolute",
        top: 0,
        bottom: 0,
        left,
        right,
        zIndex: 4,
        width: 6,
        cursor: "col-resize",
      }}
    >
      <div
        style={{
          position: "absolute",
          top: 0,
          bottom: 0,
          left: 2,
          width: 1,
          background: "transparent",
        }}
      />
    </div>
  );
}

const columnTitleStyle: React.CSSProperties = {
  flexShrink: 0,
  padding: "var(--space-2) var(--space-3)",
  borderBottom: "1px solid var(--color-border)",
  color: "var(--color-fg-subtle)",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: "var(--tracking-wide)",
};

const shareRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  width: "100%",
  padding: "var(--space-2) var(--space-3)",
  border: "none",
  borderRadius: 0,
  cursor: "pointer",
  fontSize: "var(--text-sm)",
  textAlign: "left",
};

const shareGroupHeaderStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: "var(--space-2)",
  padding: "var(--space-2) var(--space-3) var(--space-1)",
  color: "var(--color-fg-subtle)",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  textTransform: "uppercase",
  letterSpacing: "var(--tracking-wide)",
};

const truncateStyle: React.CSSProperties = {
  minWidth: 0,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const emptyColumnStyle: React.CSSProperties = {
  padding: "var(--space-4)",
  color: "var(--color-fg-subtle)",
  fontSize: "var(--text-sm)",
};
