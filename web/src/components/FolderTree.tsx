import { useQuery } from "@tanstack/react-query";
import api from "../api/client";
import type { Root, FileEntry, CustomLink } from "../api/client";
import { useCallback, useEffect, useState } from "react";
import { Icon } from "./Icon";
import { ICONS } from "../lib/icons";
import {
  hasNasfilesDrag,
  isDemoDraggedPath,
  isDemoDropTarget,
} from "../lib/fileDrag";
import { useGlobalDragCleanup } from "../lib/dragState";
import { UsageRing } from "./UsageRing";
import type { TransferJob } from "../api/client";
import { TransferProgressIndicator } from "./TransferProgressIndicator";
import {
  moveJobsForSourcePath,
  transferJobsForTarget,
} from "../lib/transferJobs";

interface FolderTreeProps {
  roots: Root[];
  activeRoot: string;
  activePath: string;
  onNavigate: (root: string, path: string) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs?: TransferJob[];
  customLinks?: CustomLink[];
}

interface ShareGroup {
  name: string;
  roots: Root[];
}

/// Split the flat roots list into ungrouped roots followed by named groups,
/// preserving the backend's ordering (ungrouped first, each group's members
/// contiguous). A group only appears when it has at least one visible root.
function partitionRoots(roots: Root[]): {
  ungrouped: Root[];
  groups: ShareGroup[];
} {
  const ungrouped: Root[] = [];
  const groups: ShareGroup[] = [];
  const indexByName = new Map<string, number>();
  for (const root of roots) {
    if (!root.group) {
      ungrouped.push(root);
      continue;
    }
    let idx = indexByName.get(root.group);
    if (idx === undefined) {
      idx = groups.length;
      indexByName.set(root.group, idx);
      groups.push({ name: root.group, roots: [] });
    }
    groups[idx].roots.push(root);
  }
  return { ungrouped, groups };
}

export function FolderTree({
  roots,
  activeRoot,
  activePath,
  onNavigate,
  onDropFiles,
  transferJobs = [],
  customLinks = [],
}: FolderTreeProps) {
  const { ungrouped, groups } = partitionRoots(roots);
  const renderRoot = (root: Root) => (
    <TreeRoot
      key={root.key}
      root={root}
      isActive={root.key === activeRoot}
      activePath={root.key === activeRoot ? activePath : ""}
      onNavigate={onNavigate}
      onDropFiles={onDropFiles}
      transferJobs={transferJobs}
    />
  );

  return (
    <nav
      role="tree"
      aria-label="Folder tree"
      style={{ userSelect: "none", width: "100%", overflow: "hidden" }}
    >
      {ungrouped.map(renderRoot)}
      {groups.map((group) => (
        <TreeGroup
          key={group.name}
          name={group.name}
          roots={group.roots}
          activeRoot={activeRoot}
          activePath={activePath}
          onNavigate={onNavigate}
          onDropFiles={onDropFiles}
          transferJobs={transferJobs}
        />
      ))}
      {customLinks.length > 0 && (
        <div
          style={{
            marginTop: "var(--space-4)",
            paddingTop: "var(--space-2)",
            borderTop: "1px solid var(--color-border)",
          }}
        >
          {customLinks.map((link) => (
            <CustomLinkItem key={link.url} link={link} />
          ))}
        </div>
      )}
    </nav>
  );
}

interface TreeGroupProps {
  name: string;
  roots: Root[];
  activeRoot: string;
  activePath: string;
  onNavigate: (root: string, path: string) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs: TransferJob[];
}

/// A collapsible sidebar section that groups several shares. The header is not
/// navigable and is not a drop target — a group is an organizational overlay,
/// never a filesystem location, so files can only be created or moved into the
/// shares inside it, not into the group itself.
function TreeGroup({
  name,
  roots,
  activeRoot,
  activePath,
  onNavigate,
  onDropFiles,
  transferJobs,
}: TreeGroupProps) {
  const containsActive = roots.some((root) => root.key === activeRoot);
  const [expanded, setExpanded] = useState(containsActive);

  // Auto-expand when the active share moves into this group (never auto-collapse).
  useEffect(() => {
    if (containsActive) setExpanded(true);
  }, [containsActive]);

  return (
    <div role="treeitem" aria-expanded={expanded}>
      <button
        onClick={() => setExpanded((current) => !current)}
        onKeyDown={(e) => {
          if (e.key === "ArrowRight") {
            setExpanded(true);
            e.preventDefault();
          }
          if (e.key === "ArrowLeft") {
            setExpanded(false);
            e.preventDefault();
          }
        }}
        title={name}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          width: "100%",
          minWidth: 0,
          boxSizing: "border-box",
          padding: "var(--space-1-5) var(--space-4)",
          border: "none",
          background: "transparent",
          color: "var(--color-sidebar-fg)",
          cursor: "pointer",
          fontSize: "var(--text-sm)",
          fontWeight: containsActive ? 600 : 500,
          textAlign: "left",
          borderRadius: 0,
          transition: `all var(--duration-fast) var(--ease-out)`,
        }}
        onMouseOver={(e) => {
          e.currentTarget.style.background = "var(--color-sidebar-hover)";
        }}
        onMouseOut={(e) => {
          e.currentTarget.style.background = "transparent";
        }}
      >
        <span
          style={{
            display: "inline-flex",
            transition: `transform var(--duration-fast) var(--ease-out)`,
            transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
            color: "var(--color-fg-subtle)",
          }}
        >
          <Icon name="chevronRight" size={14} />
        </span>
        <Icon name="folders" size={16} color="var(--color-fg-muted)" />
        <span
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {name}
        </span>
      </button>

      {expanded && (
        <div
          role="group"
          style={{
            paddingLeft: "var(--space-3)",
            boxSizing: "border-box",
            maxWidth: "100%",
            overflow: "hidden",
          }}
          className="slide-in"
        >
          {roots.map((root) => (
            <TreeRoot
              key={root.key}
              root={root}
              isActive={root.key === activeRoot}
              activePath={root.key === activeRoot ? activePath : ""}
              onNavigate={onNavigate}
              onDropFiles={onDropFiles}
              transferJobs={transferJobs}
            />
          ))}
        </div>
      )}
    </div>
  );
}

interface TreeRootProps {
  root: Root;
  isActive: boolean;
  activePath: string;
  onNavigate: (root: string, path: string) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs: TransferJob[];
}

function TreeRoot({
  root,
  isActive,
  activePath,
  onNavigate,
  onDropFiles,
  transferJobs,
}: TreeRootProps) {
  const [expanded, setExpanded] = useState(isActive);
  const [isDropTarget, setIsDropTarget] = useState(false);
  const resetDropTarget = useCallback(() => setIsDropTarget(false), []);
  const isRootActive = isActive && activePath === "";
  const isDemoRootDropTarget = isDemoDropTarget(root.key, "");
  const rootTransferJobs = transferJobsForTarget(transferJobs, root.key, "");

  useEffect(() => {
    if (isActive) setExpanded(true);
  }, [isActive]);

  useGlobalDragCleanup(resetDropTarget);

  const handleClick = () => {
    setExpanded((current) => (isRootActive ? !current : true));
    onNavigate(root.key, "");
  };

  return (
    <div role="treeitem" aria-expanded={expanded}>
      <button
        onClick={handleClick}
        onDragEnter={(e) => {
          if (
            !root.caps.write ||
            !onDropFiles ||
            !hasNasfilesDrag(e.dataTransfer)
          )
            return;
          e.preventDefault();
          e.stopPropagation();
          setExpanded(true);
          setIsDropTarget(true);
        }}
        onDragOver={(e) => {
          if (
            !root.caps.write ||
            !onDropFiles ||
            !hasNasfilesDrag(e.dataTransfer)
          )
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
          if (
            !root.caps.write ||
            !onDropFiles ||
            !hasNasfilesDrag(e.dataTransfer)
          )
            return;
          e.preventDefault();
          e.stopPropagation();
          resetDropTarget();
          onDropFiles(root.key, "", e);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowRight") {
            setExpanded(true);
            e.preventDefault();
          }
          if (e.key === "ArrowLeft") {
            setExpanded(false);
            e.preventDefault();
          }
          if (e.key === "Enter") handleClick();
        }}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          width: "100%",
          minWidth: 0,
          boxSizing: "border-box",
          padding: "var(--space-1-5) var(--space-4)",
          border: "none",
          background:
            isDropTarget || isDemoRootDropTarget || isRootActive
              ? "var(--color-sidebar-active)"
              : "transparent",
          color:
            isRootActive || isDemoRootDropTarget
              ? "var(--color-accent)"
              : "var(--color-sidebar-fg)",
          cursor: "pointer",
          fontSize: "var(--text-sm)",
          fontWeight: isRootActive ? 600 : 400,
          textAlign: "left",
          borderRadius: 0,
          transition: `all var(--duration-fast) var(--ease-out)`,
        }}
        onMouseOver={(e) => {
          if (!isRootActive && !isDropTarget && !isDemoRootDropTarget)
            e.currentTarget.style.background = "var(--color-sidebar-hover)";
        }}
        onMouseOut={(e) => {
          if (!isRootActive && !isDropTarget && !isDemoRootDropTarget)
            e.currentTarget.style.background = "transparent";
        }}
      >
        <span
          style={{
            display: "inline-flex",
            transition: `transform var(--duration-fast) var(--ease-out)`,
            transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
            color: "var(--color-fg-subtle)",
          }}
        >
          <Icon name="chevronRight" size={14} />
        </span>
        <Icon
          name={root.kind === "home" ? "home" : "folder"}
          size={16}
          color={isRootActive ? "var(--color-accent)" : "var(--color-fg-muted)"}
        />
        <span
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {root.display_name}
        </span>
        {rootTransferJobs.length > 0 ? (
          <TransferProgressIndicator jobs={rootTransferJobs} compact />
        ) : (
          <UsageRing usage={root.usage} />
        )}
      </button>

      {expanded && (
        <div
          role="group"
          style={{
            paddingLeft: "var(--space-4)",
            boxSizing: "border-box",
            maxWidth: "100%",
            overflow: "hidden",
          }}
          className="slide-in"
        >
          <TreeChildren
            rootKey={root.key}
            path=""
            activePath={activePath}
            onNavigate={onNavigate}
            onDropFiles={root.caps.write ? onDropFiles : undefined}
            transferJobs={transferJobs}
            depth={1}
          />
        </div>
      )}
    </div>
  );
}

interface TreeChildrenProps {
  rootKey: string;
  path: string;
  activePath: string;
  onNavigate: (root: string, path: string) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs: TransferJob[];
  depth: number;
}

function TreeChildren({
  rootKey,
  path,
  activePath,
  onNavigate,
  onDropFiles,
  transferJobs,
  depth,
}: TreeChildrenProps) {
  const { data, isLoading } = useQuery({
    queryKey: ["tree", rootKey, path],
    queryFn: () => api.listTree(rootKey, path),
    staleTime: 30_000,
  });

  if (isLoading) {
    return (
      <div style={{ padding: "var(--space-1) var(--space-4)" }}>
        {[1, 2].map((i) => (
          <div
            key={i}
            className="shimmer"
            style={{
              height: 20,
              marginBottom: "var(--space-1)",
              borderRadius: "var(--radius-sm)",
            }}
          />
        ))}
      </div>
    );
  }

  if (!data?.children?.length) return null;

  return (
    <>
      {data.children.map((child) => (
        <TreeNode
          key={child.name}
          rootKey={rootKey}
          entry={child}
          parentPath={path}
          activePath={activePath}
          onNavigate={onNavigate}
          onDropFiles={onDropFiles}
          transferJobs={transferJobs}
          depth={depth}
        />
      ))}
    </>
  );
}

interface TreeNodeProps {
  rootKey: string;
  entry: FileEntry;
  parentPath: string;
  activePath: string;
  onNavigate: (root: string, path: string) => void;
  onDropFiles?: (
    targetRoot: string,
    targetPath: string,
    e: React.DragEvent,
  ) => void;
  transferJobs: TransferJob[];
  depth: number;
}

function CustomLinkItem({ link }: { link: CustomLink }) {
  const iconName =
    link.icon in ICONS ? (link.icon as keyof typeof ICONS) : "link";
  return (
    <a
      href={link.url}
      target="_blank"
      rel="noopener noreferrer"
      title={link.name}
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        width: "100%",
        minWidth: 0,
        boxSizing: "border-box",
        padding: "var(--space-1-5) var(--space-4)",
        color: "var(--color-sidebar-fg)",
        fontSize: "var(--text-sm)",
        fontWeight: 400,
        textDecoration: "none",
        borderRadius: 0,
        transition: `background var(--duration-fast) var(--ease-out)`,
      }}
      onMouseOver={(e) => {
        e.currentTarget.style.background = "var(--color-sidebar-hover)";
      }}
      onMouseOut={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      <Icon
        name={iconName}
        size={16}
        color={link.icon_color || "var(--color-fg-muted)"}
      />
      <span
        style={{
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {link.name}
      </span>
    </a>
  );
}

function TreeNode({
  rootKey,
  entry,
  parentPath,
  activePath,
  onNavigate,
  onDropFiles,
  transferJobs,
  depth,
}: TreeNodeProps) {
  const fullPath = parentPath ? `${parentPath}/${entry.name}` : entry.name;
  const isActive = activePath === fullPath;
  const isInActiveLine = activePath.startsWith(fullPath + "/");
  const [expanded, setExpanded] = useState(isInActiveLine);
  const [isDropTarget, setIsDropTarget] = useState(false);
  const resetDropTarget = useCallback(() => setIsDropTarget(false), []);
  const isDemoNodeDropTarget = isDemoDropTarget(rootKey, fullPath);
  const isBeingDragged = isDemoDraggedPath(rootKey, fullPath);
  const nodeTransferJobs = transferJobsForTarget(
    transferJobs,
    rootKey,
    fullPath,
  );
  const sourceMoveJobs = moveJobsForSourcePath(transferJobs, rootKey, fullPath);
  const isBeingMoved = sourceMoveJobs.length > 0;
  const displayedTransferJobs = isBeingMoved
    ? sourceMoveJobs
    : nodeTransferJobs;

  const handleClick = () => {
    setExpanded(!expanded);
    onNavigate(rootKey, fullPath);
  };

  // Auto-expand if the active path is within this node
  if (isInActiveLine && !expanded) {
    setExpanded(true);
  }

  useGlobalDragCleanup(resetDropTarget);

  return (
    <div role="treeitem" aria-expanded={entry.is_dir ? expanded : undefined}>
      <button
        onClick={handleClick}
        onDragEnter={(e) => {
          if (!onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
          e.preventDefault();
          e.stopPropagation();
          setExpanded(true);
          setIsDropTarget(true);
        }}
        onDragOver={(e) => {
          if (!onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
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
          if (!onDropFiles || !hasNasfilesDrag(e.dataTransfer)) return;
          e.preventDefault();
          e.stopPropagation();
          resetDropTarget();
          onDropFiles(rootKey, fullPath, e);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowRight") {
            setExpanded(true);
            e.preventDefault();
          }
          if (e.key === "ArrowLeft") {
            setExpanded(false);
            e.preventDefault();
          }
          if (e.key === "Enter") handleClick();
        }}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          width: "100%",
          minWidth: 0,
          boxSizing: "border-box",
          padding: "var(--space-1) var(--space-3)",
          border: "none",
          background:
            isDropTarget || isDemoNodeDropTarget || isActive || isBeingDragged
              ? "var(--color-sidebar-active)"
              : "transparent",
          color:
            isBeingMoved || isBeingDragged
              ? "var(--color-fg-muted)"
              : isActive || isDemoNodeDropTarget
                ? "var(--color-accent)"
                : "var(--color-sidebar-fg)",
          cursor: isBeingDragged
            ? "grabbing"
            : isBeingMoved
              ? "progress"
              : "pointer",
          fontSize: "var(--text-sm)",
          fontWeight: isActive ? 500 : 400,
          textAlign: "left",
          borderRadius: "var(--radius-sm)",
          transition: `all var(--duration-fast) var(--ease-out)`,
          opacity: isBeingDragged ? 0.62 : isBeingMoved ? 0.48 : 1,
        }}
        onMouseOver={(e) => {
          if (
            !isActive &&
            !isDropTarget &&
            !isDemoNodeDropTarget &&
            !isBeingMoved &&
            !isBeingDragged
          )
            e.currentTarget.style.background = "var(--color-sidebar-hover)";
        }}
        onMouseOut={(e) => {
          if (
            !isActive &&
            !isDropTarget &&
            !isDemoNodeDropTarget &&
            !isBeingMoved &&
            !isBeingDragged
          )
            e.currentTarget.style.background = "transparent";
        }}
      >
        <span
          style={{
            display: "inline-flex",
            transition: `transform var(--duration-fast) var(--ease-out)`,
            transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
            color: "var(--color-fg-subtle)",
            visibility: entry.is_dir ? "visible" : "hidden",
          }}
        >
          <Icon name="chevronRight" size={12} />
        </span>
        <Icon
          name="folder"
          size={14}
          color={
            isBeingMoved || isBeingDragged
              ? "var(--color-fg-subtle)"
              : isActive || isDemoNodeDropTarget
                ? "var(--color-accent)"
                : "var(--color-fg-muted)"
          }
        />
        <span
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {entry.name}
        </span>
        {(isBeingMoved || isBeingDragged) && (
          <span
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--color-fg-subtle)",
            }}
          >
            {isBeingDragged ? "Dragging..." : "Moving..."}
          </span>
        )}
        <TransferProgressIndicator jobs={displayedTransferJobs} compact />
      </button>

      {expanded && depth < 8 && (
        <div
          role="group"
          style={{
            paddingLeft: "var(--space-3)",
            boxSizing: "border-box",
            maxWidth: "100%",
            overflow: "hidden",
          }}
          className="slide-in"
        >
          <TreeChildren
            rootKey={rootKey}
            path={fullPath}
            activePath={activePath}
            onNavigate={onNavigate}
            onDropFiles={onDropFiles}
            transferJobs={transferJobs}
            depth={depth + 1}
          />
        </div>
      )}
    </div>
  );
}
