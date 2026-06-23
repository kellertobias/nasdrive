interface BreadcrumbProps {
  root: string;
  rootDisplayName: string;
  path: string;
  onNavigate: (path: string) => void;
}

export function Breadcrumb({
  rootDisplayName,
  path,
  onNavigate,
}: BreadcrumbProps) {
  const segments = path ? path.split("/").filter(Boolean) : [];

  const allSegments = [
    { name: rootDisplayName, path: "" },
    ...segments.map((seg, i) => ({
      name: seg,
      path: segments.slice(0, i + 1).join("/"),
    })),
  ];

  // If too many segments, collapse middle ones
  const MAX_VISIBLE = 4;
  let visible = allSegments;
  let collapsed: typeof allSegments = [];

  if (allSegments.length > MAX_VISIBLE) {
    visible = [allSegments[0], ...allSegments.slice(-MAX_VISIBLE + 1)];
    collapsed = allSegments.slice(1, -(MAX_VISIBLE - 1));
  }

  return (
    <nav
      aria-label="Breadcrumb"
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-1)",
        fontSize: "var(--text-sm)",
        color: "var(--color-fg-muted)",
        minWidth: 0,
        overflow: "hidden",
      }}
    >
      {visible.map((segment, index) => {
        const isLast = index === visible.length - 1;
        const showCollapsed = index === 1 && collapsed.length > 0;

        return (
          <span
            key={segment.path}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-1)",
              minWidth: 0,
            }}
          >
            {index > 0 && (
              <span style={{ color: "var(--color-fg-subtle)", flexShrink: 0 }}>
                /
              </span>
            )}

            {showCollapsed && (
              <>
                <span
                  title={collapsed.map((c) => c.name).join(" / ")}
                  style={{
                    padding: "0 var(--space-1)",
                    cursor: "pointer",
                    color: "var(--color-fg-subtle)",
                    borderRadius: "var(--radius-sm)",
                    transition: `background var(--duration-fast) var(--ease-out)`,
                  }}
                  onMouseOver={(e) =>
                    (e.currentTarget.style.background = "var(--color-bg-muted)")
                  }
                  onMouseOut={(e) =>
                    (e.currentTarget.style.background = "transparent")
                  }
                >
                  ···
                </span>
                <span
                  style={{ color: "var(--color-fg-subtle)", flexShrink: 0 }}
                >
                  /
                </span>
              </>
            )}

            {isLast ? (
              <span
                style={{
                  fontWeight: 500,
                  color: "var(--color-fg)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {segment.name}
              </span>
            ) : (
              <button
                onClick={() => onNavigate(segment.path)}
                style={{
                  background: "none",
                  border: "none",
                  cursor: "pointer",
                  color: "var(--color-fg-muted)",
                  padding: "0 var(--space-1)",
                  borderRadius: "var(--radius-sm)",
                  fontSize: "var(--text-sm)",
                  transition: `all var(--duration-fast) var(--ease-out)`,
                  whiteSpace: "nowrap",
                }}
                onMouseOver={(e) => {
                  e.currentTarget.style.background = "var(--color-bg-muted)";
                  e.currentTarget.style.color = "var(--color-fg)";
                }}
                onMouseOut={(e) => {
                  e.currentTarget.style.background = "transparent";
                  e.currentTarget.style.color = "var(--color-fg-muted)";
                }}
              >
                {segment.name}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}
