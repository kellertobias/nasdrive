import type { TransferJob } from "../api/client";
import { transferProgressPercent } from "../lib/transferJobs";

export function TransferProgressIndicator({
  jobs,
  compact = false,
}: {
  jobs: TransferJob[];
  compact?: boolean;
}) {
  if (jobs.length === 0) return null;

  const percent = transferProgressPercent(jobs);
  const label = jobs.some((job) => job.operation === "copy")
    ? "Copying here"
    : "Moving here";

  if (compact) {
    return (
      <span
        title={`${label}: ${percent}%`}
        style={{
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          gap: 5,
          color: "var(--color-accent)",
          fontSize: "var(--text-xs)",
          fontWeight: 600,
          fontVariantNumeric: "tabular-nums",
          flexShrink: 0,
        }}
      >
        <span>{percent}%</span>
        <span
          style={{
            width: 16,
            height: 16,
            borderRadius: "50%",
            background: `conic-gradient(var(--color-accent) ${percent}%, var(--color-border) 0)`,
            boxShadow:
              "inset 0 0 0 1px color-mix(in oklch, var(--color-border), transparent 25%)",
          }}
        />
      </span>
    );
  }

  return (
    <div
      title={`${label}: ${percent}%`}
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        padding: "var(--space-2) var(--space-3)",
        border: "1px solid var(--color-accent)",
        borderRadius: "var(--radius-md)",
        background: "var(--color-accent-muted)",
        color: "var(--color-fg)",
        fontSize: "var(--text-sm)",
        fontWeight: 500,
      }}
    >
      <span>{label}</span>
      <div
        style={{
          width: 120,
          height: 4,
          borderRadius: 2,
          background: "var(--color-border)",
          overflow: "hidden",
        }}
      >
        <div
          style={{
            width: `${percent}%`,
            height: "100%",
            borderRadius: 2,
            background: "var(--color-accent)",
            transition: "width 200ms ease-out",
          }}
        />
      </div>
      <span
        className="tabular-nums"
        style={{ color: "var(--color-fg-muted)", fontSize: "var(--text-xs)" }}
      >
        {percent}%
      </span>
    </div>
  );
}
