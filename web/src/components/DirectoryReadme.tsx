import { useEffect, useState } from "react";
import type { FileEntry } from "../api/client";
import api from "../api/client";
import MarkdownPreview from "@uiw/react-markdown-preview";
import { Icon } from "./Icon";

/**
 * Allowlist URL sanitizer applied to every link/image href in the rendered
 * README markdown. `@uiw/react-markdown-preview` already falls back to
 * react-markdown's `defaultUrlTransform` (which strips `javascript:`), but
 * asserting it explicitly keeps the protection from silently regressing on a
 * library bump or if a custom transform is ever added: an unsafe scheme
 * (`javascript:`, `vbscript:`, `data:`, …) is stripped to an empty string,
 * while relative paths and the common safe schemes pass through unchanged.
 */
const SAFE_URL_PROTOCOL = /^(?:https?|mailto|tel|xmpp|ircs?):$/i;

function safeUrlTransform(url: string): string {
  const value = url ?? "";
  const colon = value.indexOf(":");
  if (colon === -1) return value; // relative URL, no scheme
  const slash = value.indexOf("/");
  const question = value.indexOf("?");
  const hash = value.indexOf("#");
  // A ':' appearing after the first '/', '?', or '#' is part of the path/query/
  // fragment, not a scheme (e.g. "foo/bar:baz" or "#a:b") — safe to keep.
  if (
    (slash !== -1 && colon > slash) ||
    (question !== -1 && colon > question) ||
    (hash !== -1 && colon > hash)
  ) {
    return value;
  }
  // Explicit scheme present: only allow the known-safe allowlist.
  return SAFE_URL_PROTOCOL.test(value.slice(0, colon + 1)) ? value : "";
}

interface DirectoryReadmeProps {
  entries: FileEntry[];
  root?: string;
  path?: string;
  shareConfig?: { token: string; bearer: string; subPath: string };
  onClose?: () => void;
}

export function DirectoryReadme({
  entries,
  root,
  path,
  shareConfig,
  onClose,
}: DirectoryReadmeProps) {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [showAll, setShowAll] = useState(false);

  // Find README file
  const readmeEntry =
    entries.find((e) => e.name === "README.md") ||
    entries.find((e) => e.name === "Readme.md") ||
    entries.find((e) => e.name === "readme.md");

  useEffect(() => {
    if (!readmeEntry) {
      setContent(null);
      return;
    }
    const fetchContent = async () => {
      setLoading(true);
      try {
        let downloadUrl = "";
        if (shareConfig) {
          const entryPath = shareConfig.subPath
            ? `${shareConfig.subPath}/${readmeEntry.name}`
            : readmeEntry.name;
          downloadUrl = api.shareDownloadUrl(
            shareConfig.token,
            shareConfig.bearer,
            entryPath,
          );
        } else if (root && path !== undefined) {
          const entryPath = path
            ? `${path}/${readmeEntry.name}`
            : readmeEntry.name;
          downloadUrl = api.downloadUrl(root, entryPath);
        } else {
          return;
        }

        const res = await fetch(downloadUrl, {
          headers: { "X-NasFiles-Request": "1" },
        });
        if (res.ok) {
          setContent(await res.text());
        }
      } catch (e) {
        console.error("Failed to load readme", e);
      } finally {
        setLoading(false);
      }
    };
    fetchContent();
  }, [readmeEntry, root, path, shareConfig]);

  if (!readmeEntry || (!loading && !content)) {
    return null;
  }

  const isCollapsed = !showAll && content;

  return (
    <div
      className="fade-in lg:sticky lg:h-[calc(100vh-140px)] lg:top-[var(--space-4)]"
      style={{
        width: "100%",
        maxWidth: 400,
        flexShrink: 0,
        background: "var(--color-bg-subtle)",
        borderRadius: "var(--radius-lg)",
        border: "1px solid var(--color-border)",
        padding: "var(--space-4)",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--space-2)",
          justifyContent: "space-between",
          marginBottom: "var(--space-3)",
          color: "var(--color-fg-muted)",
          flexShrink: 0,
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            minWidth: 0,
          }}
        >
          <Icon name="bookOpen" size={16} />
          <span
            style={{
              fontSize: "var(--text-sm)",
              fontWeight: 600,
              textTransform: "uppercase",
              letterSpacing: "var(--tracking-wide)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {readmeEntry.name}
          </span>
        </div>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            title="Close README"
            aria-label="Close README"
            style={{
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
              flexShrink: 0,
            }}
          >
            <Icon name="x" size={14} />
          </button>
        )}
      </div>

      <div
        className={isCollapsed ? "max-h-[180px] lg:max-h-none" : ""}
        style={{
          fontSize: "var(--text-sm)",
          overflowY: "auto", // Allow internal scrolling on desktop if readme is long
          overflowX: "hidden",
          position: "relative",
          transition: "max-height var(--duration-normal) var(--ease-out)",
          flex: 1, // Take available height
          minHeight: 0, // Prevent content bounding blowout
        }}
      >
        {loading ? (
          <div
            className="shimmer"
            style={{ height: 120, borderRadius: "var(--radius-md)" }}
          />
        ) : (
          <div data-color-mode="dark">
            <MarkdownPreview
              source={content || ""}
              urlTransform={safeUrlTransform}
              style={{
                backgroundColor: "transparent",
                color: "var(--color-fg)",
                padding: 0,
              }}
            />
          </div>
        )}

        {/* Fading overlay for mobile collapsed state ONLY */}
        {isCollapsed && (
          <div
            className="lg:hidden"
            style={{
              position: "absolute",
              bottom: 0,
              left: 0,
              right: 0,
              height: 60,
              background:
                "linear-gradient(to top, var(--color-bg-subtle), transparent)",
              pointerEvents: "none",
            }}
          />
        )}
      </div>

      {content && (
        <button
          className="lg:hidden"
          onClick={() => setShowAll(!showAll)}
          style={{
            marginTop: "var(--space-2)",
            color: "var(--color-accent)",
            fontSize: "var(--text-sm)",
            fontWeight: 500,
            background: "transparent",
            border: "none",
            cursor: "pointer",
            textAlign: "left",
            flexShrink: 0,
          }}
        >
          {showAll ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}
