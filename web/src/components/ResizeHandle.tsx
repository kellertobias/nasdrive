import { useCallback, useState } from "react";

export type ResizeLimits = { min: number; max: number };

/** Which way the pane grows when the pointer moves right. */
export type ResizeDirection = 1 | -1;

const KEY_STEP = 16;
const KEY_STEP_LARGE = 48;

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), max);

/**
 * Pointer-drag resizing with a document-wide cursor/selection lock so the
 * pointer keeps its resize cursor while dragging over the pane contents, and
 * Escape restores the width the drag started from.
 */
function useResizeDrag() {
  const [resizing, setResizing] = useState(false);

  const startResize = useCallback(
    (
      e: React.PointerEvent,
      width: number,
      limits: ResizeLimits,
      setter: (width: number) => void,
      direction: ResizeDirection = 1,
    ) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = width;
      const { body } = document;
      const previousCursor = body.style.cursor;
      const previousUserSelect = body.style.userSelect;
      body.style.cursor = "col-resize";
      body.style.userSelect = "none";
      setResizing(true);

      // A pointer can report well over 100 moves a second, and each `setter`
      // call is a store write that re-renders the pane and schedules a
      // persisted width. Coalescing onto animation frames caps that at one
      // update per painted frame — the drag looks identical and costs a
      // fraction as much.
      let frame: number | null = null;
      let pendingWidth: number | null = null;

      const applyPending = () => {
        frame = null;
        if (pendingWidth === null) return;
        setter(pendingWidth);
        pendingWidth = null;
      };

      const onMove = (event: PointerEvent) => {
        pendingWidth = clamp(
          startWidth + (event.clientX - startX) * direction,
          limits.min,
          limits.max,
        );
        if (frame === null) frame = window.requestAnimationFrame(applyPending);
      };
      const stop = () => {
        // Land the last sample: the final pointermove may still be queued
        // behind a frame that never runs once listeners are gone.
        if (frame !== null) window.cancelAnimationFrame(frame);
        applyPending();
        body.style.cursor = previousCursor;
        body.style.userSelect = previousUserSelect;
        setResizing(false);
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", stop);
        window.removeEventListener("pointercancel", stop);
        window.removeEventListener("keydown", onKeyDown);
      };
      const onKeyDown = (event: KeyboardEvent) => {
        if (event.key !== "Escape") return;
        // Drop the queued sample first, or `stop`'s flush would immediately
        // re-apply the width Escape is meant to discard.
        pendingWidth = null;
        setter(startWidth);
        stop();
      };

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", stop);
      window.addEventListener("pointercancel", stop);
      window.addEventListener("keydown", onKeyDown);
    },
    [],
  );

  return { resizing, startResize };
}

/**
 * A draggable pane divider. Absolutely positioned — the nearest positioned
 * ancestor must be the pane container, and it must not be the scrolling
 * element, or the handle scrolls away with the content.
 */
export function ResizeHandle({
  label,
  value,
  limits,
  onChange,
  defaultValue,
  direction = 1,
  left,
  right,
}: {
  label: string;
  value: number;
  limits: ResizeLimits;
  onChange: (width: number) => void;
  /** Width restored on double-click. Omitted disables the reset gesture. */
  defaultValue?: number;
  direction?: ResizeDirection;
  left?: number;
  right?: number;
}) {
  const { resizing, startResize } = useResizeDrag();

  const nudge = (delta: number) =>
    onChange(clamp(value + delta * direction, limits.min, limits.max));

  const onKeyDown = (e: React.KeyboardEvent) => {
    const step = e.shiftKey ? KEY_STEP_LARGE : KEY_STEP;
    switch (e.key) {
      case "ArrowLeft":
        nudge(-step);
        break;
      case "ArrowRight":
        nudge(step);
        break;
      case "Home":
        onChange(limits.min);
        break;
      case "End":
        onChange(limits.max);
        break;
      default:
        return;
    }
    e.preventDefault();
  };

  return (
    <div
      className="resize-handle"
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={Math.round(value)}
      aria-valuemin={limits.min}
      aria-valuemax={limits.max}
      tabIndex={0}
      data-resizing={resizing || undefined}
      onPointerDown={(e) =>
        startResize(e, value, limits, onChange, direction)
      }
      onKeyDown={onKeyDown}
      onDoubleClick={
        defaultValue === undefined ? undefined : () => onChange(defaultValue)
      }
      style={{ left, right }}
    />
  );
}
