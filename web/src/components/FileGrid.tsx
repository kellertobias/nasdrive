import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { FileEntry } from "../api/client";
import { getFileIcon, formatFileSize, hasThumbnail } from "../lib/icons";
import { useViewStore } from "../state/view";
import { MiddleEllipsis } from "./MiddleEllipsis";
import { FileIcon } from "./Icon";
import { ThumbnailImage } from "./ThumbnailImage";
import {
  entryPath,
  hasExternalFileDrag,
  hasNasfilesDrag,
  isDemoDraggedPath,
  isDemoDragHoverPath,
  isDemoDropTarget,
  setFileDragPayload,
} from "../lib/fileDrag";
import { useGlobalDragCleanup } from "../lib/dragState";
import type { TransferJob } from "../api/client";
import { TransferProgressIndicator } from "./TransferProgressIndicator";
import {
  incomingTransferPlaceholders,
  moveJobsForSourcePath,
  transferJobsForTarget,
  transferProgressPercent,
} from "../lib/transferJobs";
import { GalleryFeedbackBadges } from "./GalleryFeedbackBadges";

interface FileGridProps {
  entries: FileEntry[];
  onOpen: (entry: FileEntry) => void;
  root?: string;
  path: string;
  scrollParentRef?: React.RefObject<HTMLElement | null>;
  onContextMenu?: (e: React.MouseEvent, entry: FileEntry) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs?: TransferJob[];
}

const GRID_MIN_TILE_WIDTH = 160;
const GRID_GAP = 12;
const GRID_TILE_HEIGHT = 174;

type GridItem =
  | { kind: "entry"; entry: FileEntry }
  | {
      kind: "placeholder";
      placeholder: ReturnType<typeof incomingTransferPlaceholders>[number];
    };

function useElementWidth(ref: React.RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(0);

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    const resizeObserver = new ResizeObserver(([entry]) => {
      setWidth(entry.contentRect.width);
    });
    resizeObserver.observe(element);
    setWidth(element.getBoundingClientRect().width);

    return () => resizeObserver.disconnect();
  }, [ref]);

  return width;
}

export function FileGrid({
  entries,
  onOpen,
  root = "",
  path,
  scrollParentRef,
  onContextMenu,
  onDropFiles,
  transferJobs = [],
}: FileGridProps) {
  const { selectedPaths, select, toggleSelect, rangeSelect, clearSelection } =
    useViewStore();
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const resetDropTarget = useCallback(() => setDropTarget(null), []);
  const gridRef = useRef<HTMLDivElement>(null);
  const gridWidth = useElementWidth(gridRef);
  const transferPlaceholders = incomingTransferPlaceholders(
    transferJobs,
    root,
    path,
    entries.map((entry) => entry.name),
  );
  const items = useMemo<GridItem[]>(
    () => [
      ...entries.map((entry) => ({ kind: "entry" as const, entry })),
      ...transferPlaceholders.map((placeholder) => ({
        kind: "placeholder" as const,
        placeholder,
      })),
    ],
    [entries, transferPlaceholders],
  );
  const columnCount = Math.max(
    1,
    Math.floor((gridWidth + GRID_GAP) / (GRID_MIN_TILE_WIDTH + GRID_GAP)),
  );
  const rowCount = Math.ceil(items.length / columnCount);
  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollParentRef?.current ?? gridRef.current,
    estimateSize: () => GRID_TILE_HEIGHT + GRID_GAP,
    overscan: 4,
  });
  // Track the last-clicked item index for shift-range selection
  const lastClickedIndex = useRef<number>(-1);

  useGlobalDragCleanup(resetDropTarget);

  return (
    <div
      ref={gridRef}
      role="grid"
      aria-label="Files"
      style={{
        position: "relative",
        minHeight: rowVirtualizer.getTotalSize(),
      }}
      // Clicking empty grid space deselects all
      onClick={(e) => {
        if (e.target === e.currentTarget) clearSelection();
      }}
    >
      {rowVirtualizer.getVirtualItems().flatMap((virtualRow) => {
        const rowStart = virtualRow.index * columnCount;
        return items
          .slice(rowStart, rowStart + columnCount)
          .map((item, columnIndex) => {
            const index = rowStart + columnIndex;
            const width = `calc((100% - ${GRID_GAP * (columnCount - 1)}px) / ${columnCount})`;

            if (item.kind === "placeholder") {
              const { placeholder } = item;
              const percent = transferProgressPercent([placeholder.job]);

              return (
                <div
                  key={placeholder.key}
                  role="gridcell"
                  aria-disabled="true"
                  style={{
                    position: "absolute",
                    top: 0,
                    left: `calc(${columnIndex} * (${width} + ${GRID_GAP}px))`,
                    width,
                    height: GRID_TILE_HEIGHT,
                    transform: `translateY(${virtualRow.start}px)`,
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    padding: "var(--space-4) var(--space-3)",
                    border: "2px solid transparent",
                    borderRadius: "var(--radius-lg)",
                    background: "transparent",
                    color: "var(--color-fg-muted)",
                    textAlign: "center",
                    opacity: 0.58,
                    cursor: "default",
                    outline: "none",
                    pointerEvents: "none",
                  }}
                >
                  <FileIcon
                    svg={
                      getFileIcon({
                        name: placeholder.name,
                        size: 0,
                        modified_at: 0,
                        is_dir: false,
                        mime_type: null,
                        has_thumbnail: false,
                      }).svg
                    }
                    color="var(--color-fg-subtle)"
                    size={36}
                  />

                  <div
                    style={{
                      width: "100%",
                      fontSize: "var(--text-sm)",
                      fontWeight: 450,
                      lineHeight: "var(--leading-sm)",
                    }}
                  >
                    <MiddleEllipsis text={placeholder.name} maxWidth={140} />
                  </div>

                  <div
                    style={{
                      width: "100%",
                      height: 4,
                      borderRadius: 2,
                      background: "var(--color-border)",
                      overflow: "hidden",
                    }}
                  >
                    <div
                      style={{
                        width: `${percent}%`,
                        height: "100%",
                        borderRadius: 2,
                        background: "var(--color-accent)",
                        transition: "width 200ms ease-out",
                      }}
                    />
                  </div>
                </div>
              );
            }

            const { entry } = item;
            const filePath = entryPath(path, entry.name);
            const isSelected = selectedPaths.has(filePath);
            const icon = getFileIcon(entry);
            const showThumb = hasThumbnail(entry) && root;
            const isDropTarget =
              dropTarget === filePath || isDemoDropTarget(root, filePath);
            const isBeingDragged = isDemoDraggedPath(root, filePath);
            const isDragHover = isDemoDragHoverPath(root, filePath);
            const entryTransferJobs = entry.is_dir
              ? transferJobsForTarget(transferJobs, root, filePath)
              : [];
            const sourceMoveJobs = entry.is_dir
              ? moveJobsForSourcePath(transferJobs, root, filePath)
              : [];
            const isBeingMoved = sourceMoveJobs.length > 0;
            const displayedTransferJobs = isBeingMoved
              ? sourceMoveJobs
              : entryTransferJobs;

            return (
              <button
                key={entry.name}
                role="gridcell"
                aria-selected={isSelected}
                aria-busy={isBeingMoved || undefined}
                draggable={Boolean(root)}
                onDragStart={(e) => {
                  if (!root) return;
                  const visiblePaths = entries.map((en) =>
                    entryPath(path, en.name),
                  );
                  const paths = selectedPaths.has(filePath)
                    ? visiblePaths.filter((visiblePath) =>
                        selectedPaths.has(visiblePath),
                      )
                    : [filePath];
                  if (!selectedPaths.has(filePath)) select(filePath);
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
                onClick={(e) => {
                  e.stopPropagation();
                  if (e.shiftKey && lastClickedIndex.current >= 0) {
                    // Range select: add all paths between lastClicked and current
                    const lo = Math.min(lastClickedIndex.current, index);
                    const hi = Math.max(lastClickedIndex.current, index);
                    const rangePaths = entries
                      .slice(lo, hi + 1)
                      .map((en) => (path ? `${path}/${en.name}` : en.name));
                    rangeSelect(rangePaths);
                  } else if (e.metaKey || e.ctrlKey) {
                    toggleSelect(filePath);
                    lastClickedIndex.current = index;
                  } else {
                    select(filePath);
                    lastClickedIndex.current = index;
                  }
                }}
                onDoubleClick={() => onOpen(entry)}
                onContextMenu={(e) => onContextMenu?.(e, entry)}
                onDragEnter={(e) => {
                  if (
                    !entry.is_dir ||
                    !root ||
                    !onDropFiles ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  e.preventDefault();
                  e.stopPropagation();
                  setDropTarget(filePath);
                }}
                onDragOver={(e) => {
                  if (
                    !entry.is_dir ||
                    !root ||
                    !onDropFiles ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  e.preventDefault();
                  e.stopPropagation();
                  e.dataTransfer.dropEffect =
                    hasExternalFileDrag(e.dataTransfer) ||
                    e.dataTransfer.effectAllowed === "copy"
                      ? "copy"
                      : "move";
                }}
                onDragLeave={(e) => {
                  if (e.currentTarget.contains(e.relatedTarget as Node | null))
                    return;
                  resetDropTarget();
                }}
                onDrop={(e) => {
                  if (
                    !entry.is_dir ||
                    !root ||
                    !onDropFiles ||
                    !(
                      hasNasfilesDrag(e.dataTransfer) ||
                      hasExternalFileDrag(e.dataTransfer)
                    )
                  )
                    return;
                  e.preventDefault();
                  e.stopPropagation();
                  resetDropTarget();
                  onDropFiles(root, filePath, e);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onOpen(entry);
                }}
                style={{
                  position: "absolute",
                  top: 0,
                  left: `calc(${columnIndex} * (${width} + ${GRID_GAP}px))`,
                  width,
                  height: GRID_TILE_HEIGHT,
                  transform: isDragHover
                    ? `translateY(${virtualRow.start}px) translate(18px, -14px) scale(0.98)`
                    : `translateY(${virtualRow.start}px)`,
                  display: "flex",
                  flexDirection: "column",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  padding: showThumb
                    ? "var(--space-2)"
                    : "var(--space-4) var(--space-3)",
                  border: `2px ${isBeingDragged || isDragHover ? "dashed" : "solid"} ${isDropTarget || isSelected || isBeingDragged || isDragHover ? "var(--color-accent)" : "transparent"}`,
                  borderRadius: "var(--radius-lg)",
                  background:
                    isDropTarget || isSelected || isBeingDragged || isDragHover
                      ? "var(--color-accent-muted)"
                      : "transparent",
                  boxShadow: isDragHover ? "var(--shadow-lg)" : "none",
                  zIndex: isDragHover ? 20 : undefined,
                  cursor:
                    isBeingDragged || isDragHover
                      ? "grabbing"
                      : isBeingMoved
                        ? "progress"
                        : "pointer",
                  textAlign: "center",
                  transition: `all var(--duration-fast) var(--ease-out)`,
                  animationDelay: `${Math.min(index * 20, 300)}ms`,
                  outline: "none",
                  color: isBeingMoved
                    ? "var(--color-fg-muted)"
                    : "var(--color-fg)",
                  opacity: isBeingDragged ? 0.62 : isBeingMoved ? 0.48 : 1,
                }}
                onMouseOver={(e) => {
                  if (
                    !isSelected &&
                    !isDropTarget &&
                    !isBeingMoved &&
                    !isBeingDragged &&
                    !isDragHover
                  ) {
                    e.currentTarget.style.background = "var(--color-bg-muted)";
                  }
                }}
                onMouseOut={(e) => {
                  if (
                    !isSelected &&
                    !isDropTarget &&
                    !isBeingMoved &&
                    !isBeingDragged &&
                    !isDragHover
                  ) {
                    e.currentTarget.style.background = "transparent";
                  }
                }}
              >
                {/* Icon or thumbnail */}
                {showThumb ? (
                  <ThumbnailImage root={root} path={path} entry={entry} />
                ) : (
                  <div style={{ position: "relative", display: "inline-flex" }}>
                    <FileIcon
                      svg={icon.svg}
                      color={
                        isBeingMoved ? "var(--color-fg-subtle)" : icon.color
                      }
                      size={36}
                    />
                    <span style={{ position: "absolute", right: -10, top: -8 }}>
                      <TransferProgressIndicator
                        jobs={displayedTransferJobs}
                        compact
                      />
                    </span>
                  </div>
                )}

                {/* Filename */}
                <div
                  style={{
                    width: "100%",
                    fontSize: "var(--text-sm)",
                    fontWeight: 450,
                    lineHeight: "var(--leading-sm)",
                    padding: showThumb ? "0 var(--space-1)" : 0,
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      gap: "var(--space-1)",
                      minWidth: 0,
                    }}
                  >
                    <MiddleEllipsis text={entry.name} maxWidth={120} />
                    <GalleryFeedbackBadges entry={entry} compact />
                  </div>
                </div>

                {(isBeingMoved || isBeingDragged || isDragHover) && (
                  <div
                    style={{
                      fontSize: "var(--text-xs)",
                      color: "var(--color-fg-subtle)",
                    }}
                  >
                    {isBeingDragged || isDragHover
                      ? "Dragging..."
                      : "Moving..."}
                  </div>
                )}

                {/* Size / item count */}
                {!isBeingMoved &&
                  !isBeingDragged &&
                  !isDragHover &&
                  (entry.is_dir ? (
                    entry.item_count != null && (
                      <div
                        className="tabular-nums"
                        style={{
                          fontSize: "var(--text-xs)",
                          color: "var(--color-fg-subtle)",
                        }}
                      >
                        {entry.item_count === 1
                          ? "1 item"
                          : `${entry.item_count} items`}
                      </div>
                    )
                  ) : (
                    <div
                      className="tabular-nums"
                      style={{
                        fontSize: "var(--text-xs)",
                        color: "var(--color-fg-subtle)",
                      }}
                    >
                      {formatFileSize(entry.size)}
                    </div>
                  ))}
              </button>
            );
          });
      })}
    </div>
  );
}
