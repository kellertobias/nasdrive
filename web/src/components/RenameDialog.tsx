import { useState, useRef, useEffect } from "react";
import { Icon } from "./Icon";

interface RenameDialogProps {
  open: boolean;
  currentName: string;
  onClose: () => void;
  onRename: (newName: string) => void;
}

export function RenameDialog({
  open,
  currentName,
  onClose,
  onRename,
}: RenameDialogProps) {
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setName(currentName);
      setError("");
      setTimeout(() => {
        if (inputRef.current) {
          inputRef.current.focus();
          // Select the name part (without extension)
          const dotIdx = currentName.lastIndexOf(".");
          if (dotIdx > 0) {
            inputRef.current.setSelectionRange(0, dotIdx);
          } else {
            inputRef.current.select();
          }
        }
      }, 50);
    }
  }, [open, currentName]);

  if (!open) return null;

  const validate = (value: string): string => {
    if (!value.trim()) return "Name cannot be empty";
    if (value.includes("/") || value.includes("\\"))
      return "Name cannot contain slashes";
    if (value === "." || value === "..") return "Invalid name";
    if (value === currentName) return "Name unchanged";
    if (value.length > 255) return "Name too long";
    return "";
  };

  const handleSubmit = () => {
    const err = validate(name);
    if (err) {
      setError(err);
      return;
    }
    onRename(name.trim());
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.4)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 100,
      }}
      className="fade-in"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        style={{
          background: "var(--color-bg)",
          borderRadius: "var(--radius-xl)",
          boxShadow: "var(--shadow-xl)",
          padding: "var(--space-6)",
          width: 400,
          maxWidth: "90vw",
        }}
        className="slide-in"
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-3)",
            marginBottom: "var(--space-4)",
          }}
        >
          <Icon name="fileText" size={20} color="var(--color-fg-muted)" />
          <h2
            style={{ margin: 0, fontSize: "var(--text-lg)", fontWeight: 600 }}
          >
            Rename
          </h2>
        </div>

        <input
          ref={inputRef}
          type="text"
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            setError("");
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleSubmit();
            if (e.key === "Escape") onClose();
          }}
          style={{
            width: "100%",
            padding: "var(--space-2) var(--space-3)",
            border: `1px solid ${error ? "var(--color-danger)" : "var(--color-border)"}`,
            borderRadius: "var(--radius-md)",
            fontSize: "var(--text-sm)",
            outline: "none",
            background: "var(--color-bg)",
            color: "var(--color-fg)",
            transition: "border-color var(--duration-fast) var(--ease-out)",
            boxSizing: "border-box",
          }}
        />

        {error && (
          <div
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--color-danger)",
              marginTop: "var(--space-1)",
            }}
          >
            {error}
          </div>
        )}

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "var(--space-2)",
            marginTop: "var(--space-4)",
          }}
        >
          <button
            onClick={onClose}
            style={{
              padding: "var(--space-2) var(--space-4)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "transparent",
              color: "var(--color-fg)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
            }}
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            style={{
              padding: "var(--space-2) var(--space-4)",
              border: "none",
              borderRadius: "var(--radius-md)",
              background: "var(--color-accent)",
              color: "var(--color-accent-fg)",
              cursor: "pointer",
              fontWeight: 500,
              fontSize: "var(--text-sm)",
            }}
          >
            Rename
          </button>
        </div>
      </div>
    </div>
  );
}
