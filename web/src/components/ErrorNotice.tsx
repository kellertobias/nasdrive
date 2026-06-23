import { useEffect, useState } from "react";
import { Icon } from "./Icon";

export interface ErrorNoticeData {
  id: number;
  title: string;
  message: string;
  details: string;
}

interface ErrorDialogProps {
  error: ErrorNoticeData | null;
  onClose: () => void;
}

interface ErrorToastsProps {
  toasts: ErrorNoticeData[];
  onDismiss: (id: number) => void;
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const input = document.createElement("textarea");
    input.value = text;
    input.style.position = "fixed";
    input.style.opacity = "0";
    document.body.appendChild(input);
    input.select();
    document.execCommand("copy");
    document.body.removeChild(input);
  }
}

export function ErrorDialog({ error, onClose }: ErrorDialogProps) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setCopied(false);
  }, [error?.id]);

  if (!error) return null;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "var(--color-overlay)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        zIndex: 140,
        padding: "var(--space-4)",
      }}
      className="fade-in"
      role="dialog"
      aria-modal="true"
      aria-labelledby="error-dialog-title"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        style={{
          width: 560,
          maxWidth: "100%",
          maxHeight: "min(720px, 90vh)",
          background: "var(--color-bg)",
          border: "1px solid var(--color-border)",
          borderRadius: "var(--radius-lg)",
          boxShadow: "var(--shadow-xl)",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
        }}
        className="slide-in"
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-3)",
            padding: "var(--space-5) var(--space-5) var(--space-3)",
          }}
        >
          <Icon name="alertTriangle" size={22} color="var(--color-danger)" />
          <h2
            id="error-dialog-title"
            style={{ margin: 0, fontSize: "var(--text-lg)", fontWeight: 600 }}
          >
            {error.title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            title="Close"
            style={{
              marginLeft: "auto",
              border: "none",
              background: "transparent",
              color: "var(--color-fg-muted)",
              cursor: "pointer",
              padding: "var(--space-1)",
              display: "inline-flex",
            }}
          >
            <Icon name="x" size={18} />
          </button>
        </div>

        <div style={{ padding: "0 var(--space-5)", overflow: "auto" }}>
          <p
            style={{
              margin: "0 0 var(--space-3)",
              color: "var(--color-fg)",
              fontSize: "var(--text-sm)",
              lineHeight: "var(--leading-base)",
              whiteSpace: "pre-wrap",
            }}
          >
            {error.message}
          </p>

          <pre
            style={{
              margin: 0,
              padding: "var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-bg-muted)",
              color: "var(--color-fg)",
              fontFamily: "var(--font-mono)",
              fontSize: "var(--text-xs)",
              lineHeight: "var(--leading-sm)",
              whiteSpace: "pre-wrap",
              overflowWrap: "anywhere",
            }}
          >
            {error.details}
          </pre>
        </div>

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: "var(--space-2)",
            padding: "var(--space-4) var(--space-5) var(--space-5)",
          }}
        >
          <button
            type="button"
            onClick={async () => {
              await copyText(error.details);
              setCopied(true);
            }}
            style={{
              padding: "var(--space-2) var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "transparent",
              color: "var(--color-fg)",
              cursor: "pointer",
              fontSize: "var(--text-sm)",
            }}
          >
            {copied ? "Copied" : "Copy error details"}
          </button>
          <button
            type="button"
            onClick={onClose}
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
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

export function ErrorToasts({ toasts, onDismiss }: ErrorToastsProps) {
  return (
    <div
      style={{
        position: "fixed",
        right: "var(--space-4)",
        bottom: "var(--space-4)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-2)",
        zIndex: 130,
        width: 360,
        maxWidth: "calc(100vw - 32px)",
      }}
    >
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="status"
          className="slide-in"
          style={{
            border: "1px solid var(--color-border)",
            borderLeft: "3px solid var(--color-warning)",
            borderRadius: "var(--radius-md)",
            boxShadow: "var(--shadow-lg)",
            background: "var(--color-bg)",
            padding: "var(--space-3)",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "flex-start",
              gap: "var(--space-2)",
            }}
          >
            <Icon name="alertTriangle" size={17} color="var(--color-warning)" />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div
                style={{
                  fontSize: "var(--text-sm)",
                  fontWeight: 600,
                  marginBottom: "var(--space-1)",
                }}
              >
                {toast.title}
              </div>
              <div
                style={{
                  color: "var(--color-fg-muted)",
                  fontSize: "var(--text-xs)",
                  lineHeight: "var(--leading-sm)",
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                }}
              >
                {toast.message}
              </div>
              <button
                type="button"
                onClick={() => {
                  void copyText(toast.details);
                }}
                style={{
                  marginTop: "var(--space-2)",
                  border: "none",
                  background: "transparent",
                  color: "var(--color-accent)",
                  padding: 0,
                  cursor: "pointer",
                  fontSize: "var(--text-xs)",
                  fontWeight: 600,
                }}
              >
                Copy error details
              </button>
            </div>
            <button
              type="button"
              onClick={() => onDismiss(toast.id)}
              aria-label="Dismiss"
              title="Dismiss"
              style={{
                border: "none",
                background: "transparent",
                color: "var(--color-fg-muted)",
                cursor: "pointer",
                padding: 0,
                display: "inline-flex",
              }}
            >
              <Icon name="x" size={16} />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
