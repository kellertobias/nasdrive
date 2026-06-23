import type { RootUsage } from "../api/client";

interface UsageRingProps {
  usage?: RootUsage | null;
}

export function UsageRing({ usage }: UsageRingProps) {
  if (!usage || usage.total_bytes <= 0) return null;

  const ratio = Math.min(1, Math.max(0, usage.used_bytes / usage.total_bytes));
  const percent = Math.round(ratio * 100);
  const color =
    percent >= 90
      ? "var(--color-danger)"
      : percent >= 75
        ? "var(--color-warning)"
        : "var(--color-accent)";
  const used = formatBytes(usage.used_bytes);
  const total = formatBytes(usage.total_bytes);

  return (
    <span
      aria-label={`${percent}% used`}
      title={`${used} of ${total} used (${percent}%)`}
      style={{
        flex: "0 0 auto",
        marginLeft: "auto",
        display: "inline-flex",
        alignItems: "center",
        gap: 5,
        minHeight: 20,
      }}
    >
      <span
        style={{
          display: "inline-flex",
          flexDirection: "column",
          justifyContent: "center",
          alignItems: "flex-end",
          minWidth: 42,
          lineHeight: 1.05,
          fontSize: 8,
          fontWeight: 500,
          color: "var(--color-fg-subtle)",
          fontVariantNumeric: "tabular-nums",
          whiteSpace: "nowrap",
        }}
      >
        <span>{used} used</span>
        <span>{total} total</span>
      </span>
      <span
        style={{
          flex: "0 0 auto",
          width: 16,
          height: 16,
          borderRadius: "50%",
          background: `conic-gradient(${color} ${percent}%, var(--color-border) 0)`,
          boxShadow:
            "inset 0 0 0 1px color-mix(in oklch, var(--color-border), transparent 25%)",
          position: "relative",
        }}
      >
        <span
          style={{
            position: "absolute",
            inset: 4.2,
            borderRadius: "50%",
            background: "var(--color-sidebar-bg)",
          }}
        />
      </span>
    </span>
  );
}

function formatBytes(bytes: number) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  const digits = value >= 10 || unitIndex === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unitIndex]}`;
}
