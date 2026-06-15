import type { FileEntry } from '../api/client';
import { getFileIcon, formatFileSize, formatModifiedDate } from '../lib/icons';
import { useViewStore } from '../state/view';
import { MiddleEllipsis } from './MiddleEllipsis';
import { FileIcon } from './Icon';
import { useCallback, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { entryPath, hasNasfilesDrag, isDemoDraggedPath, isDemoDragHoverPath, isDemoDropTarget, setFileDragPayload } from '../lib/fileDrag';
import { useGlobalDragCleanup } from '../lib/dragState';
import type { TransferJob } from '../api/client';
import { TransferProgressIndicator } from './TransferProgressIndicator';
import { incomingTransferPlaceholders, moveJobsForSourcePath, transferJobsForTarget, transferProgressPercent } from '../lib/transferJobs';

interface FileListProps {
  entries: FileEntry[];
  onOpen: (entry: FileEntry) => void;
  root?: string;
  path: string;
  scrollParentRef?: React.RefObject<HTMLElement | null>;
  onContextMenu?: (e: React.MouseEvent, entry: FileEntry) => void;
  onDropFiles?: (targetRoot: string, targetPath: string, e: React.DragEvent) => void;
  transferJobs?: TransferJob[];
}

const LIST_ROW_HEIGHT = 38;

type ListItem =
  | { kind: 'entry'; entry: FileEntry }
  | { kind: 'placeholder'; placeholder: ReturnType<typeof incomingTransferPlaceholders>[number] };

export function FileList({ entries, onOpen, root = '', path, scrollParentRef, onContextMenu, onDropFiles, transferJobs = [] }: FileListProps) {
  const { selectedPaths, select, toggleSelect, rangeSelect, clearSelection, sortField, setSortField, sortDirection } = useViewStore();
  const lastClickedIndex = useRef<number>(-1);
  const listRef = useRef<HTMLDivElement>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const resetDropTarget = useCallback(() => setDropTarget(null), []);
  const transferPlaceholders = incomingTransferPlaceholders(
    transferJobs,
    root,
    path,
    entries.map((entry) => entry.name),
  );

  const sorted = useMemo(() => [...entries].sort((a, b) => {
    // Always keep dirs first
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;

    let cmp = 0;
    switch (sortField) {
      case 'name':
        cmp = a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });
        break;
      case 'size':
        cmp = a.size - b.size;
        break;
      case 'modified_at':
        cmp = a.modified_at - b.modified_at;
        break;
    }
    return sortDirection === 'asc' ? cmp : -cmp;
  }), [entries, sortDirection, sortField]);
  const items = useMemo<ListItem[]>(
    () => [
      ...sorted.map((entry) => ({ kind: 'entry' as const, entry })),
      ...transferPlaceholders.map((placeholder) => ({ kind: 'placeholder' as const, placeholder })),
    ],
    [sorted, transferPlaceholders],
  );
  // eslint-disable-next-line react-hooks/incompatible-library
  const rowVirtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollParentRef?.current ?? listRef.current,
    estimateSize: () => LIST_ROW_HEIGHT,
    overscan: 12,
  });

  useGlobalDragCleanup(resetDropTarget);

  const columnHeaderStyle = (field: string): React.CSSProperties => ({
    padding: 'var(--space-2) var(--space-3)',
    background: 'transparent',
    border: 'none',
    cursor: 'pointer',
    fontSize: 'var(--text-xs)',
    fontWeight: 600,
    textTransform: 'uppercase' as const,
    letterSpacing: 'var(--tracking-wide)',
    color: sortField === field ? 'var(--color-accent)' : 'var(--color-fg-subtle)',
    textAlign: 'left' as const,
    userSelect: 'none' as const,
    transition: `color var(--duration-fast) var(--ease-out)`,
  });

  return (
    <div
      ref={listRef}
      role="grid"
      aria-label="Files"
      onClick={(e) => { if (e.target === e.currentTarget) clearSelection(); }}
    >
      {/* Column headers */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: '1fr 100px 140px',
        borderBottom: '1px solid var(--color-border)',
        marginBottom: 'var(--space-1)',
      }}>
        <button style={columnHeaderStyle('name')} onClick={() => setSortField('name')}>
          Name {sortField === 'name' && (sortDirection === 'asc' ? '↑' : '↓')}
        </button>
        <button style={columnHeaderStyle('size')} onClick={() => setSortField('size')}>
          Size {sortField === 'size' && (sortDirection === 'asc' ? '↑' : '↓')}
        </button>
        <button style={columnHeaderStyle('modified_at')} onClick={() => setSortField('modified_at')}>
          Modified {sortField === 'modified_at' && (sortDirection === 'asc' ? '↑' : '↓')}
        </button>
      </div>

      <div style={{
        position: 'relative',
        height: rowVirtualizer.getTotalSize(),
      }}>
      {rowVirtualizer.getVirtualItems().map((virtualRow) => {
        const item = items[virtualRow.index];
        const index = virtualRow.index;

        if (item.kind === 'placeholder') {
          const { placeholder } = item;
          const percent = transferProgressPercent([placeholder.job]);
          const icon = getFileIcon({
            name: placeholder.name,
            size: 0,
            modified_at: 0,
            is_dir: false,
            mime_type: null,
            has_thumbnail: false,
          });

          return (
            <div
              key={placeholder.key}
              role="row"
              aria-disabled="true"
              style={{
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                height: LIST_ROW_HEIGHT,
                transform: `translateY(${virtualRow.start}px)`,
                display: 'grid',
                gridTemplateColumns: '1fr 100px 140px',
                alignItems: 'center',
                padding: 'var(--space-1-5) 0',
                borderRadius: 'var(--radius-md)',
                background: 'transparent',
                opacity: 0.58,
                cursor: 'default',
                outline: 'none',
                pointerEvents: 'none',
              }}
            >
              <div style={{
                display: 'flex',
                alignItems: 'center',
                gap: 'var(--space-2)',
                padding: '0 var(--space-3)',
                minWidth: 0,
              }}>
                <FileIcon svg={icon.svg} color="var(--color-fg-subtle)" size={18} />
                <span style={{
                  flex: 1,
                  minWidth: 0,
                  fontSize: 'var(--text-sm)',
                  fontWeight: 400,
                  color: 'var(--color-fg-muted)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}>
                  <MiddleEllipsis text={placeholder.name} maxWidth={400} />
                </span>
              </div>

              <div style={{
                padding: '0 var(--space-3)',
                display: 'flex',
                alignItems: 'center',
                gap: 'var(--space-1)',
              }}>
                <div style={{
                  flex: 1,
                  height: 4,
                  minWidth: 36,
                  borderRadius: 2,
                  background: 'var(--color-border)',
                  overflow: 'hidden',
                }}>
                  <div style={{
                    width: `${percent}%`,
                    height: '100%',
                    borderRadius: 2,
                    background: 'var(--color-accent)',
                    transition: 'width 200ms ease-out',
                  }} />
                </div>
                <span className="tabular-nums" style={{ fontSize: 'var(--text-xs)', color: 'var(--color-fg-subtle)' }}>
                  {percent}%
                </span>
              </div>

              <div className="tabular-nums" style={{
                fontSize: 'var(--text-sm)',
                color: 'var(--color-fg-muted)',
                padding: '0 var(--space-3)',
              }}>
                —
              </div>
            </div>
          );
        }

        const { entry } = item;
        const filePath = entryPath(path, entry.name);
        const isSelected = selectedPaths.has(filePath);
        const icon = getFileIcon(entry);
        const isDropTarget = dropTarget === filePath || isDemoDropTarget(root, filePath);
        const isBeingDragged = isDemoDraggedPath(root, filePath);
        const isDragHover = isDemoDragHoverPath(root, filePath);
        const entryTransferJobs = entry.is_dir ? transferJobsForTarget(transferJobs, root, filePath) : [];
        const sourceMoveJobs = entry.is_dir ? moveJobsForSourcePath(transferJobs, root, filePath) : [];
        const isBeingMoved = sourceMoveJobs.length > 0;
        const displayedTransferJobs = isBeingMoved ? sourceMoveJobs : entryTransferJobs;

        return (
          <div
            key={entry.name}
            role="row"
            aria-selected={isSelected}
            aria-busy={isBeingMoved || undefined}
            draggable={Boolean(root)}
            onDragStart={(e) => {
              if (!root) return;
              const visiblePaths = sorted.map((en) => entryPath(path, en.name));
              const paths = selectedPaths.has(filePath)
                ? visiblePaths.filter((visiblePath) => selectedPaths.has(visiblePath))
                : [filePath];
              if (!selectedPaths.has(filePath)) select(filePath);
              setFileDragPayload(e.dataTransfer, { root, paths }, {
                label: entry.name,
                iconSvg: icon.svg,
                iconColor: icon.color,
              });
            }}
            onDragEnd={resetDropTarget}
            onClick={(e) => {
              if (e.shiftKey && lastClickedIndex.current >= 0) {
                const lo = Math.min(lastClickedIndex.current, index);
                const hi = Math.max(lastClickedIndex.current, index);
                const rangePaths = sorted
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
              if (!entry.is_dir || !root || !onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
              e.preventDefault();
              e.stopPropagation();
              setDropTarget(filePath);
            }}
            onDragOver={(e) => {
              if (!entry.is_dir || !root || !onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
              e.preventDefault();
              e.stopPropagation();
              e.dataTransfer.dropEffect = e.dataTransfer.effectAllowed === 'copy' ? 'copy' : 'move';
            }}
            onDragLeave={(e) => {
              if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
              resetDropTarget();
            }}
            onDrop={(e) => {
              if (!entry.is_dir || !root || !onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
              e.preventDefault();
              e.stopPropagation();
              resetDropTarget();
              onDropFiles(root, filePath, e);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') onOpen(entry);
            }}
            tabIndex={0}
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              right: 0,
              height: LIST_ROW_HEIGHT,
              transform: isDragHover
                ? `translateY(${virtualRow.start}px) translate(18px, -10px) scale(0.99)`
                : `translateY(${virtualRow.start}px)`,
              display: 'grid',
              gridTemplateColumns: '1fr 100px 140px',
              alignItems: 'center',
              padding: 'var(--space-1-5) 0',
              borderRadius: 'var(--radius-md)',
              background: isDropTarget || isSelected || isBeingDragged || isDragHover ? 'var(--color-accent-muted)' : 'transparent',
              border: isBeingDragged || isDragHover ? '1px dashed var(--color-accent)' : '1px solid transparent',
              boxShadow: isDragHover ? 'var(--shadow-lg)' : 'none',
              zIndex: isDragHover ? 20 : undefined,
              cursor: isBeingDragged || isDragHover ? 'grabbing' : isBeingMoved ? 'progress' : 'pointer',
              transition: `all var(--duration-fast) var(--ease-out)`,
              animationDelay: `${Math.min(index * 10, 200)}ms`,
              outline: 'none',
              opacity: isBeingDragged ? 0.62 : isBeingMoved ? 0.48 : 1,
            }}
            onMouseOver={(e) => {
              if (!isSelected && !isDropTarget && !isBeingMoved && !isBeingDragged && !isDragHover) e.currentTarget.style.background = 'var(--color-bg-muted)';
            }}
            onMouseOut={(e) => {
              if (!isSelected && !isDropTarget && !isBeingMoved && !isBeingDragged && !isDragHover) e.currentTarget.style.background = 'transparent';
            }}
          >
            <div style={{
              display: 'flex',
              alignItems: 'center',
              gap: 'var(--space-2)',
              padding: '0 var(--space-3)',
              minWidth: 0,
            }}>
              <FileIcon svg={icon.svg} color={isBeingMoved ? 'var(--color-fg-subtle)' : icon.color} size={18} />
              <span style={{
                flex: 1,
                minWidth: 0,
                fontSize: 'var(--text-sm)',
                fontWeight: entry.is_dir ? 500 : 400,
                color: isBeingMoved || isBeingDragged || isDragHover ? 'var(--color-fg-muted)' : 'var(--color-fg)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}>
                <MiddleEllipsis text={entry.name} maxWidth={400} />
              </span>
              <TransferProgressIndicator jobs={displayedTransferJobs} compact />
            </div>

            <div className="tabular-nums" style={{
              fontSize: 'var(--text-sm)',
              color: 'var(--color-fg-muted)',
              padding: '0 var(--space-3)',
            }}>
              {isBeingDragged || isDragHover ? 'Dragging...' : isBeingMoved ? 'Moving...' : entry.is_dir ? '—' : formatFileSize(entry.size)}
            </div>

            <div className="tabular-nums" style={{
              fontSize: 'var(--text-sm)',
              color: 'var(--color-fg-muted)',
              padding: '0 var(--space-3)',
            }}>
              {formatModifiedDate(entry.modified_at)}
            </div>
          </div>
        );
      })}
      </div>
    </div>
  );
}
