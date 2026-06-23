import { useRef, useEffect, useState } from "react";

interface MiddleEllipsisProps {
  text: string;
  maxWidth?: number;
}

/**
 * Truncates text in the middle so file extensions remain visible.
 * Falls back to CSS ellipsis if canvas measurement isn't available.
 */
export function MiddleEllipsis({ text, maxWidth = 200 }: MiddleEllipsisProps) {
  const containerRef = useRef<HTMLSpanElement>(null);
  const [displayText, setDisplayText] = useState(text);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const computedStyle = getComputedStyle(container);
    ctx.font = `${computedStyle.fontSize} ${computedStyle.fontFamily}`;

    const fullWidth = ctx.measureText(text).width;
    const containerWidth = maxWidth;

    if (fullWidth <= containerWidth) {
      setDisplayText(text);
      return;
    }

    const ellipsis = "…";
    const ellipsisWidth = ctx.measureText(ellipsis).width;
    const availableWidth = containerWidth - ellipsisWidth;

    // Find the extension (last dot and everything after)
    const lastDot = text.lastIndexOf(".");
    let start = text;
    let end = "";

    if (lastDot > 0) {
      end = text.substring(lastDot);
      start = text.substring(0, lastDot);
    }

    const endWidth = ctx.measureText(end).width;
    const startAvailable = availableWidth - endWidth;

    if (startAvailable <= 0) {
      setDisplayText(text);
      return;
    }

    // Binary search for the right truncation point
    let lo = 0;
    let hi = start.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      const substr = start.substring(0, mid);
      if (ctx.measureText(substr).width <= startAvailable) {
        lo = mid;
      } else {
        hi = mid - 1;
      }
    }

    if (lo < start.length) {
      setDisplayText(`${start.substring(0, lo)}${ellipsis}${end}`);
    } else {
      setDisplayText(text);
    }
  }, [text, maxWidth]);

  return (
    <span
      ref={containerRef}
      title={text}
      style={{
        display: "inline-block",
        maxWidth,
        overflow: "hidden",
        whiteSpace: "nowrap",
      }}
    >
      {displayText}
    </span>
  );
}
