interface MiddleEllipsisProps {
  text: string;
  /** Width cap for the name. Defaults to filling whatever the parent allows. */
  maxWidth?: number | string;
}

/**
 * Truncates text in the middle so file extensions remain visible.
 *
 * Pure CSS, no measurement. The name is split into a stem that is allowed to
 * shrink and ellipsize, and a suffix (the extension) pinned at its natural
 * width, so the browser's own layout does the truncation:
 *
 *     [ Some very long file na… ][ .mkv ]
 *       flex: 0 1 auto, ellipsis   flex-shrink: 0
 *
 * The previous implementation measured with a canvas: every instance created a
 * `<canvas>`, called `getComputedStyle` (which forces a synchronous layout) and
 * ran a binary search of `measureText` calls — in an effect, for every row in a
 * virtualized list. Scrolling mounts rows continuously, so that ran constantly
 * and showed up directly as scroll jank. It was also only approximate, because
 * it truncated against the `maxWidth` prop rather than the real column width.
 */
export function MiddleEllipsis({
  text,
  maxWidth = "100%",
}: MiddleEllipsisProps) {
  const { stem, suffix } = splitOnExtension(text);

  return (
    <span
      title={text}
      style={{
        display: "inline-flex",
        maxWidth,
        minWidth: 0,
        overflow: "hidden",
        whiteSpace: "nowrap",
        verticalAlign: "bottom",
      }}
    >
      <span
        style={{
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {stem}
      </span>
      {suffix && (
        <span style={{ flexShrink: 0, whiteSpace: "pre" }}>{suffix}</span>
      )}
    </span>
  );
}

/**
 * Split a filename into the part that may be truncated and the extension to
 * keep. A leading dot is part of the name, not an extension, and a name with no
 * dot has nothing to pin.
 */
function splitOnExtension(text: string): { stem: string; suffix: string } {
  const lastDot = text.lastIndexOf(".");
  if (lastDot <= 0) return { stem: text, suffix: "" };
  return { stem: text.slice(0, lastDot), suffix: text.slice(lastDot) };
}
