import { useEffect, useState } from "react";
import type { FileEntry } from "../api/client";
import api from "../api/client";
import {
  formatFileSize,
  formatModifiedDate,
  getFileIcon,
  hasThumbnail,
} from "../lib/icons";
import { FileIcon, Icon } from "./Icon";
import { ThumbnailImage } from "./ThumbnailImage";
import { GalleryFeedbackBadges } from "./GalleryFeedbackBadges";

export interface FileDetailsSelection {
  entry: FileEntry;
  parentPath: string;
  path: string;
}

interface FileDetailsPaneProps {
  root: string;
  selected: FileDetailsSelection | null;
  width?: number | string;
  sticky?: boolean;
  flush?: boolean;
  title?: string;
  onPreview?: (entry: FileEntry, parentPath: string) => void;
  onClose?: () => void;
}

export function FileDetailsPane({
  root,
  selected,
  width = "100%",
  sticky = false,
  flush = false,
  title = "Info",
  onPreview,
  onClose,
}: FileDetailsPaneProps) {
  const entry = selected?.entry;
  const [fileInfo, setFileInfo] = useState<FileEntry | null>(null);
  const [dirSize, setDirSize] = useState<number | "loading" | null>(null);
  const icon = entry ? getFileIcon(entry) : null;
  const showThumb = Boolean(entry && !entry.is_dir && hasThumbnail(entry));
  const mediaInfo = fileInfo?.media_info ?? entry?.media_info ?? null;
  const imageInfo = fileInfo?.image_info ?? entry?.image_info ?? null;
  const displayEntry = fileInfo ?? entry;
  const galleryFeedback = displayEntry?.gallery_feedback ?? null;
  const mediaDetails = mediaInfo ? getMediaInfoDetails(mediaInfo) : [];
  const imageDetails = imageInfo ? getImageInfoDetails(imageInfo) : [];
  const selectedPath = selected?.path ?? "";
  const entryName = entry?.name ?? "";
  const entryIsDirectory = Boolean(entry?.is_dir);

  useEffect(() => {
    setFileInfo(null);
    if (!selectedPath || !entryName || entryIsDirectory) return;

    let cancelled = false;
    api
      .fileInfo(root, selectedPath)
      .then((info) => {
        if (!cancelled) setFileInfo(info);
      })
      .catch(() => {
        if (!cancelled) setFileInfo(null);
      });

    return () => {
      cancelled = true;
    };
  }, [entryIsDirectory, entryName, root, selectedPath]);

  useEffect(() => {
    setDirSize(null);
    if (!selectedPath || !entryName || !entryIsDirectory || !root) return;

    setDirSize("loading");
    let cancelled = false;
    api
      .folderSizes(root, [selectedPath])
      .then((result) => {
        if (!cancelled) setDirSize(result.sizes[selectedPath] ?? null);
      })
      .catch(() => {
        if (!cancelled) setDirSize(null);
      });

    return () => {
      cancelled = true;
    };
  }, [entryIsDirectory, entryName, root, selectedPath]);

  return (
    <aside
      aria-label={title}
      style={{
        position: sticky ? "sticky" : undefined,
        right: sticky ? 0 : undefined,
        top: sticky ? 0 : undefined,
        zIndex: sticky ? 2 : undefined,
        width,
        minWidth: width,
        maxWidth: width,
        height: "100%",
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        borderLeft:
          sticky && !flush ? "1px solid var(--color-border)" : undefined,
        border: sticky || flush ? undefined : "1px solid var(--color-border)",
        borderRadius: sticky || flush ? 0 : "var(--radius-lg)",
        background:
          sticky || flush ? "var(--color-bg)" : "var(--color-bg-subtle)",
        overflowY: "auto",
      }}
    >
      <div
        style={{
          ...columnTitleStyle,
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: "var(--space-2)",
        }}
      >
        <span>{title}</span>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            title="Close"
            aria-label="Close"
            style={iconButtonStyle}
          >
            <Icon name="x" size={14} />
          </button>
        )}
      </div>

      {!entry && (
        <div style={{ ...emptyPaneStyle, padding: "var(--space-6)" }}>
          Select an item
        </div>
      )}

      {entry && selected && (
        <div
          style={{
            padding: "var(--space-4)",
            display: "flex",
            flexDirection: "column",
            gap: "var(--space-4)",
          }}
        >
          <div
            style={{
              aspectRatio: "4 / 3",
              borderRadius: flush ? 0 : "var(--radius-md)",
              background: "var(--color-bg-muted)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              overflow: "hidden",
            }}
          >
            {showThumb ? (
              <ThumbnailImage
                root={root}
                path={selected.parentPath}
                entry={entry}
                width={480}
                fallbackSize={56}
              />
            ) : (
              icon && <FileIcon svg={icon.svg} color={icon.color} size={56} />
            )}
          </div>

          <div style={{ minWidth: 0 }}>
            <div
              style={{
                fontSize: "var(--text-base)",
                fontWeight: 600,
                color: "var(--color-fg)",
                overflowWrap: "anywhere",
                lineHeight: "var(--leading-sm)",
                display: "flex",
                alignItems: "center",
                gap: "var(--space-1)",
                flexWrap: "wrap",
              }}
            >
              <span>{entry.name}</span>
              {displayEntry && <GalleryFeedbackBadges entry={displayEntry} />}
            </div>
            <div
              style={{
                marginTop: "var(--space-1)",
                fontSize: "var(--text-xs)",
                color: "var(--color-fg-muted)",
                overflowWrap: "anywhere",
              }}
            >
              {selected.path}
            </div>
          </div>

          <dl style={detailsListStyle}>
            <InfoTerm
              label="Kind"
              value={entry.is_dir ? "Folder" : entry.mime_type || "File"}
            />
            <InfoTerm
              label="Size"
              value={
                entry.is_dir
                  ? dirSize === "loading"
                    ? "Calculating…"
                    : dirSize != null
                      ? formatFileSize(dirSize)
                      : "—"
                  : formatFileSize(entry.size)
              }
            />
            <InfoTerm
              label="Modified"
              value={formatModifiedDate(entry.modified_at)}
            />
          </dl>

          {mediaDetails.length > 0 && (
            <DetailSection title="Media" details={mediaDetails} />
          )}

          {imageDetails.length > 0 && (
            <DetailSection title="Image" details={imageDetails} />
          )}

          {galleryFeedback?.note && (
            <section
              style={{
                borderTop: "1px solid var(--color-border)",
                paddingTop: "var(--space-4)",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-1)",
                  marginBottom: "var(--space-2)",
                  color: "var(--color-fg-subtle)",
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                  letterSpacing: "var(--tracking-wide)",
                  textTransform: "uppercase",
                }}
              >
                <Icon name="fileText" size={14} />
                Gallery note
              </div>
              <div
                style={{
                  padding: "var(--space-3)",
                  border: "1px solid var(--color-border)",
                  borderRadius: "var(--radius-md)",
                  background: "var(--color-bg)",
                  color: "var(--color-fg)",
                  fontSize: "var(--text-sm)",
                  lineHeight: "var(--leading-sm)",
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                }}
              >
                {galleryFeedback.note}
              </div>
            </section>
          )}

          {!entry.is_dir && onPreview && (
            <button
              type="button"
              onClick={() => onPreview(entry, selected.parentPath)}
              style={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                gap: "var(--space-2)",
                padding: "var(--space-2) var(--space-3)",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-md)",
                background: "transparent",
                color: "var(--color-fg)",
                cursor: "pointer",
                fontSize: "var(--text-sm)",
                fontWeight: 500,
              }}
            >
              <Icon name="folderSearch" size={16} />
              Preview
            </button>
          )}
        </div>
      )}
    </aside>
  );
}

function DetailSection({
  title,
  details,
}: {
  title: string;
  details: Array<{ label: string; value: string }>;
}) {
  return (
    <section
      style={{
        borderTop: "1px solid var(--color-border)",
        paddingTop: "var(--space-4)",
      }}
    >
      <div
        style={{
          marginBottom: "var(--space-3)",
          color: "var(--color-fg-subtle)",
          fontSize: "var(--text-xs)",
          fontWeight: 600,
          letterSpacing: "var(--tracking-wide)",
          textTransform: "uppercase",
        }}
      >
        {title}
      </div>
      <dl style={detailsListStyle}>
        {details.map((detail) => (
          <InfoTerm
            key={detail.label}
            label={detail.label}
            value={detail.value}
          />
        ))}
      </dl>
    </section>
  );
}

function InfoTerm({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt style={{ color: "var(--color-fg-subtle)" }}>{label}</dt>
      <dd
        style={{
          color: "var(--color-fg)",
          minWidth: 0,
          overflowWrap: "anywhere",
        }}
      >
        {value}
      </dd>
    </>
  );
}

function getMediaInfoDetails(info: NonNullable<FileEntry["media_info"]>) {
  const details: Array<{ label: string; value: string }> = [];

  if (info.duration_ms !== null && info.duration_ms !== undefined) {
    details.push({ label: "Length", value: formatDuration(info.duration_ms) });
  }

  if (info.width && info.height) {
    details.push({
      label: "Resolution",
      value: `${info.width} x ${info.height}`,
    });
  }

  const streams: string[] = [];
  if (info.video_codec) streams.push(`Video: ${info.video_codec}`);
  if (info.audio_codec) streams.push(`Audio: ${info.audio_codec}`);
  if (streams.length > 0) {
    details.push({ label: "Streams", value: streams.join(" / ") });
  }

  const encodings = [info.video_codec, info.audio_codec].filter(Boolean);
  if (encodings.length > 0) {
    details.push({ label: "Encoding", value: encodings.join(" / ") });
  }

  const audioLanguages = info.audio_languages ?? [];
  if (audioLanguages.length > 0) {
    details.push({ label: "Audio", value: audioLanguages.join(", ") });
  }

  return details;
}

function getImageInfoDetails(info: NonNullable<FileEntry["image_info"]>) {
  const details: Array<{ label: string; value: string }> = [
    { label: "Resolution", value: `${info.width} x ${info.height}` },
  ];

  if (info.format) details.push({ label: "Format", value: info.format });
  if (info.has_alpha) details.push({ label: "Alpha", value: "Yes" });

  const exif = info.exif ?? {};
  const camera = [exif.Make, exif.Model].filter(Boolean).join(" ");
  if (camera) details.push({ label: "Camera", value: camera });
  if (exif.DateTimeOriginal || exif.DateTime) {
    details.push({
      label: "Captured",
      value: exif.DateTimeOriginal || exif.DateTime,
    });
  }
  if (exif.Orientation)
    details.push({ label: "Orientation", value: exif.Orientation });
  if (exif.Software) details.push({ label: "Software", value: exif.Software });

  return details;
}

function formatDuration(durationMs: number) {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1000));
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);

  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
  }

  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

const columnTitleStyle: React.CSSProperties = {
  padding: "var(--space-2) var(--space-3)",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  color: "var(--color-fg-subtle)",
  textTransform: "uppercase",
  letterSpacing: "var(--tracking-wide)",
  borderBottom: "1px solid var(--color-border)",
  flexShrink: 0,
};

const detailsListStyle: React.CSSProperties = {
  display: "grid",
  gridTemplateColumns: "auto minmax(0, 1fr)",
  gap: "var(--space-2) var(--space-3)",
  fontSize: "var(--text-sm)",
};

const emptyPaneStyle: React.CSSProperties = {
  color: "var(--color-fg-subtle)",
  fontSize: "var(--text-sm)",
  textAlign: "center",
};

const iconButtonStyle: React.CSSProperties = {
  width: 26,
  height: 26,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "transparent",
  color: "var(--color-fg-muted)",
  cursor: "pointer",
};
