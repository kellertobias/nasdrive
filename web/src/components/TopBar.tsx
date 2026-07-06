import api from "../api/client";
import type { SearchResult, TransferJob, UserInfo } from "../api/client";
import { useViewStore } from "../state/view";
import { useState, useRef, useEffect, useMemo } from "react";
import type { CSSProperties } from "react";
import { FileIcon, Icon } from "./Icon";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { formatDate, formatFileSize, getFileIcon } from "../lib/icons";
import { AppLogo } from "./AppLogo";
import { transferProgressPercent } from "../lib/transferJobs";

interface TopBarProps {
  user: UserInfo | null;
  currentRoot?: string;
  onMobileSidebarToggle?: () => void;
}

function estimateRemainingMs(
  jobs: Array<{
    created_at: number;
    total_bytes: number;
    transferred_bytes: number;
    total_entries: number;
    completed_entries: number;
  }>,
) {
  const now = Date.now();
  const estimates = jobs
    .map((job) => {
      const total = job.total_bytes > 0 ? job.total_bytes : job.total_entries;
      const done =
        job.total_bytes > 0 ? job.transferred_bytes : job.completed_entries;
      const elapsed = Math.max(0, now - job.created_at);
      if (total <= 0 || done <= 0 || done >= total || elapsed < 1000)
        return null;
      return ((total - done) / done) * elapsed;
    })
    .filter(
      (value): value is number => value !== null && Number.isFinite(value),
    );

  if (estimates.length === 0) return null;
  return Math.max(...estimates);
}

function formatRemainingTime(ms: number | null) {
  if (ms === null) return "estimating";
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s left`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m left`;
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes > 0 ? `${hours}h ${restMinutes}m left` : `${hours}h left`;
}

export function TopBar({ user, onMobileSidebarToggle }: TopBarProps) {
  const { toggleSidebar } = useViewStore();
  const navigate = useNavigate();
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const queryClient = useQueryClient();
  const [menuOpen, setMenuOpen] = useState(false);
  const [transferMenuOpen, setTransferMenuOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [debouncedSearchQuery, setDebouncedSearchQuery] = useState("");
  const menuRef = useRef<HTMLDivElement>(null);
  const transferMenuRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLDivElement>(null);
  const seenFinishedJobs = useRef<Set<string>>(new Set());
  const buildDate =
    user?.build.date && user.build.date !== "unknown"
      ? new Date(user.build.date).toLocaleString()
      : "unknown";
  const buildCommit =
    user?.build.commit && user.build.commit !== "unknown"
      ? user.build.commit.slice(0, 12)
      : "unknown";
  const serverStartedAt =
    user?.build.started_at && user.build.started_at !== "unknown"
      ? new Date(user.build.started_at).toLocaleString()
      : "unknown";
  const hasSidebar = pathname.startsWith("/r/");

  const navigateToFiles = () => {
    navigate({ to: "/" });
  };

  const handleMenuClick = () => {
    if (
      hasSidebar &&
      onMobileSidebarToggle &&
      window.matchMedia("(max-width: 720px)").matches
    ) {
      onMobileSidebarToggle();
      return;
    }
    if (hasSidebar) {
      toggleSidebar();
      return;
    }
    navigateToFiles();
  };

  const { data: transferJobData } = useQuery({
    queryKey: ["transfer-jobs"],
    queryFn: api.transferJobs,
    enabled: Boolean(user),
    refetchInterval: user ? 1000 : false,
    staleTime: 1000,
  });

  const transferJobs = useMemo(
    () => transferJobData?.jobs ?? [],
    [transferJobData?.jobs],
  );
  const activeTransferJobs = transferJobs.filter(
    (job) => job.status === "queued" || job.status === "running",
  );
  const pausedTransferJobs = transferJobs.filter(
    (job) => job.status === "paused_needs_confirmation",
  );
  const visibleTransferJobs = [...pausedTransferJobs, ...activeTransferJobs];
  const activeTransferCount = activeTransferJobs.length;
  const activeOperation = activeTransferJobs.some(
    (job) => job.operation === "delete",
  )
    ? "Deleting"
    : activeTransferJobs.some((job) => job.operation === "copy")
      ? "Copying"
      : "Moving";
  const totalBytes = activeTransferJobs.reduce(
    (sum, job) => sum + job.total_bytes,
    0,
  );
  const transferredBytes = activeTransferJobs.reduce(
    (sum, job) => sum + job.transferred_bytes,
    0,
  );
  const totalEntries = activeTransferJobs.reduce(
    (sum, job) => sum + job.total_entries,
    0,
  );
  const completedEntries = activeTransferJobs.reduce(
    (sum, job) => sum + job.completed_entries,
    0,
  );
  const progressPct =
    totalBytes > 0
      ? Math.min(100, Math.round((transferredBytes / totalBytes) * 100))
      : totalEntries > 0
        ? Math.min(100, Math.round((completedEntries / totalEntries) * 100))
        : 0;
  const progressLabel =
    totalBytes > 0
      ? `${formatFileSize(transferredBytes)} / ${formatFileSize(totalBytes)}`
      : totalEntries > 0
        ? `${completedEntries} / ${totalEntries} items`
        : "Preparing";
  const remainingLabel = formatRemainingTime(
    estimateRemainingMs(activeTransferJobs),
  );
  const cancelTransferMutation = useMutation({
    mutationFn: (jobId: string) => api.cancelFileJob(jobId),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["transfer-jobs"] });
    },
  });
  const resumeTransferMutation = useMutation({
    mutationFn: (jobId: string) => api.resumeFileJob(jobId),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["transfer-jobs"] });
    },
  });
  const cleanupTransferMutation = useMutation({
    mutationFn: (jobId: string) => api.cleanupFileJob(jobId),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["transfer-jobs"] });
    },
  });

  useEffect(() => {
    const handle = window.setTimeout(() => {
      setDebouncedSearchQuery(searchQuery.trim());
    }, 250);
    return () => window.clearTimeout(handle);
  }, [searchQuery]);

  const searchEnabled = Boolean(user && debouncedSearchQuery.length >= 2);
  const {
    data: searchData,
    isFetching: searchFetching,
    error: searchError,
  } = useQuery({
    queryKey: ["search", debouncedSearchQuery],
    queryFn: () => api.search(debouncedSearchQuery, 50),
    enabled: searchEnabled,
    staleTime: 5_000,
  });

  const openSearchResult = (result: SearchResult) => {
    const targetPath = result.entry.is_dir ? result.path : result.parent_path;
    setSearchOpen(false);
    if (!result.entry.is_dir) {
      useViewStore.getState().select(result.path);
    } else {
      useViewStore.getState().clearSelection();
    }
    navigate({
      to: "/r/$root/$",
      params: { root: result.root, _splat: targetPath },
    });
  };

  // Close menu on click outside
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
      if (
        transferMenuRef.current &&
        !transferMenuRef.current.contains(e.target as Node)
      ) {
        setTransferMenuOpen(false);
      }
      if (searchRef.current && !searchRef.current.contains(e.target as Node)) {
        setSearchOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  useEffect(() => {
    for (const job of transferJobs) {
      if (
        (job.status === "done" || job.status === "error") &&
        !seenFinishedJobs.current.has(job.id)
      ) {
        seenFinishedJobs.current.add(job.id);
        queryClient.invalidateQueries({ queryKey: ["listing"] });
        queryClient.invalidateQueries({ queryKey: ["tree"] });
        queryClient.invalidateQueries({ queryKey: ["roots"] });
      }
    }
  }, [queryClient, transferJobs]);

  return (
    <header
      className="app-topbar"
      style={{
        display: "flex",
        alignItems: "center",
        height: 52,
        padding: "0 var(--space-4)",
        borderBottom: "1px solid var(--color-border)",
        background: "var(--color-bg)",
        gap: "var(--space-3)",
        flexShrink: 0,
      }}
    >
      {/* Sidebar toggle */}
      <button
        onClick={handleMenuClick}
        title={hasSidebar ? "Toggle sidebar" : "Back to files"}
        aria-label={hasSidebar ? "Toggle sidebar" : "Back to files"}
        style={{
          background: "none",
          border: "none",
          cursor: "pointer",
          padding: "var(--space-1)",
          borderRadius: "var(--radius-sm)",
          color: "var(--color-fg-muted)",
          display: "flex",
          alignItems: "center",
          transition: `color var(--duration-fast) var(--ease-out)`,
        }}
        onMouseOver={(e) => (e.currentTarget.style.color = "var(--color-fg)")}
        onMouseOut={(e) =>
          (e.currentTarget.style.color = "var(--color-fg-muted)")
        }
      >
        <Icon name="menu" size={20} />
      </button>

      {/* Logo */}
      <button
        type="button"
        onClick={hasSidebar ? undefined : navigateToFiles}
        title={hasSidebar ? undefined : "Back to files"}
        aria-label="nasfiles"
        style={{
          background: "none",
          border: "none",
          padding: 0,
          color: "var(--color-fg)",
          cursor: hasSidebar ? "default" : "pointer",
          display: "inline-flex",
          alignItems: "center",
        }}
      >
        <AppLogo size={26} wordmarkSize={16} compact />
      </button>

      {user && (
        <div
          ref={searchRef}
          className="app-topbar-search"
          style={{ position: "relative", flex: "0 1 520px", minWidth: 220 }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              height: 34,
              padding: "0 var(--space-3)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-bg-muted)",
              color: "var(--color-fg-muted)",
            }}
          >
            <Icon name="search" size={16} />
            <input
              value={searchQuery}
              onFocus={() => setSearchOpen(true)}
              onChange={(e) => {
                setSearchQuery(e.target.value);
                setSearchOpen(true);
              }}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setSearchOpen(false);
                  e.currentTarget.blur();
                }
              }}
              placeholder="Search files"
              style={{
                width: "100%",
                minWidth: 0,
                border: "none",
                outline: "none",
                background: "transparent",
                color: "var(--color-fg)",
                fontSize: "var(--text-sm)",
              }}
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => {
                  setSearchQuery("");
                  setDebouncedSearchQuery("");
                }}
                title="Clear search"
                aria-label="Clear search"
                style={iconButtonStyle}
              >
                <Icon name="x" size={14} />
              </button>
            )}
          </div>

          {searchOpen && searchQuery.trim().length > 0 && (
            <SearchResultsPanel
              query={searchQuery.trim()}
              results={searchData?.results ?? []}
              indexReady={searchData?.index_ready ?? true}
              liveComplete={searchData?.live_complete ?? true}
              isLoading={searchFetching}
              error={searchError}
              onOpen={openSearchResult}
            />
          )}
        </div>
      )}

      <div style={{ flex: 1 }} />

      {visibleTransferJobs.length > 0 && (
        <div ref={transferMenuRef} style={{ position: "relative" }}>
          <button
            type="button"
            onClick={() => setTransferMenuOpen((open) => !open)}
            title="File operation details"
            aria-haspopup="menu"
            aria-expanded={transferMenuOpen}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              minWidth: 220,
              maxWidth: 300,
              padding: "var(--space-1) var(--space-2)",
              border: "1px solid var(--color-border)",
              borderRadius: "var(--radius-md)",
              background: "var(--color-bg-muted)",
              color: "var(--color-fg)",
              cursor: "pointer",
              textAlign: "left",
            }}
          >
            <Icon
              name={pausedTransferJobs.length > 0 ? "alertTriangle" : "upload"}
              size={15}
              color="var(--color-accent)"
            />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                  gap: "var(--space-2)",
                  marginBottom: 3,
                }}
              >
                <span
                  style={{
                    fontSize: "var(--text-xs)",
                    fontWeight: 600,
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  {activeTransferCount > 0
                    ? `${activeOperation} ${activeTransferCount > 1 ? `${activeTransferCount} jobs` : `${activeTransferJobs[0]?.paths.length ?? 0} item(s)`}`
                    : `${pausedTransferJobs.length} job${pausedTransferJobs.length === 1 ? "" : "s"} need attention`}
                </span>
                <span
                  className="app-topbar-transfer-percent tabular-nums"
                  style={{
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-muted)",
                  }}
                >
                  {progressPct}%
                </span>
              </div>
              <div
                style={{
                  height: 3,
                  borderRadius: 2,
                  background: "var(--color-border)",
                  overflow: "hidden",
                  marginBottom: 3,
                }}
              >
                <div
                  style={{
                    height: "100%",
                    width: `${progressPct}%`,
                    borderRadius: 2,
                    background: "var(--color-accent)",
                    transition: "width 200ms ease-out",
                  }}
                />
              </div>
              <div
                style={{
                  fontSize: "var(--text-xs)",
                  color: "var(--color-fg-subtle)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {activeTransferCount > 0
                  ? `${progressLabel} · ${remainingLabel}`
                  : "Resume or clean up recovered work"}
              </div>
            </div>
            <Icon name="chevronDown" size={14} color="var(--color-fg-subtle)" />
          </button>

          {transferMenuOpen && (
            <div
              role="menu"
              style={{
                position: "absolute",
                top: "100%",
                right: 0,
                marginTop: "var(--space-1)",
                width: 420,
                maxWidth: "calc(100vw - 24px)",
                maxHeight: "min(420px, calc(100vh - 80px))",
                overflowY: "auto",
                background: "var(--color-bg)",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-lg)",
                boxShadow: "var(--shadow-lg)",
                padding: "var(--space-2)",
                zIndex: 60,
              }}
              className="fade-in"
            >
              <div
                style={{
                  padding: "var(--space-2)",
                  borderBottom: "1px solid var(--color-border-muted)",
                  marginBottom: "var(--space-1)",
                  fontSize: "var(--text-sm)",
                  fontWeight: 600,
                }}
              >
                File operations
              </div>
              {visibleTransferJobs.map((job) => (
                <TransferJobMenuItem
                  key={job.id}
                  job={job}
                  cancelling={
                    cancelTransferMutation.variables === job.id &&
                    cancelTransferMutation.isPending
                  }
                  resuming={
                    resumeTransferMutation.variables === job.id &&
                    resumeTransferMutation.isPending
                  }
                  cleaning={
                    cleanupTransferMutation.variables === job.id &&
                    cleanupTransferMutation.isPending
                  }
                  onCancel={() => cancelTransferMutation.mutate(job.id)}
                  onResume={() => resumeTransferMutation.mutate(job.id)}
                  onCleanup={() => cleanupTransferMutation.mutate(job.id)}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {/* User menu */}
      {user && (
        <div ref={menuRef} style={{ position: "relative" }}>
          <button
            onClick={() => setMenuOpen(!menuOpen)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              background: "none",
              border: "none",
              cursor: "pointer",
              padding: "var(--space-1) var(--space-2)",
              borderRadius: "var(--radius-md)",
              transition: `background var(--duration-fast) var(--ease-out)`,
              color: "var(--color-fg)",
            }}
            onMouseOver={(e) =>
              (e.currentTarget.style.background = "var(--color-bg-muted)")
            }
            onMouseOut={(e) =>
              (e.currentTarget.style.background = "transparent")
            }
          >
            {user.picture_url ? (
              <img
                src={user.picture_url}
                alt=""
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: "var(--radius-full)",
                  objectFit: "cover",
                }}
              />
            ) : (
              <div
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: "var(--radius-full)",
                  background: "var(--color-accent)",
                  color: "var(--color-accent-fg)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  fontSize: "var(--text-sm)",
                  fontWeight: 600,
                }}
              >
                {user.display_name.charAt(0).toUpperCase()}
              </div>
            )}
            <span
              className="app-topbar-user-name"
              style={{ fontSize: "var(--text-sm)", fontWeight: 500 }}
            >
              {user.display_name}
            </span>
            <Icon name="chevronDown" size={14} color="var(--color-fg-subtle)" />
          </button>

          {/* Dropdown menu */}
          {menuOpen && (
            <div
              style={{
                position: "absolute",
                top: "100%",
                right: 0,
                marginTop: "var(--space-1)",
                background: "var(--color-bg)",
                border: "1px solid var(--color-border)",
                borderRadius: "var(--radius-lg)",
                boxShadow: "var(--shadow-lg)",
                minWidth: 200,
                padding: "var(--space-1)",
                zIndex: 50,
              }}
              className="fade-in"
            >
              <div
                style={{
                  padding: "var(--space-2) var(--space-3)",
                  borderBottom: "1px solid var(--color-border-muted)",
                  marginBottom: "var(--space-1)",
                }}
              >
                <div
                  style={{
                    fontWeight: 500,
                    fontSize: "var(--text-sm)",
                    color: "var(--color-fg)",
                  }}
                >
                  {user.display_name}
                </div>
                <div
                  style={{
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-subtle)",
                  }}
                >
                  {user.username}
                </div>
              </div>

              {user.is_admin && (
                <a
                  href="/admin"
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-2)",
                    width: "100%",
                    padding: "var(--space-2) var(--space-3)",
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    borderRadius: "var(--radius-md)",
                    fontSize: "var(--text-sm)",
                    color: "var(--color-fg)",
                    textDecoration: "none",
                    transition: `background var(--duration-fast) var(--ease-out)`,
                  }}
                  onMouseOver={(e) =>
                    (e.currentTarget.style.background = "var(--color-bg-muted)")
                  }
                  onMouseOut={(e) =>
                    (e.currentTarget.style.background = "transparent")
                  }
                >
                  <Icon name="settings" size={16} />
                  Administration
                </a>
              )}

              <a
                href="/profile"
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  width: "100%",
                  padding: "var(--space-2) var(--space-3)",
                  background: "none",
                  border: "none",
                  cursor: "pointer",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                  color: "var(--color-fg)",
                  textDecoration: "none",
                  transition: `background var(--duration-fast) var(--ease-out)`,
                }}
                onMouseOver={(e) =>
                  (e.currentTarget.style.background = "var(--color-bg-muted)")
                }
                onMouseOut={(e) =>
                  (e.currentTarget.style.background = "transparent")
                }
              >
                <Icon name="user" size={16} />
                Profile
              </a>

              <button
                onClick={() => {
                  api.logout().catch(() => {});
                  window.location.href = "/";
                }}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  width: "100%",
                  padding: "var(--space-2) var(--space-3)",
                  background: "none",
                  border: "none",
                  cursor: "pointer",
                  borderRadius: "var(--radius-md)",
                  fontSize: "var(--text-sm)",
                  color: "var(--color-danger)",
                  transition: `background var(--duration-fast) var(--ease-out)`,
                  textAlign: "left",
                }}
                onMouseOver={(e) =>
                  (e.currentTarget.style.background =
                    "var(--color-danger-muted)")
                }
                onMouseOut={(e) =>
                  (e.currentTarget.style.background = "transparent")
                }
              >
                <Icon name="logout" size={16} />
                Sign out
              </button>

              <div
                style={{
                  padding: "var(--space-2) var(--space-3)",
                  borderTop: "1px solid var(--color-border-muted)",
                  marginTop: "var(--space-1)",
                  color: "var(--color-fg-subtle)",
                  fontSize: "var(--text-xs)",
                  lineHeight: 1.5,
                }}
              >
                <div>Build {buildDate}</div>
                <div>Restarted {serverStartedAt}</div>
                <div style={{ fontFamily: "monospace" }}>{buildCommit}</div>
              </div>
            </div>
          )}
        </div>
      )}
    </header>
  );
}

interface SearchResultsPanelProps {
  query: string;
  results: SearchResult[];
  indexReady: boolean;
  liveComplete: boolean;
  isLoading: boolean;
  error: unknown;
  onOpen: (result: SearchResult) => void;
}

function SearchResultsPanel({
  query,
  results,
  indexReady,
  liveComplete,
  isLoading,
  error,
  onOpen,
}: SearchResultsPanelProps) {
  const tooShort = query.length < 2;

  return (
    <div
      role="listbox"
      style={{
        position: "absolute",
        top: "calc(100% + var(--space-1))",
        left: 0,
        width: 560,
        maxWidth: "calc(100vw - 24px)",
        maxHeight: "min(520px, calc(100vh - 76px))",
        overflowY: "auto",
        background: "var(--color-bg)",
        border: "1px solid var(--color-border)",
        borderRadius: "var(--radius-lg)",
        boxShadow: "var(--shadow-lg)",
        padding: "var(--space-2)",
        zIndex: 70,
      }}
      className="fade-in"
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: "var(--space-3)",
          padding: "var(--space-2)",
          borderBottom: "1px solid var(--color-border-muted)",
          marginBottom: "var(--space-1)",
        }}
      >
        <span
          style={{
            fontSize: "var(--text-sm)",
            fontWeight: 600,
            color: "var(--color-fg)",
          }}
        >
          Search results
        </span>
        {!tooShort && (isLoading || !indexReady || !liveComplete) && (
          <span
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--color-fg-subtle)",
            }}
          >
            {isLoading
              ? "Checking files"
              : !indexReady
                ? "Index warming up"
                : "Live check limited"}
          </span>
        )}
      </div>

      {tooShort && (
        <SearchPanelMessage icon="search" text="Type at least 2 characters" />
      )}
      {!tooShort && Boolean(error) && (
        <SearchPanelMessage
          icon="alertTriangle"
          text={error instanceof Error ? error.message : "Search failed"}
        />
      )}
      {!tooShort && !error && !isLoading && results.length === 0 && (
        <SearchPanelMessage icon="folderSearch" text="No matching files" />
      )}

      {!tooShort &&
        results.map((result) => {
          const icon = getFileIcon(result.entry);
          const location = result.parent_path
            ? `${result.root_display_name} / ${result.parent_path}`
            : result.root_display_name;

          return (
            <button
              key={`${result.root}:${result.path}`}
              type="button"
              role="option"
              onClick={() => onOpen(result)}
              style={{
                display: "grid",
                gridTemplateColumns: "32px minmax(0, 1fr) auto",
                alignItems: "center",
                gap: "var(--space-2)",
                width: "100%",
                padding: "var(--space-2)",
                border: "none",
                borderRadius: "var(--radius-md)",
                background: "transparent",
                color: "var(--color-fg)",
                cursor: "pointer",
                textAlign: "left",
              }}
              onMouseOver={(e) =>
                (e.currentTarget.style.background = "var(--color-bg-muted)")
              }
              onMouseOut={(e) =>
                (e.currentTarget.style.background = "transparent")
              }
            >
              <FileIcon svg={icon.svg} color={icon.color} size={22} />
              <span style={{ minWidth: 0 }}>
                <span
                  style={{
                    display: "block",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    fontSize: "var(--text-sm)",
                    fontWeight: 500,
                  }}
                >
                  {result.entry.name}
                </span>
                <span
                  style={{
                    display: "block",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    fontSize: "var(--text-xs)",
                    color: "var(--color-fg-subtle)",
                    marginTop: 2,
                  }}
                >
                  {location}
                </span>
              </span>
              <span
                style={{
                  justifySelf: "end",
                  color: "var(--color-fg-subtle)",
                  fontSize: "var(--text-xs)",
                  whiteSpace: "nowrap",
                }}
              >
                {result.entry.is_dir
                  ? formatDate(result.entry.modified_at)
                  : formatFileSize(result.entry.size)}
              </span>
            </button>
          );
        })}
    </div>
  );
}

function SearchPanelMessage({
  icon,
  text,
}: {
  icon: "search" | "alertTriangle" | "folderSearch";
  text: string;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-2)",
        padding: "var(--space-4)",
        color: "var(--color-fg-subtle)",
        fontSize: "var(--text-sm)",
      }}
    >
      <Icon name={icon} size={16} />
      {text}
    </div>
  );
}

function TransferJobMenuItem({
  job,
  cancelling,
  resuming,
  cleaning,
  onCancel,
  onResume,
  onCleanup,
}: {
  job: TransferJob;
  cancelling: boolean;
  resuming: boolean;
  cleaning: boolean;
  onCancel: () => void;
  onResume: () => void;
  onCleanup: () => void;
}) {
  const percent = transferProgressPercent([job]);
  const operationLabel =
    job.operation === "copy"
      ? "Copy"
      : job.operation === "move"
        ? "Move"
        : "Delete";
  const title = `${operationLabel} ${job.paths.length} item${job.paths.length === 1 ? "" : "s"}`;
  const detail =
    job.total_bytes > 0
      ? `${formatFileSize(job.transferred_bytes)} / ${formatFileSize(job.total_bytes)}`
      : job.total_entries > 0
        ? `${job.completed_entries} / ${job.total_entries} items`
        : "Preparing";
  const destination = job.dest_path || "/";
  const isPaused = job.status === "paused_needs_confirmation";

  return (
    <div
      role="menuitem"
      style={{
        display: "grid",
        gridTemplateColumns: "1fr auto",
        gap: "var(--space-2)",
        padding: "var(--space-2)",
        borderRadius: "var(--radius-md)",
      }}
    >
      <div style={{ minWidth: 0 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: "var(--space-2)",
          }}
        >
          <div
            style={{
              minWidth: 0,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              fontSize: "var(--text-sm)",
              fontWeight: 600,
              color: "var(--color-fg)",
            }}
          >
            {title}
          </div>
          <span
            className="tabular-nums"
            style={{
              fontSize: "var(--text-xs)",
              color: "var(--color-fg-muted)",
            }}
          >
            {percent}%
          </span>
        </div>
        <div
          style={{
            height: 4,
            borderRadius: 2,
            background: "var(--color-border)",
            overflow: "hidden",
            margin: "var(--space-1) 0",
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
        <div
          style={{
            display: "grid",
            gap: 2,
            color: "var(--color-fg-subtle)",
            fontSize: "var(--text-xs)",
            minWidth: 0,
          }}
        >
          <span
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {detail}
          </span>
          <span
            style={{
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {job.operation === "delete"
              ? `from ${job.source_root}`
              : `to ${job.dest_root}:${destination}`}
          </span>
          {isPaused && (
            <span
              style={{ color: "var(--color-warning, var(--color-accent))" }}
            >
              Needs confirmation after restart
            </span>
          )}
          {job.error && (
            <span
              style={{
                color: "var(--color-danger)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {job.error}
            </span>
          )}
        </div>
      </div>
      <div style={{ display: "flex", gap: 4, alignItems: "start" }}>
        {isPaused && (
          <>
            <button
              type="button"
              onClick={onResume}
              disabled={resuming}
              title="Resume operation"
              aria-label="Resume operation"
              style={jobActionButtonStyle(resuming)}
            >
              <Icon name="checkCircle" size={14} />
            </button>
            <button
              type="button"
              onClick={onCleanup}
              disabled={cleaning}
              title="Clean up operation"
              aria-label="Clean up operation"
              style={jobActionButtonStyle(cleaning)}
            >
              <Icon name="x" size={14} />
            </button>
          </>
        )}
        {!isPaused && (
          <button
            type="button"
            onClick={onCancel}
            disabled={cancelling}
            title="Cancel operation"
            aria-label="Cancel operation"
            style={jobActionButtonStyle(cancelling)}
          >
            <Icon name="x" size={14} />
          </button>
        )}
      </div>
    </div>
  );
}

function jobActionButtonStyle(disabled: boolean): CSSProperties {
  return {
    width: 30,
    height: 30,
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    border: "1px solid var(--color-border)",
    borderRadius: "var(--radius-md)",
    background: "transparent",
    color: "var(--color-fg-muted)",
    cursor: disabled ? "progress" : "pointer",
    opacity: disabled ? 0.6 : 1,
  };
}

const iconButtonStyle: CSSProperties = {
  width: 22,
  height: 22,
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  border: "none",
  borderRadius: "var(--radius-sm)",
  background: "transparent",
  color: "var(--color-fg-muted)",
  cursor: "pointer",
  padding: 0,
};
