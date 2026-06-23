import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icon";
import { ICONS } from "../lib/icons";

interface ContextMenuItem {
  label: string;
  iconName: keyof typeof ICONS;
  onClick: () => void;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ x, y });

  // Adjust position to keep menu on screen
  useEffect(() => {
    if (menuRef.current) {
      const rect = menuRef.current.getBoundingClientRect();
      let adjX = x;
      let adjY = y;

      if (adjX + rect.width > window.innerWidth - 8) {
        adjX = window.innerWidth - rect.width - 8;
      }
      if (adjY + rect.height > window.innerHeight - 8) {
        adjY = window.innerHeight - rect.height - 8;
      }

      setPosition({ x: adjX, y: adjY });
    }
  }, [x, y]);

  // Close on click outside or Escape
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };

    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      ref={menuRef}
      role="menu"
      className="fade-in"
      style={{
        position: "fixed",
        left: position.x,
        top: position.y,
        background: "var(--color-bg)",
        border: "1px solid var(--color-border)",
        borderRadius: "var(--radius-lg)",
        boxShadow: "var(--shadow-lg)",
        padding: "var(--space-1)",
        minWidth: 180,
        zIndex: 200,
      }}
    >
      {items.map((item, i) => {
        if (item.separator) {
          return (
            <div
              key={`sep-${i}`}
              style={{
                height: 1,
                background: "var(--color-border-muted)",
                margin: "var(--space-1) var(--space-2)",
              }}
            />
          );
        }

        return (
          <button
            key={item.label}
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              if (!item.disabled) {
                item.onClick();
                onClose();
              }
            }}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              width: "100%",
              padding: "var(--space-2) var(--space-3)",
              border: "none",
              background: "transparent",
              color: item.danger
                ? "var(--color-danger)"
                : item.disabled
                  ? "var(--color-fg-subtle)"
                  : "var(--color-fg)",
              cursor: item.disabled ? "default" : "pointer",
              fontSize: "var(--text-sm)",
              borderRadius: "var(--radius-md)",
              textAlign: "left",
              opacity: item.disabled ? 0.5 : 1,
              transition: "background var(--duration-fast) var(--ease-out)",
            }}
            onMouseOver={(e) => {
              if (!item.disabled) {
                e.currentTarget.style.background = item.danger
                  ? "var(--color-danger-muted)"
                  : "var(--color-bg-muted)";
              }
            }}
            onMouseOut={(e) => {
              e.currentTarget.style.background = "transparent";
            }}
          >
            <Icon
              name={item.iconName}
              size={16}
              color={
                item.danger ? "var(--color-danger)" : "var(--color-fg-muted)"
              }
            />
            {item.label}
          </button>
        );
      })}
    </div>
  );
}

export type { ContextMenuItem };
