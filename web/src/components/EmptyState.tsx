import { Icon } from "./Icon";
import { ICONS } from "../lib/icons";

interface EmptyStateProps {
  iconName: keyof typeof ICONS;
  title: string;
  description: string;
  action?: {
    label: string;
    onClick: () => void;
  };
}

export function EmptyState({
  iconName,
  title,
  description,
  action,
}: EmptyStateProps) {
  return (
    <div
      className="fade-in"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        padding: "var(--space-16) var(--space-8)",
        textAlign: "center",
        gap: "var(--space-4)",
      }}
    >
      <Icon
        name={iconName}
        size={48}
        color="var(--color-fg-subtle)"
        style={{ opacity: 0.6 }}
      />

      <div
        style={{
          fontSize: "var(--text-lg)",
          fontWeight: 600,
          color: "var(--color-fg)",
        }}
      >
        {title}
      </div>

      <div
        style={{
          fontSize: "var(--text-sm)",
          color: "var(--color-fg-muted)",
          maxWidth: 320,
          lineHeight: "var(--leading-base)",
        }}
      >
        {description}
      </div>

      {action && (
        <button
          onClick={action.onClick}
          style={{
            marginTop: "var(--space-2)",
            padding: "var(--space-2) var(--space-6)",
            background: "var(--color-accent)",
            color: "var(--color-accent-fg)",
            border: "none",
            borderRadius: "var(--radius-md)",
            cursor: "pointer",
            fontWeight: 500,
            fontSize: "var(--text-sm)",
            transition: `background var(--duration-fast) var(--ease-out)`,
          }}
          onMouseOver={(e) =>
            (e.currentTarget.style.background = "var(--color-accent-hover)")
          }
          onMouseOut={(e) =>
            (e.currentTarget.style.background = "var(--color-accent)")
          }
        >
          {action.label}
        </button>
      )}
    </div>
  );
}
