import { useEffect, useCallback, useState, useRef } from "react";
import type { FileEntry } from "../api/client";
import api from "../api/client";
import { getPreviewType, getFileIcon, formatFileSize } from "../lib/icons";
import { Icon, FileIcon } from "./Icon";
import { MediaPreview } from "./MediaPreview";
import CodeMirror from "@uiw/react-codemirror";
import { monokai } from "@uiw/codemirror-theme-monokai";
import { loadLanguage } from "@uiw/codemirror-extensions-langs";
interface PreviewPaneProps {
  entry: FileEntry;
  root: string;
  path: string;
  entries: FileEntry[];
  mediaPreviewTranscodingEnabled: boolean;
  onClose: () => void;
  onNavigate: (entry: FileEntry) => void;
}

export function PreviewPane({
  entry,
  root,
  path,
  entries,
  mediaPreviewTranscodingEnabled,
  onClose,
  onNavigate,
}: PreviewPaneProps) {
  const entryPath = path ? `${path}/${entry.name}` : entry.name;
  const detectedPreviewType = getPreviewType(entry);
  const previewType = detectedPreviewType;
  const isMediaPreview = previewType === "video" || previewType === "audio";
  const [fileInfo, setFileInfo] = useState<FileEntry | null>(null);
  const downloadUrl = api.downloadUrl(root, entryPath);
  const audioCoverUrl =
    previewType === "audio" && entry.has_thumbnail
      ? api.thumbnailUrl(root, entryPath, 720, entry)
      : null;
  const loadMediaInfo = useCallback(
    () => api.fileInfo(root, entryPath),
    [entryPath, root],
  );
  const createMediaPreviewUrl = useCallback(
    (session: string) => api.previewUrl(root, entryPath, session),
    [entryPath, root],
  );
  const loadMediaPreviewStatus = useCallback(
    (session: string) => api.previewStatus(root, entryPath, session),
    [entryPath, root],
  );
  const mediaInfo = fileInfo?.media_info ?? entry.media_info ?? null;
  const imageInfo = fileInfo?.image_info ?? entry.image_info ?? null;
  const imagePreview = imagePreviewSource(entry, imageInfo);
  const imagePreviewUrl =
    previewType === "image" && imagePreview
      ? api.thumbnailUrl(
          root,
          entryPath,
          imagePreview.width,
          entry,
          0,
          imagePreview.format,
        )
      : downloadUrl;
  const mediaDetails = mediaInfo ? formatMediaDetails(mediaInfo) : [];
  const imageDetails = imageInfo ? formatImageDetails(imageInfo) : [];

  // Find current index for prev/next navigation
  const fileEntries = entries.filter((e) => !e.is_dir);
  const currentIndex = fileEntries.findIndex((e) => e.name === entry.name);

  const navigatePrev = useCallback(() => {
    if (currentIndex > 0) onNavigate(fileEntries[currentIndex - 1]);
  }, [currentIndex, fileEntries, onNavigate]);

  const navigateNext = useCallback(() => {
    if (currentIndex < fileEntries.length - 1)
      onNavigate(fileEntries[currentIndex + 1]);
  }, [currentIndex, fileEntries, onNavigate]);

  // Keyboard handler
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target instanceof Element ? e.target : null;
      if (
        target?.closest(
          '.video-js, audio, video, input, textarea, select, button, a, [role="slider"]',
        )
      ) {
        if (
          e.key === "ArrowLeft" ||
          e.key === "ArrowRight" ||
          e.key === " " ||
          e.key === "Space" ||
          e.key === "Spacebar"
        ) {
          return;
        }
      }

      if (
        e.key === "Escape" ||
        ((e.key === " " || e.key === "Space" || e.key === "Spacebar") &&
          !e.repeat)
      ) {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
        e.preventDefault();
        navigatePrev();
      } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
        e.preventDefault();
        navigateNext();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose, navigatePrev, navigateNext]);

  useEffect(() => {
    setFileInfo(null);
    if (
      previewType !== "video" &&
      previewType !== "audio" &&
      previewType !== "image"
    )
      return;

    let cancelled = false;
    loadMediaInfo()
      .then((info) => {
        if (!cancelled) setFileInfo(info);
      })
      .catch(() => {
        if (!cancelled) setFileInfo(null);
      });

    return () => {
      cancelled = true;
    };
  }, [loadMediaInfo, previewType]);

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "#000",
        display: "flex",
        flexDirection: "column",
        zIndex: 200,
      }}
      className="preview-pane fade-in"
      onClickCapture={(e) => {
        if ((e.target as Element | null)?.closest("[data-preview-no-close]")) {
          return;
        }
        if (
          isMediaPreview &&
          shouldCloseFromPreviewBackdropClick(
            e.currentTarget,
            e.clientX,
            e.clientY,
          )
        ) {
          onClose();
        }
      }}
      onClick={(e) => {
        if (isMediaPreview) return;
        // Close only if clicking the outer backdrop, with a safe zone.
        // We measure the content area (flex child) and close only if the
        // click lands more than 75px outside it.
        if (e.target === e.currentTarget) {
          const safeZone = 75;
          const contentEl = e.currentTarget.querySelector(
            "[data-preview-content]",
          ) as HTMLElement | null;
          if (contentEl) {
            const cr = contentEl.getBoundingClientRect();
            const inSafeZone =
              e.clientX >= cr.left - safeZone &&
              e.clientX <= cr.right + safeZone &&
              e.clientY >= cr.top - safeZone &&
              e.clientY <= cr.bottom + safeZone;
            if (!inSafeZone) onClose();
          } else {
            onClose();
          }
        }
      }}
    >
      {/* Top bar */}
      <div
        data-preview-no-close
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          padding: "var(--space-3) var(--space-4)",
          color: "#fff",
          flexShrink: 0,
          position: "relative",
          zIndex: 3,
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            overflow: "hidden",
          }}
        >
          <span
            style={{
              fontSize: "var(--text-sm)",
              fontWeight: 500,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {entry.name}
          </span>
          <span
            style={{
              fontSize: "var(--text-xs)",
              color: "rgba(255,255,255,0.5)",
            }}
          >
            {formatFileSize(entry.size)}
          </span>
          {mediaDetails.map((detail) => (
            <span
              key={detail}
              style={{
                fontSize: "var(--text-xs)",
                color: "rgba(255,255,255,0.5)",
                whiteSpace: "nowrap",
              }}
            >
              {detail}
            </span>
          ))}
          {imageDetails.map((detail) => (
            <span
              key={detail}
              style={{
                fontSize: "var(--text-xs)",
                color: "rgba(255,255,255,0.5)",
                whiteSpace: "nowrap",
              }}
            >
              {detail}
            </span>
          ))}
          {previewType === "image" && imagePreview && (
            <span style={previewBadgeStyle}>Preview</span>
          )}
          {fileEntries.length > 1 && (
            <span
              style={{
                fontSize: "var(--text-xs)",
                color: "rgba(255,255,255,0.4)",
              }}
            >
              {currentIndex + 1} / {fileEntries.length}
            </span>
          )}
        </div>

        <div style={{ display: "flex", gap: "var(--space-2)" }}>
          <a
            href={downloadUrl}
            target="_blank"
            rel="noopener"
            style={iconButtonStyle}
            title="Download"
          >
            <Icon name="download" size={16} color="#fff" />
          </a>
          <button onClick={onClose} style={iconButtonStyle} title="Close (Esc)">
            <Icon name="x" size={16} color="#fff" />
          </button>
        </div>
      </div>

      {/* Navigation arrows */}
      {currentIndex > 0 && (
        <button
          data-preview-no-close
          onClick={(e) => {
            e.stopPropagation();
            navigatePrev();
          }}
          style={{ ...navArrowStyle, left: "var(--space-2)" }}
          title="Previous (←)"
        >
          <Icon
            name="chevronRight"
            size={24}
            color="#fff"
            style={{ transform: "rotate(180deg)" }}
          />
        </button>
      )}
      {currentIndex < fileEntries.length - 1 && (
        <button
          data-preview-no-close
          onClick={(e) => {
            e.stopPropagation();
            navigateNext();
          }}
          style={{ ...navArrowStyle, right: "var(--space-2)" }}
          title="Next (→)"
        >
          <Icon name="chevronRight" size={24} color="#fff" />
        </button>
      )}

      {/* Content area */}
      <div
        data-preview-content
        style={{
          flex: 1,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          overflow: "hidden",
          padding: "var(--space-4)",
        }}
        onClick={(e) => {
          if (!isMediaPreview) e.stopPropagation();
        }}
      >
        {previewType === "image" && (
          <ImagePreview url={imagePreviewUrl} name={entry.name} />
        )}
        {previewType === "video" && (
          <MediaPreview
            entry={entry}
            kind="video"
            actualUrl={downloadUrl}
            canTranscode={mediaPreviewTranscodingEnabled}
            createPreviewUrl={createMediaPreviewUrl}
            loadPreviewStatus={loadMediaPreviewStatus}
            loadFileInfo={loadMediaInfo}
            initialFileInfo={fileInfo}
            onInfoLoaded={setFileInfo}
          />
        )}
        {previewType === "audio" && (
          <MediaPreview
            entry={entry}
            kind="audio"
            actualUrl={downloadUrl}
            coverArtUrl={audioCoverUrl}
            canTranscode={mediaPreviewTranscodingEnabled}
            createPreviewUrl={createMediaPreviewUrl}
            loadPreviewStatus={loadMediaPreviewStatus}
            loadFileInfo={loadMediaInfo}
            initialFileInfo={fileInfo}
            onInfoLoaded={setFileInfo}
          />
        )}
        {previewType === "text" && (
          <TextPreview url={downloadUrl} name={entry.name} />
        )}
        {previewType === "pdf" && (
          <PdfPreview url={downloadUrl} name={entry.name} />
        )}
        {previewType === null && (
          <FallbackPreview entry={entry} downloadUrl={downloadUrl} />
        )}
      </div>
    </div>
  );
}

function formatMediaDetails(
  info: NonNullable<FileEntry["media_info"]>,
): string[] {
  const details: string[] = [];

  if (info.width && info.height) {
    details.push(`${info.width}x${info.height}`);
  }

  const codecs = [info.video_codec, info.audio_codec].filter(Boolean);
  if (codecs.length > 0) {
    details.push(codecs.join(" / "));
  }

  const audioLanguages = info.audio_languages ?? [];
  if (audioLanguages.length > 0) {
    details.push(`audio: ${audioLanguages.join(",")}`);
  }

  if (info.duration_ms !== null && info.duration_ms !== undefined) {
    details.push(formatDuration(info.duration_ms));
  }

  return details;
}

function formatImageDetails(
  info: NonNullable<FileEntry["image_info"]>,
): string[] {
  const details = [`${info.width}x${info.height}`];

  if (info.format) details.push(info.format);
  if (info.has_alpha) details.push("transparent");

  const exif = info.exif ?? {};
  const camera = [exif.Make, exif.Model].filter(Boolean).join(" ");
  if (camera) details.push(camera);
  if (exif.DateTimeOriginal || exif.DateTime)
    details.push(exif.DateTimeOriginal || exif.DateTime);

  return details;
}

function transparentPreviewFormat(name: string): "jpeg" | "png" {
  const ext = name.split(".").pop()?.toLowerCase();
  return ext === "png" || ext === "webp" || ext === "gif" || ext === "svg"
    ? "png"
    : "jpeg";
}

function imagePreviewSource(
  entry: FileEntry,
  info: FileEntry["image_info"] | null | undefined,
): { width: number; format: "jpeg" | "png" } | null {
  if (!entry.has_thumbnail || !info) return null;

  const ext = entry.name.split(".").pop()?.toLowerCase();
  const limit = ext === "png" ? 2000 : 1080;
  if (info.width <= limit && info.height <= limit) return null;

  return {
    width: limit,
    format: transparentPreviewFormat(entry.name),
  };
}

function formatDuration(durationMs: number): string {
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

const PREVIEW_BACKDROP_SAFE_ZONE_PX = 50;

function shouldCloseFromPreviewBackdropClick(
  container: HTMLElement,
  clientX: number,
  clientY: number,
): boolean {
  const protectedElements = Array.from(
    container.querySelectorAll<HTMLElement>("[data-preview-no-close]"),
  );

  return !protectedElements.some((element) => {
    const rect = element.getBoundingClientRect();
    return (
      clientX >= rect.left - PREVIEW_BACKDROP_SAFE_ZONE_PX &&
      clientX <= rect.right + PREVIEW_BACKDROP_SAFE_ZONE_PX &&
      clientY >= rect.top - PREVIEW_BACKDROP_SAFE_ZONE_PX &&
      clientY <= rect.bottom + PREVIEW_BACKDROP_SAFE_ZONE_PX
    );
  });
}

// ---------------------------------------------------------------------------
// Image preview with zoom/pan
// ---------------------------------------------------------------------------

function ImagePreview({ url, name }: { url: string; name: string }) {
  const [loaded, setLoaded] = useState(false);
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const dragging = useRef(false);
  const lastPos = useRef({ x: 0, y: 0 });

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.stopPropagation();
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    setScale((s) => Math.min(10, Math.max(0.1, s * delta)));
  }, []);

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (scale > 1) {
        dragging.current = true;
        lastPos.current = { x: e.clientX, y: e.clientY };
      }
    },
    [scale],
  );

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (dragging.current) {
      const dx = e.clientX - lastPos.current.x;
      const dy = e.clientY - lastPos.current.y;
      lastPos.current = { x: e.clientX, y: e.clientY };
      setTranslate((t) => ({ x: t.x + dx, y: t.y + dy }));
    }
  }, []);

  const handleMouseUp = useCallback(() => {
    dragging.current = false;
  }, []);

  // Double click to reset zoom
  const handleDoubleClick = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor:
          scale > 1 ? (dragging.current ? "grabbing" : "grab") : "zoom-in",
        overflow: "hidden",
        backgroundColor: "rgba(255,255,255,0.02)",
      }}
      onWheel={handleWheel}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      onMouseLeave={handleMouseUp}
      onDoubleClick={handleDoubleClick}
      onClick={(e) => e.stopPropagation()}
    >
      {!loaded && (
        <div
          className="shimmer"
          style={{ width: 300, height: 200, borderRadius: 8 }}
        />
      )}
      <img
        src={url}
        alt={name}
        onLoad={() => setLoaded(true)}
        style={{
          maxWidth: "100%",
          maxHeight: "100%",
          objectFit: "contain",
          transform: `scale(${scale}) translate(${translate.x / scale}px, ${translate.y / scale}px)`,
          transition: dragging.current
            ? "none"
            : "transform var(--duration-fast) var(--ease-out)",
          opacity: loaded ? 1 : 0,
          userSelect: "none",
          pointerEvents: "none",
          backgroundColor: "#fff",
          backgroundImage:
            "linear-gradient(45deg, rgba(0,0,0,0.18) 25%, transparent 25%), linear-gradient(-45deg, rgba(0,0,0,0.18) 25%, transparent 25%), linear-gradient(45deg, transparent 75%, rgba(0,0,0,0.18) 75%), linear-gradient(-45deg, transparent 75%, rgba(0,0,0,0.18) 75%)",
          backgroundSize: "20px 20px",
          backgroundPosition: "0 0, 0 10px, 10px -10px, -10px 0",
        }}
        draggable={false}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Text preview
// ---------------------------------------------------------------------------

function TextPreview({ url, name }: { url: string; name: string }) {
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    fetch(url, { headers: { "X-NasFiles-Request": "1" } })
      .then(async (r) => {
        if (r.ok) {
          const text = await r.text();
          setContent(text.slice(0, 500_000)); // Cap at 500KB for performance
        } else {
          setError(true);
        }
      })
      .catch(() => setError(true));
  }, [url]);

  if (error)
    return (
      <div style={{ color: "rgba(255,255,255,0.5)" }}>Failed to load file</div>
    );

  const ext = name.split(".").pop()?.toLowerCase();
  let langExtension;
  try {
    langExtension = ext
      ? loadLanguage(ext as Parameters<typeof loadLanguage>[0])
      : undefined;
  } catch {
    // Ignore unsupported languages
  }

  return (
    <div
      style={{
        width: "100%",
        maxWidth: 1000,
        height: "100%",
        overflow: "hidden",
        background: "#272822",
        border: "1px solid rgba(255,255,255,0.1)",
        borderRadius: "var(--preview-content-radius, var(--radius-lg))",
        display: "flex",
        flexDirection: "column",
      }}
      onClick={(e) => e.stopPropagation()}
    >
      {content === null ? (
        <div className="shimmer" style={{ flex: 1, borderRadius: 8 }} />
      ) : (
        <div style={{ flex: 1, overflow: "auto" }}>
          <CodeMirror
            value={content}
            editable={false}
            theme={monokai}
            extensions={langExtension ? [langExtension] : []}
            basicSetup={{
              lineNumbers: true,
              foldGutter: true,
              highlightActiveLine: true,
            }}
          />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// PDF preview (iframe fallback — pdfjs-dist can be added later for better UX)
// ---------------------------------------------------------------------------

function PdfPreview({ url }: { url: string; name: string }) {
  return (
    <div
      style={{
        width: "100%",
        maxWidth: 900,
        height: "100%",
        borderRadius: "var(--preview-content-radius, var(--radius-lg))",
        overflow: "hidden",
        boxShadow: "0 20px 60px rgba(0,0,0,0.5)",
        display: "flex",
        flexDirection: "column",
      }}
      onClick={(e) => e.stopPropagation()}
    >
      {/* Use <object> which works better cross-browser than <iframe> for PDFs */}
      <object
        data={`${url}#toolbar=1&navpanes=0`}
        type="application/pdf"
        style={{
          width: "100%",
          flex: 1,
          border: "none",
          background: "#fff",
        }}
      >
        {/* Fallback for browsers that can't render inline PDFs */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            gap: "var(--space-4)",
            padding: "var(--space-8)",
            color: "#fff",
            textAlign: "center",
          }}
        >
          <Icon name="file" size={48} color="rgba(255,255,255,0.5)" />
          <div style={{ fontSize: "var(--text-base)" }}>
            PDF preview not supported in this browser.
          </div>
          <a
            href={url}
            target="_blank"
            rel="noopener"
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "var(--space-2)",
              padding: "var(--space-2) var(--space-4)",
              background: "rgba(255,255,255,0.15)",
              border: "1px solid rgba(255,255,255,0.2)",
              borderRadius: "var(--radius-md)",
              color: "#fff",
              textDecoration: "none",
              fontSize: "var(--text-sm)",
              fontWeight: 500,
            }}
          >
            <Icon name="download" size={14} color="#fff" />
            Download PDF
          </a>
        </div>
      </object>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Fallback — unsupported preview
// ---------------------------------------------------------------------------

function FallbackPreview({
  entry,
  downloadUrl,
}: {
  entry: FileEntry;
  downloadUrl: string;
}) {
  const icon = getFileIcon(entry);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        gap: "var(--space-4)",
        color: "#fff",
        textAlign: "center",
      }}
    >
      <FileIcon svg={icon.svg} color={icon.color} size={64} />
      <div style={{ fontSize: "var(--text-lg)", fontWeight: 500 }}>
        {entry.name}
      </div>
      <div
        style={{ fontSize: "var(--text-sm)", color: "rgba(255,255,255,0.5)" }}
      >
        {formatFileSize(entry.size)} · No preview available
      </div>
      <a
        href={downloadUrl}
        target="_blank"
        rel="noopener"
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: "var(--space-2)",
          padding: "var(--space-2) var(--space-4)",
          background: "rgba(255,255,255,0.15)",
          border: "1px solid rgba(255,255,255,0.2)",
          borderRadius: "var(--radius-md)",
          color: "#fff",
          textDecoration: "none",
          fontSize: "var(--text-sm)",
          fontWeight: 500,
        }}
      >
        <Icon name="file" size={14} color="#fff" />
        Download
      </a>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Styles
// ---------------------------------------------------------------------------

const iconButtonStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: 32,
  height: 32,
  border: "none",
  borderRadius: "var(--radius-md)",
  background: "rgba(255,255,255,0.1)",
  cursor: "pointer",
  textDecoration: "none",
};

const previewBadgeStyle: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  height: 20,
  padding: "0 8px",
  border: "1px solid rgba(255,255,255,0.22)",
  borderRadius: 999,
  background: "rgba(255,255,255,0.12)",
  color: "rgba(255,255,255,0.86)",
  fontSize: "var(--text-xs)",
  fontWeight: 600,
  lineHeight: 1,
  whiteSpace: "nowrap",
};

const navArrowStyle: React.CSSProperties = {
  position: "fixed",
  top: "50%",
  transform: "translateY(-50%)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  width: 44,
  height: 44,
  border: "none",
  borderRadius: "var(--radius-full)",
  background: "rgba(255,255,255,0.1)",
  cursor: "pointer",
  zIndex: 210,
  transition: "background var(--duration-fast) var(--ease-out)",
};
