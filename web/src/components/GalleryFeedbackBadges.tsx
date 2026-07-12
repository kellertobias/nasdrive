import type { FileEntry } from "../api/client";
import { Icon } from "./Icon";

interface GalleryFeedbackBadgesProps {
  entry: FileEntry;
  compact?: boolean;
}

export function GalleryFeedbackBadges({
  entry,
  compact = false,
}: GalleryFeedbackBadgesProps) {
  const feedback = entry.gallery_feedback;
  if (!feedback?.marked && !feedback?.note) return null;

  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: compact ? 3 : "var(--space-1)",
        flexShrink: 0,
        minWidth: 0,
      }}
    >
      {feedback.marked && (
        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            minHeight: compact ? 16 : 18,
            padding: compact ? "0 5px" : "0 var(--space-1-5)",
            borderRadius: "var(--radius-sm)",
            background: "var(--color-success)",
            color: "var(--color-accent-fg)",
            fontSize: "var(--text-xs)",
            fontWeight: 700,
            lineHeight: "var(--leading-xs)",
            whiteSpace: "nowrap",
          }}
        >
          Marked
        </span>
      )}
      {feedback.note && (
        <span
          title="Has gallery note"
          aria-label="Has gallery note"
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--color-warning)",
          }}
        >
          <Icon name="fileText" size={compact ? 14 : 16} />
        </span>
      )}
    </span>
  );
}
