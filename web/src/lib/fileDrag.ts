export const NASFILES_DRAG_TYPE = "application/x-nasfiles-entries";
export const NASFILES_DEMO_DRAGGING_KEY = "nasfiles-demo-dragging";
export const NASFILES_DEMO_DRAG_HOVER_KEY = "nasfiles-demo-drag-hover";
export const NASFILES_DEMO_DROP_TARGET_KEY = "nasfiles-demo-drop-target";

export interface FileDragPayload {
  root: string;
  paths: string[];
}

export interface FileDropTarget {
  root: string;
  path: string;
}

interface FileDragPreview {
  label?: string;
  iconSvg?: string;
  iconColor?: string;
}

export function entryPath(parentPath: string, name: string) {
  return parentPath ? `${parentPath}/${name}` : name;
}

export function parentPath(path: string) {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? "" : path.slice(0, idx);
}

export function hasNasfilesDrag(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.types).includes(NASFILES_DRAG_TYPE);
}

export function hasExternalFileDrag(dataTransfer: DataTransfer) {
  return (
    Array.from(dataTransfer.types).includes("Files") ||
    Array.from(dataTransfer.items).some((item) => item.kind === "file")
  );
}

export function getExternalDropFiles(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.files).filter((file) => file.name);
}

interface NasfilesDropEvent extends Event {
  __nasfilesExternalDropHandled?: boolean;
}

export function markExternalDropHandled(event: Event) {
  (event as NasfilesDropEvent).__nasfilesExternalDropHandled = true;
}

export function isExternalDropHandled(event: Event) {
  return Boolean((event as NasfilesDropEvent).__nasfilesExternalDropHandled);
}

function basename(path: string) {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(idx + 1);
}

function buildDragPreview(
  payload: FileDragPayload,
  preview: FileDragPreview = {},
) {
  if (typeof document === "undefined") return null;

  const label = preview.label || basename(payload.paths[0] || "");
  const count = payload.paths.length;
  const element = document.createElement("div");
  element.setAttribute("aria-hidden", "true");
  element.style.cssText = [
    "position: fixed",
    "top: -1000px",
    "left: -1000px",
    "z-index: 2147483647",
    "display: flex",
    "align-items: center",
    "gap: 8px",
    "max-width: 320px",
    "height: 44px",
    "padding: 0 12px 0 10px",
    "border: 1px solid var(--color-border)",
    "border-radius: var(--radius-md)",
    "background: color-mix(in oklch, var(--color-bg) 94%, transparent)",
    "color: var(--color-fg)",
    "box-shadow: var(--shadow-lg)",
    "font: 500 var(--text-sm)/var(--leading-sm) var(--font-sans)",
    "pointer-events: none",
  ].join(";");

  const icon = document.createElement("span");
  icon.style.cssText = [
    "display: inline-flex",
    "align-items: center",
    "justify-content: center",
    "width: 24px",
    "height: 24px",
    "flex: 0 0 auto",
    `color: ${preview.iconColor || "var(--color-accent)"}`,
  ].join(";");
  if (preview.iconSvg) {
    icon.innerHTML = preview.iconSvg;
  } else {
    icon.textContent = count > 1 ? String(count) : "";
  }
  element.appendChild(icon);

  const text = document.createElement("span");
  text.textContent = count > 1 ? `${label} + ${count - 1} more` : label;
  text.style.cssText = [
    "min-width: 0",
    "max-width: 220px",
    "overflow: hidden",
    "text-overflow: ellipsis",
    "white-space: nowrap",
  ].join(";");
  element.appendChild(text);

  if (count > 1) {
    const badge = document.createElement("span");
    badge.textContent = String(count);
    badge.style.cssText = [
      "display: inline-flex",
      "align-items: center",
      "justify-content: center",
      "min-width: 20px",
      "height: 20px",
      "padding: 0 6px",
      "border-radius: var(--radius-full)",
      "background: var(--color-accent)",
      "color: var(--color-accent-fg)",
      "font: 600 var(--text-xs)/1 var(--font-sans)",
      "font-variant-numeric: tabular-nums",
    ].join(";");
    element.appendChild(badge);
  }

  document.body.appendChild(element);
  return element;
}

export function setFileDragPayload(
  dataTransfer: DataTransfer,
  payload: FileDragPayload,
  preview?: FileDragPreview,
) {
  const encoded = JSON.stringify(payload);
  dataTransfer.effectAllowed = "copyMove";
  dataTransfer.setData(NASFILES_DRAG_TYPE, encoded);
  dataTransfer.setData("text/plain", encoded);

  const previewElement = buildDragPreview(payload, preview);
  if (previewElement) {
    dataTransfer.setDragImage(previewElement, 20, 22);
    window.setTimeout(() => previewElement.remove(), 0);
  }
}

export function getFileDragPayload(
  dataTransfer: DataTransfer,
): FileDragPayload | null {
  const raw =
    dataTransfer.getData(NASFILES_DRAG_TYPE) ||
    dataTransfer.getData("text/plain");
  if (!raw) return null;

  try {
    const parsed = JSON.parse(raw) as Partial<FileDragPayload>;
    if (
      typeof parsed.root === "string" &&
      Array.isArray(parsed.paths) &&
      parsed.paths.every((path) => typeof path === "string")
    ) {
      return { root: parsed.root, paths: parsed.paths };
    }
  } catch {
    return null;
  }

  return null;
}

function readJsonLocalStorage<T>(key: string): T | null {
  if (typeof window === "undefined") return null;

  const raw = window.localStorage.getItem(key);
  if (!raw) return null;

  try {
    return JSON.parse(raw) as T;
  } catch {
    return null;
  }
}

export function getDemoDraggingPayload() {
  const parsed = readJsonLocalStorage<Partial<FileDragPayload>>(
    NASFILES_DEMO_DRAGGING_KEY,
  );
  if (
    parsed &&
    typeof parsed.root === "string" &&
    Array.isArray(parsed.paths) &&
    parsed.paths.every((path) => typeof path === "string")
  ) {
    return { root: parsed.root, paths: parsed.paths };
  }
  return null;
}

export function getDemoDropTarget() {
  const parsed = readJsonLocalStorage<Partial<FileDropTarget>>(
    NASFILES_DEMO_DROP_TARGET_KEY,
  );
  if (
    parsed &&
    typeof parsed.root === "string" &&
    typeof parsed.path === "string"
  ) {
    return { root: parsed.root, path: parsed.path };
  }
  return null;
}

export function isDemoDraggedPath(root: string, path: string) {
  const payload = getDemoDraggingPayload();
  return Boolean(
    payload && payload.root === root && payload.paths.includes(path),
  );
}

export function getDemoDragHoverPayload() {
  const parsed = readJsonLocalStorage<Partial<FileDragPayload>>(
    NASFILES_DEMO_DRAG_HOVER_KEY,
  );
  if (
    parsed &&
    typeof parsed.root === "string" &&
    Array.isArray(parsed.paths) &&
    parsed.paths.every((path) => typeof path === "string")
  ) {
    return { root: parsed.root, paths: parsed.paths };
  }
  return null;
}

export function isDemoDragHoverPath(root: string, path: string) {
  const payload = getDemoDragHoverPayload();
  return Boolean(
    payload && payload.root === root && payload.paths.includes(path),
  );
}

export function isDemoDropTarget(root: string, path: string) {
  const target = getDemoDropTarget();
  return Boolean(target && target.root === root && target.path === path);
}

export function isSelfOrDescendantDrop(
  payload: FileDragPayload,
  targetRoot: string,
  targetPath: string,
) {
  if (payload.root !== targetRoot) return false;
  return payload.paths.some(
    (path) => targetPath === path || targetPath.startsWith(`${path}/`),
  );
}
