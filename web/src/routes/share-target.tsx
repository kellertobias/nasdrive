import { createFileRoute, Link } from "@tanstack/react-router";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import api from "../api/client";
import { AppLogo } from "../components/AppLogo";
import { Icon } from "../components/Icon";
import {
  deleteIncomingShare,
  filesFromIncomingShare,
  readIncomingShare,
  rememberPendingShareId,
  saveShareTarget,
  storedShareTarget,
} from "../lib/shareTarget";

export const Route = createFileRoute("/share-target")({
  validateSearch: (search: Record<string, unknown>) => ({
    shareId: typeof search.shareId === "string" ? search.shareId : "",
  }),
  component: ShareTargetPage,
});

function ShareTargetPage() {
  const { shareId } = Route.useSearch();
  const [targetRoot, setTargetRoot] = useState(storedShareTarget().root);
  const [targetPath, setTargetPath] = useState(storedShareTarget().path);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const { data: user } = useQuery({
    queryKey: ["me"],
    queryFn: api.me,
    retry: false,
  });
  const { data: rootsData } = useQuery({
    queryKey: ["roots"],
    queryFn: api.roots,
    enabled: !!user,
  });
  const { data: incomingShare, isLoading: isLoadingShare } = useQuery({
    queryKey: ["incoming-share", shareId],
    queryFn: () => readIncomingShare(shareId),
    enabled: !!shareId,
  });

  const writableRoots = useMemo(
    () => (rootsData?.roots || []).filter((root) => root.caps.write),
    [rootsData],
  );

  useEffect(() => {
    if (!targetRoot && writableRoots.length > 0) {
      const homeRoot =
        writableRoots.find((root) => root.kind === "home") || writableRoots[0];
      setTargetRoot(homeRoot.key);
    }
  }, [targetRoot, writableRoots]);

  useEffect(() => {
    if (!user && shareId) rememberPendingShareId(shareId);
  }, [shareId, user]);

  const files = incomingShare ? filesFromIncomingShare(incomingShare) : [];

  const uploadMutation = useMutation({
    mutationFn: async () => {
      if (!incomingShare) throw new Error("No shared content was found.");
      if (!targetRoot) throw new Error("Choose an upload destination.");

      saveShareTarget(targetRoot, targetPath.trim());
      setStatus("Uploading shared files...");
      const handle = api.upload(targetRoot, targetPath.trim(), files);
      await handle.promise;
      await deleteIncomingShare(incomingShare.id);
    },
    onSuccess: () => {
      setError("");
      setStatus("Shared files uploaded.");
    },
    onError: (err) => {
      setStatus("");
      setError(err instanceof Error ? err.message : String(err));
    },
  });

  if (!user) {
    return (
      <CenteredShell>
        <AppLogo size={64} wordmarkSize={24} />
        <p style={mutedTextStyle}>
          Sign in to upload the shared item to your NASDrive folder.
        </p>
        <Link to="/" style={primaryLinkStyle}>
          Sign in
        </Link>
      </CenteredShell>
    );
  }

  const selectedRoot = writableRoots.find((root) => root.key === targetRoot);
  const openPath = `/r/${encodeURIComponent(targetRoot)}/${targetPath.trim()}`;

  return (
    <CenteredShell>
      <AppLogo size={56} wordmarkSize={22} />
      <section style={panelStyle}>
        <div style={{ display: "grid", gap: 6 }}>
          <h1 style={headingStyle}>Upload shared files</h1>
          <p style={mutedTextStyle}>
            {isLoadingShare
              ? "Loading shared content..."
              : incomingShare
                ? `${files.length} item${files.length === 1 ? "" : "s"} ready`
                : "No shared content is waiting."}
          </p>
        </div>

        {files.length > 0 && (
          <div style={fileListStyle}>
            {files.slice(0, 5).map((file) => (
              <div key={`${file.name}-${file.size}`} style={fileRowStyle}>
                <Icon name="file" size={16} />
                <span style={{ minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
                  {file.name}
                </span>
              </div>
            ))}
            {files.length > 5 && (
              <div style={{ ...fileRowStyle, color: "var(--color-fg-muted)" }}>
                +{files.length - 5} more
              </div>
            )}
          </div>
        )}

        <label style={labelStyle}>
          Folder root
          <select
            value={targetRoot}
            onChange={(event) => setTargetRoot(event.target.value)}
            style={inputStyle}
          >
            {writableRoots.map((root) => (
              <option key={root.key} value={root.key}>
                {root.display_name}
              </option>
            ))}
          </select>
        </label>

        <label style={labelStyle}>
          Folder path
          <input
            value={targetPath}
            onChange={(event) => setTargetPath(event.target.value)}
            placeholder="Uploads/Shared"
            style={inputStyle}
          />
        </label>

        <button
          type="button"
          disabled={
            !incomingShare ||
            files.length === 0 ||
            !selectedRoot ||
            uploadMutation.isPending
          }
          onClick={() => uploadMutation.mutate()}
          style={buttonStyle}
        >
          <Icon name="upload" size={16} />
          {uploadMutation.isPending ? "Uploading..." : "Upload"}
        </button>

        {status && <p style={successStyle}>{status}</p>}
        {error && <p role="alert" style={errorStyle}>{error}</p>}
        {uploadMutation.isSuccess && selectedRoot && (
          <Link to={openPath} style={secondaryLinkStyle}>
            Open destination
          </Link>
        )}
      </section>
    </CenteredShell>
  );
}

function CenteredShell({ children }: { children: React.ReactNode }) {
  return (
    <main
      style={{
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "var(--space-5)",
        padding: "var(--space-4)",
        background: "var(--color-bg)",
      }}
    >
      {children}
    </main>
  );
}

const panelStyle: React.CSSProperties = {
  width: "min(440px, 100%)",
  display: "grid",
  gap: "var(--space-4)",
  padding: "var(--space-5)",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg-subtle)",
};

const headingStyle: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-xl)",
  lineHeight: "var(--leading-xl)",
  fontWeight: 700,
  letterSpacing: "var(--tracking-normal)",
};

const mutedTextStyle: React.CSSProperties = {
  margin: 0,
  color: "var(--color-fg-muted)",
  fontSize: "var(--text-sm)",
};

const labelStyle: React.CSSProperties = {
  display: "grid",
  gap: "var(--space-2)",
  color: "var(--color-fg-muted)",
  fontSize: "var(--text-sm)",
  fontWeight: 600,
};

const inputStyle: React.CSSProperties = {
  width: "100%",
  padding: "10px 12px",
  border: "1px solid var(--color-border)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg)",
  color: "var(--color-fg)",
  font: "inherit",
};

const buttonStyle: React.CSSProperties = {
  minHeight: 42,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 8,
  border: 0,
  borderRadius: "var(--radius-md)",
  background: "var(--color-accent)",
  color: "var(--color-accent-fg)",
  fontWeight: 700,
  cursor: "pointer",
};

const primaryLinkStyle: React.CSSProperties = {
  ...buttonStyle,
  width: "min(320px, 100%)",
  textDecoration: "none",
};

const secondaryLinkStyle: React.CSSProperties = {
  color: "var(--color-accent)",
  fontWeight: 700,
  textDecoration: "none",
  textAlign: "center",
};

const fileListStyle: React.CSSProperties = {
  display: "grid",
  gap: 6,
  padding: "var(--space-3)",
  border: "1px solid var(--color-border-muted)",
  borderRadius: "var(--radius-md)",
  background: "var(--color-bg)",
};

const fileRowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  minWidth: 0,
  fontSize: "var(--text-sm)",
};

const successStyle: React.CSSProperties = {
  margin: 0,
  color: "var(--color-success)",
  fontSize: "var(--text-sm)",
  fontWeight: 600,
};

const errorStyle: React.CSSProperties = {
  margin: 0,
  color: "var(--color-danger)",
  fontSize: "var(--text-sm)",
  fontWeight: 600,
};
