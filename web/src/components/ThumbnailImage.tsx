import { useEffect, useMemo, useState } from "react";
import type { FileEntry } from "../api/client";
import api from "../api/client";
import { getFileIcon } from "../lib/icons";
import { FileIcon } from "./Icon";

interface ThumbnailImageProps {
  root: string;
  path: string;
  entry: FileEntry;
  width?: number;
  fallbackSize?: number;
  aspectRatio?: string;
}

const RETRY_DELAYS = [400, 1200, 2600];

export function ThumbnailImage({
  root,
  path,
  entry,
  width = 480,
  fallbackSize = 36,
  aspectRatio = "4/3",
}: ThumbnailImageProps) {
  const [loaded, setLoaded] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [failed, setFailed] = useState(false);
  const entryPath = path ? `${path}/${entry.name}` : entry.name;
  const src = useMemo(
    () => api.thumbnailUrl(root, entryPath, width, entry, attempt),
    [attempt, entry, entryPath, root, width],
  );

  useEffect(() => {
    setLoaded(false);
    setAttempt(0);
    setFailed(false);
  }, [entryPath, entry.modified_at, entry.size, width]);

  if (failed) {
    const icon = getFileIcon(entry);
    return <FileIcon svg={icon.svg} color={icon.color} size={fallbackSize} />;
  }

  return (
    <div
      style={{
        width: "100%",
        aspectRatio,
        borderRadius: "var(--radius-md)",
        overflow: "hidden",
        background: "var(--color-bg-muted)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        position: "relative",
      }}
    >
      {!loaded && (
        <div
          className="shimmer"
          style={{
            position: "absolute",
            inset: 0,
            borderRadius: "var(--radius-md)",
          }}
        />
      )}
      <img
        key={src}
        src={src}
        loading="lazy"
        alt=""
        onLoad={() => setLoaded(true)}
        onError={() => {
          const delay = RETRY_DELAYS[attempt];
          if (delay === undefined) {
            setFailed(true);
            return;
          }
          window.setTimeout(() => setAttempt((value) => value + 1), delay);
        }}
        style={{
          width: "100%",
          height: "100%",
          objectFit: "cover",
          opacity: loaded ? 1 : 0,
          transition: "opacity var(--duration-normal) var(--ease-out)",
        }}
      />
    </div>
  );
}
