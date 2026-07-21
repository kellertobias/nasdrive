const API_BASE = "";
const DEFAULT_API_REQUEST_TIMEOUT_MS = 30_000;

interface FetchOptions extends RequestInit {
  skipCsrf?: boolean;
}

export interface DownloadProgress {
  loaded: number;
  total: number | null;
  lengthComputable: boolean;
  pct: number | null;
}

export type ShareType = "typical" | "gallery" | "dropbox" | "collaboration";

export interface GalleryJob {
  id: string;
  share_id: string;
  status: string;
  total_items: number;
  processed_items: number;
  error: string | null;
  created_at: number;
  updated_at: number;
  finished_at: number | null;
}

export interface GalleryItem {
  id: string;
  relative_path: string;
  filename: string;
  sequence: number;
  source_mtime_ms: number;
  source_size: number;
  width: number | null;
  height: number | null;
  captured_at: number | null;
  mime_type: string | null;
  thumbnail_ready: boolean;
  preview_ready: boolean;
  error: string | null;
  marked: boolean;
  note: string | null;
}

interface AbortableDownload {
  promise: Promise<void>;
  abort: () => void;
}

class ApiError extends Error {
  status: number;
  statusText: string;
  body: unknown;

  constructor(status: number, statusText: string, body: unknown) {
    super(apiErrorMessage(status, statusText, body));
    this.name = "ApiError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

export class UploadAbortedError extends Error {
  constructor() {
    super("Upload cancelled");
    this.name = "UploadAbortedError";
  }
}

export class DownloadAbortedError extends Error {
  constructor() {
    super("Download cancelled");
    this.name = "DownloadAbortedError";
  }
}

function apiErrorMessage(
  status: number,
  statusText: string,
  body: unknown,
): string {
  if (body && typeof body === "object" && "error" in body) {
    const error = (body as { error?: unknown }).error;
    if (typeof error === "string" && error.trim()) return error;
  }

  const fallback = [status, statusText].filter(Boolean).join(" ");
  return fallback || "Request failed";
}

function formatApiError(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

function formatApiErrorDetails(err: unknown): string {
  if (err instanceof ApiError) {
    const lines = [
      err.message,
      err.status
        ? `Status: ${err.status}${err.statusText ? ` ${err.statusText}` : ""}`
        : "",
    ].filter(Boolean);

    if (err.body !== null && err.body !== undefined) {
      lines.push(
        "Response body:",
        typeof err.body === "string"
          ? err.body
          : JSON.stringify(err.body, null, 2),
      );
    }

    return lines.join("\n");
  }

  if (err instanceof Error) {
    return err.stack || err.message;
  }

  return String(err);
}

function notifyDownloadProgress(
  e: ProgressEvent,
  onProgress?: (progress: DownloadProgress) => void,
) {
  if (!onProgress) return;
  const total = e.lengthComputable ? e.total : null;
  onProgress({
    loaded: e.loaded,
    total,
    lengthComputable: e.lengthComputable,
    pct: total ? Math.round((e.loaded / total) * 100) : null,
  });
}

function notifyDownloadComplete(
  blob: Blob | null,
  onProgress?: (progress: DownloadProgress) => void,
) {
  if (!onProgress) return;
  const size = blob?.size ?? 0;
  onProgress({
    loaded: size,
    total: size || null,
    lengthComputable: size > 0,
    pct: 100,
  });
}

// @tour authentication:30 The single HTTP chokepoint
// Every request in the app funnels through here, and three details matter for auth. It sets
// the [`X-NasFiles-Request` header](glossary:csrf-header) on every state-changing method
// unless `skipCsrf` is passed; it sends `credentials: "same-origin"` so the `nasfiles.sid`
// cookie rides along; and it converts a `401` into a bare `ApiError` without reading the
// body.
//
// That header is not decoration — the server rejects state-changing requests that lack it.

async function apiFetch<T>(
  path: string,
  options: FetchOptions = {},
): Promise<T> {
  const { skipCsrf, ...fetchOptions } = options;
  const controller = new AbortController();
  const upstreamSignal = fetchOptions.signal;
  let timedOut = false;
  const timeoutId = globalThis.setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, DEFAULT_API_REQUEST_TIMEOUT_MS);
  const abortFromUpstream = () => controller.abort(upstreamSignal?.reason);

  if (upstreamSignal?.aborted) {
    controller.abort(upstreamSignal.reason);
  } else {
    upstreamSignal?.addEventListener("abort", abortFromUpstream, {
      once: true,
    });
  }

  const headers = new Headers(fetchOptions.headers);

  // Add CSRF header for state-changing methods
  const method = (fetchOptions.method || "GET").toUpperCase();
  if (!skipCsrf && ["POST", "PUT", "DELETE", "PATCH"].includes(method)) {
    headers.set("X-NasFiles-Request", "1");
  }

  if (
    !headers.has("Content-Type") &&
    fetchOptions.body &&
    typeof fetchOptions.body === "string"
  ) {
    headers.set("Content-Type", "application/json");
  }

  let response: Response;
  try {
    response = await fetch(`${API_BASE}${path}`, {
      ...fetchOptions,
      headers,
      credentials: "same-origin",
      signal: controller.signal,
    });
  } catch (err) {
    if (timedOut) {
      throw new ApiError(0, "Request timed out", null);
    }
    throw err;
  } finally {
    globalThis.clearTimeout(timeoutId);
    upstreamSignal?.removeEventListener("abort", abortFromUpstream);
  }

  if (response.status === 401) {
    throw new ApiError(401, "Unauthorized", null);
  }

  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(response.status, response.statusText, body);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return response.json();
}

// ---- API types ----

export interface CustomLink {
  icon: string;
  icon_color: string;
  name: string;
  url: string;
}

export interface UserInfo {
  user_id: string;
  username: string;
  display_name: string;
  picture_url: string | null;
  is_admin: boolean;
  roots: Root[];
  custom_links: CustomLink[];
  auth: AuthInfo;
  capabilities: ServerCapabilities;
  build: BuildInfo;
}

export interface AuthInfo {
  mode: "sso" | "local";
  passkeys_enabled: boolean;
  totp_enabled: boolean;
}

export interface AuthConfig {
  mode: "sso" | "local";
  local_enabled: boolean;
  sso_enabled: boolean;
  passkeys_enabled: boolean;
  totp_enabled: boolean;
  /** True when dev mode is bypassing real authentication (synthetic dev user). */
  dev_auth_bypass: boolean;
}

export interface ServerCapabilities {
  archive_extraction: boolean;
  thumbnails: boolean;
  media_preview_transcoding: boolean;
  media_metadata_probe: boolean;
  sftp_enabled: boolean;
  sftp_hostname: string | null;
  sftp_port: number;
}

export interface BuildInfo {
  commit: string;
  date: string;
  started_at: string;
}

export interface SftpKey {
  id: string;
  key_fingerprint: string;
  label: string | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
}

export interface SftpTempUser {
  id: string;
  created_by_user_id: string;
  display_name: string;
  root_kind: string;
  root_key: string;
  relative_path: string;
  can_write: boolean;
  expires_at: number;
  revoked_at: number | null;
  created_at: number;
  restored_from_id: string | null;
  key_fingerprint: string | null;
  public_key: string | null;
  last_used_at: number | null;
}

export interface SftpAccessLogEntry {
  id: string;
  principal_kind: string;
  principal_id: string;
  occurred_at: number;
  action: string;
  root_key: string | null;
  path: string | null;
  ip: string | null;
  success: boolean;
  error: string | null;
}

export interface ActiveSftpSession {
  session_id: string;
  principal_kind: string;
  principal_id: string;
  display_name: string;
  remote_ip: string | null;
  connected_at: number;
  bytes_read: number;
  bytes_written: number;
}

export interface IpBlocklistEntry {
  ip: string;
  reason: string | null;
  blocked_at: number;
  last_seen_at: number;
  hit_count: number;
}

export interface FolderCaps {
  read: boolean;
  write: boolean;
  share: boolean;
}

export interface Root {
  key: string;
  display_name: string;
  kind: "common" | "home";
  caps: FolderCaps;
  /** Optional sidebar group. Roots sharing a group name render nested under a
   *  collapsible group header. Groups are display-only and not navigable. */
  group?: string | null;
  usage?: RootUsage | null;
}

export interface RootUsage {
  used_bytes: number;
  total_bytes: number;
  available_bytes: number;
}

export interface MediaInfo {
  duration_ms: number | null;
  width: number | null;
  height: number | null;
  video_codec: string | null;
  audio_codec: string | null;
  bitrate_bps: number | null;
  format_name: string | null;
  video_mime_codec: string | null;
  audio_mime_codec: string | null;
  audio_languages: string[];
}

export interface ImageInfo {
  width: number;
  height: number;
  format: string | null;
  has_alpha: boolean;
  exif?: Record<string, string>;
}

export interface FileEntry {
  name: string;
  size: number;
  modified_at: number;
  is_dir: boolean;
  mime_type: string | null;
  has_thumbnail: boolean;
  media_info?: MediaInfo | null;
  image_info?: ImageInfo | null;
  item_count?: number;
  gallery_feedback?: {
    marked: boolean;
    note?: string | null;
  } | null;
}

export interface DirectoryListing {
  path: string;
  entries: FileEntry[];
}

export interface TreeListing {
  path: string;
  children: FileEntry[];
}

export interface SearchResult {
  root: string;
  root_display_name: string;
  path: string;
  parent_path: string;
  entry: FileEntry;
  source: "index" | "live";
  score: number;
}

export interface SearchResponse {
  query: string;
  results: SearchResult[];
  live_complete: boolean;
  index_ready: boolean;
}

export interface TransferJob {
  id: string;
  operation: "move" | "copy" | "delete";
  source_root: string;
  dest_root: string;
  dest_path: string;
  paths: string[];
  status:
    | "queued"
    | "running"
    | "paused_needs_confirmation"
    | "done"
    | "error"
    | "canceled";
  total_bytes: number;
  transferred_bytes: number;
  total_entries: number;
  completed_entries: number;
  error: string | null;
  created_at: number;
  updated_at: number;
  finished_at: number | null;
}

export interface PreviewStatus {
  session: string;
  state: "queued" | "starting" | "streaming" | "completed" | "failed" | string;
  profile: string;
  mode: string;
  bytes_sent: number;
  elapsed_ms: number;
  exit_status: string | null;
  stderr_tail: string | null;
  error: string | null;
  created_at: number;
  updated_at: number;
}

export type ExtractMode = "here" | "here_remove" | "subfolder";

export interface TrustedDeviceProof {
  id: string;
  hash: string;
  code: string;
}

export interface TrustedDevice {
  id: string;
  label: string | null;
  created_at: number;
  last_used_at: number | null;
  expires_at: number | null;
  revoked_at: number | null;
}

export interface PasskeyInfo {
  id: string;
  label: string | null;
  created_at: number;
  last_used_at: number | null;
  revoked_at: number | null;
}

export interface ApiToken {
  id: string;
  label: string;
  access_key: string;
  created_at: number;
  expires_at: number | null;
  last_used_at: number | null;
}

export interface CreatedApiToken extends ApiToken {
  secret_key: string;
}

export interface AdminUserDetails {
  id: string;
  username: string;
  display_name: string;
  picture_url: string | null;
  is_admin: boolean;
  has_home: boolean;
  auth_provider: string;
  folder_permissions: Record<string, FolderCaps>;
  passkey_count: number;
  totp_enabled: boolean;
  trusted_device_count: number;
  created_at: number;
  last_login_at: number;
}

export interface LocalLoginResponse {
  ok: boolean;
  requires_totp: boolean;
  challenge_id?: string;
}

export interface TotpLoginResponse {
  ok: boolean;
  trusted_device?: {
    id: string;
    secret: string;
    hash: string;
    label: string | null;
    expires_at: number | null;
  } | null;
}

// ---- API functions ----

export const api = {
  authConfig: () => apiFetch<AuthConfig>("/api/auth/config"),

  me: async () => {
    try {
      return await apiFetch<UserInfo>("/api/me");
    } catch (err: unknown) {
      if (err instanceof ApiError && err.status === 401) return null;
      throw err;
    }
  },

  roots: () => apiFetch<{ roots: Root[] }>("/api/roots"),

  listDirectory: (root: string, path: string = "") =>
    apiFetch<DirectoryListing>(
      `/api/files/${encodeURIComponent(root)}/list?path=${encodeURIComponent(path)}`,
    ),

  listTree: (root: string, path: string = "") =>
    apiFetch<TreeListing>(
      `/api/files/${encodeURIComponent(root)}/tree?path=${encodeURIComponent(path)}`,
    ),

  search: (q: string, limit: number = 100) => {
    const params = new URLSearchParams({ q, limit: String(limit) });
    return apiFetch<SearchResponse>(`/api/search?${params.toString()}`);
  },

  fileInfo: (root: string, path: string) =>
    apiFetch<FileEntry & { path: string }>(
      `/api/files/${encodeURIComponent(root)}/info?path=${encodeURIComponent(path)}`,
    ),

  folderSizes: (root: string, paths: string[]) =>
    apiFetch<{ sizes: Record<string, number> }>(
      `/api/files/${encodeURIComponent(root)}/folder-sizes`,
      {
        method: "POST",
        body: JSON.stringify({ paths }),
      },
    ),

  downloadUrl: (root: string, path: string) =>
    `/api/files/${encodeURIComponent(root)}/download?path=${encodeURIComponent(path)}`,

  previewUrl: (root: string, path: string, session?: string) => {
    const params = new URLSearchParams({ path });
    if (session) params.set("session", session);
    return `/api/files/${encodeURIComponent(root)}/preview?${params.toString()}`;
  },

  previewStatus: (root: string, path: string, session: string) => {
    const params = new URLSearchParams({ path, session });
    return apiFetch<PreviewStatus>(
      `/api/files/${encodeURIComponent(root)}/preview-status?${params.toString()}`,
    );
  },

  thumbnailUrl: (
    root: string,
    path: string,
    width: number = 480,
    entry?: Pick<FileEntry, "modified_at" | "size">,
    retry: number = 0,
    format?: "jpeg" | "png",
  ) => {
    const params = new URLSearchParams({
      path,
      w: String(width),
    });
    if (format) params.set("format", format);
    if (entry) params.set("v", `${entry.modified_at}-${entry.size}`);
    if (retry > 0) params.set("retry", String(retry));
    return `/api/files/${encodeURIComponent(root)}/thumbnail?${params.toString()}`;
  },

  logout: () => apiFetch<void>("/auth/logout", { method: "POST" }),

  // @tour authentication:40 The login call itself
  // A thin wrapper POSTing `{username, password, trusted_device}` to `/auth/local/login`.
  // Its siblings `localLoginTotp`, `startPasskeyLogin` and `finishPasskeyLogin` cover the
  // other legs of the flow.
  //
  // Note that `me` further up deliberately swallows a 401 and returns `null`, so an
  // unauthenticated visitor renders the login page instead of an error screen.

  localLogin: (body: {
    username: string;
    password: string;
    trusted_device?: TrustedDeviceProof | null;
  }) =>
    apiFetch<LocalLoginResponse>("/auth/local/login", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  localLoginTotp: (body: {
    challenge_id: string;
    code: string;
    trust_computer: boolean;
    device_label?: string;
  }) =>
    apiFetch<TotpLoginResponse>("/auth/local/login/totp", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  startPasskeyLogin: (username: string) =>
    apiFetch<unknown>("/auth/local/passkey/options", {
      method: "POST",
      body: JSON.stringify({ username }),
    }),

  finishPasskeyLogin: (credential: unknown) =>
    apiFetch<{ ok: boolean }>("/auth/local/passkey/finish", {
      method: "POST",
      body: JSON.stringify(credential),
    }),

  changePassword: (current_password: string, new_password: string) =>
    apiFetch<{ ok: boolean }>("/api/profile/password", {
      method: "POST",
      body: JSON.stringify({ current_password, new_password }),
    }),

  startTotpSetup: () =>
    apiFetch<{ secret: string; url: string }>("/api/profile/totp/setup", {
      method: "POST",
    }),

  confirmTotpSetup: (code: string) =>
    apiFetch<{ ok: boolean }>("/api/profile/totp/confirm", {
      method: "POST",
      body: JSON.stringify({ code }),
    }),

  removeTotp: () =>
    apiFetch<{ ok: boolean }>("/api/profile/totp", { method: "DELETE" }),

  listTrustedDevices: () =>
    apiFetch<{ devices: TrustedDevice[] }>("/api/profile/trusted-devices"),

  revokeTrustedDevice: (id: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/profile/trusted-devices/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    ),

  listPasskeys: () =>
    apiFetch<{ passkeys: PasskeyInfo[] }>("/api/profile/passkeys"),

  startPasskeyRegistration: () =>
    apiFetch<unknown>("/api/profile/passkeys/options", { method: "POST" }),

  finishPasskeyRegistration: (credential: unknown) =>
    apiFetch<{ ok: boolean }>("/api/profile/passkeys/finish", {
      method: "POST",
      body: JSON.stringify(credential),
    }),

  revokePasskey: (id: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/profile/passkeys/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    ),

  // ---- API tokens ----

  listApiTokens: () =>
    apiFetch<{ tokens: ApiToken[] }>("/api/profile/api-tokens"),

  createApiToken: (label: string, expires_in: number | null) =>
    apiFetch<CreatedApiToken>("/api/profile/api-tokens", {
      method: "POST",
      body: JSON.stringify({ label, expires_in }),
    }),

  renewApiToken: (id: string, extend_by: number) =>
    apiFetch<{ ok: boolean; expires_at: number }>(
      `/api/profile/api-tokens/${encodeURIComponent(id)}`,
      {
        method: "PATCH",
        body: JSON.stringify({ extend_by }),
      },
    ),

  revokeApiToken: (id: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/profile/api-tokens/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    ),

  // ---- Write operations ----

  mkdir: (root: string, path: string, name: string) =>
    apiFetch<{ ok: boolean }>(`/api/files/${encodeURIComponent(root)}/mkdir`, {
      method: "POST",
      body: JSON.stringify({ path, name }),
    }),

  rename: (root: string, path: string, newName: string) =>
    apiFetch<{ ok: boolean }>(`/api/files/${encodeURIComponent(root)}/rename`, {
      method: "POST",
      body: JSON.stringify({ path, new_name: newName }),
    }),

  moveEntries: (root: string, paths: string[], dest: string) =>
    apiFetch<{ ok: boolean }>(`/api/files/${encodeURIComponent(root)}/move`, {
      method: "POST",
      body: JSON.stringify({ paths, dest }),
    }),

  // @tour file-transfers:40 The API client method
  // POSTs to `/api/files/{root}/transfer` with `{paths, dest_root, dest, operation}`. The
  // source root travels in the URL while the destination root travels in the body — that
  // asymmetry is what makes cross-root transfers expressible on a per-root route.
  //
  // The declared `job_id` in the response is not used by the caller: progress is discovered
  // by listing all jobs rather than tracking this one. Contrast `moveEntries` just above,
  // which is the older synchronous endpoint.

  transferEntries: (
    root: string,
    paths: string[],
    destRoot: string,
    dest: string,
    operation: "move" | "copy",
  ) =>
    apiFetch<{ ok: boolean; job_id: string }>(
      `/api/files/${encodeURIComponent(root)}/transfer`,
      {
        method: "POST",
        body: JSON.stringify({ paths, dest_root: destRoot, dest, operation }),
      },
    ),

  transferJobs: () => apiFetch<{ jobs: TransferJob[] }>("/api/transfer-jobs"),

  cancelTransferJob: (jobId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/transfer-jobs/${encodeURIComponent(jobId)}/cancel`,
      {
        method: "POST",
      },
    ),

  resumeFileJob: (jobId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/file-jobs/${encodeURIComponent(jobId)}/resume`,
      {
        method: "POST",
      },
    ),

  cancelFileJob: (jobId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/file-jobs/${encodeURIComponent(jobId)}/cancel`,
      {
        method: "POST",
      },
    ),

  cleanupFileJob: (jobId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/file-jobs/${encodeURIComponent(jobId)}/cleanup`,
      {
        method: "POST",
      },
    ),

  deleteEntries: (root: string, paths: string[]) =>
    apiFetch<{ ok: boolean; job_id: string }>(
      `/api/files/${encodeURIComponent(root)}/delete`,
      {
        method: "POST",
        body: JSON.stringify({ paths }),
      },
    ),

  extractArchive: (root: string, path: string, mode: ExtractMode) =>
    apiFetch<{ ok: boolean }>(
      `/api/files/${encodeURIComponent(root)}/extract`,
      {
        method: "POST",
        body: JSON.stringify({ path, mode }),
      },
    ),

  downloadZip: (
    root: string,
    paths: string[],
    onProgress?: (progress: DownloadProgress) => void,
  ) => {
    // Use XHR to get binary blob, then trigger download
    return new Promise<void>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open("POST", `/api/files/${encodeURIComponent(root)}/zip`);
      xhr.setRequestHeader("Content-Type", "application/json");
      xhr.setRequestHeader("X-NasFiles-Request", "1");
      xhr.responseType = "blob";
      xhr.addEventListener("progress", (e) =>
        notifyDownloadProgress(e, onProgress),
      );

      xhr.addEventListener("load", () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          notifyDownloadComplete(xhr.response, onProgress);
          const disposition =
            xhr.getResponseHeader("Content-Disposition") || "";
          const match = disposition.match(/filename="?([^"]+)"?/);
          const filename = match?.[1] || "download.zip";

          const url = URL.createObjectURL(xhr.response);
          const a = document.createElement("a");
          a.href = url;
          a.download = filename;
          document.body.appendChild(a);
          a.click();
          document.body.removeChild(a);
          URL.revokeObjectURL(url);
          resolve();
        } else {
          readXhrErrorBody(xhr).then((body) => {
            reject(new ApiError(xhr.status, xhr.statusText, body));
          });
        }
      });

      xhr.addEventListener("error", () =>
        reject(new ApiError(0, "Network error", null)),
      );
      xhr.send(JSON.stringify({ paths }));
    });
  },

  upload: (
    root: string,
    path: string,
    files: File[],
    onProgress?: (pct: number) => void,
  ): {
    promise: Promise<{ ok: boolean; files_uploaded: number }>;
    abort: () => void;
  } => {
    const xhr = new XMLHttpRequest();
    const promise = new Promise<{ ok: boolean; files_uploaded: number }>(
      (resolve, reject) => {
        const formData = new FormData();
        for (const file of files) {
          formData.append("file", file, file.name);
        }

        xhr.open(
          "POST",
          `/api/files/${encodeURIComponent(root)}/upload?path=${encodeURIComponent(path)}`,
        );
        xhr.setRequestHeader("X-NasFiles-Request", "1");

        xhr.upload.addEventListener("progress", (e) => {
          if (e.lengthComputable && onProgress) {
            onProgress(Math.round((e.loaded / e.total) * 100));
          }
        });

        xhr.addEventListener("load", () => {
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(
              (parseJsonResponse(xhr.responseText) as {
                ok: boolean;
                files_uploaded: number;
              }) ?? { ok: true, files_uploaded: 0 },
            );
          } else {
            reject(
              new ApiError(
                xhr.status,
                xhr.statusText,
                parseJsonResponse(xhr.responseText),
              ),
            );
          }
        });

        xhr.addEventListener("error", () => {
          reject(new ApiError(0, "Network error", null));
        });

        xhr.addEventListener("abort", () => {
          reject(new UploadAbortedError());
        });

        xhr.send(formData);
      },
    );
    return { promise, abort: () => xhr.abort() };
  },

  // ---- Share management ----

  // @tour share-management:30 The typed API surface
  // `createShare` spreads `opts` into a body alongside `root_key` and `path` and POSTs to
  // `/api/shares`, declaring the response as `{ id, token, url, created_at, expires_at }`.
  // `listShares`, `getShare` and `revokeShare` complete the owner-side CRUD.
  //
  // Note that the client sends `allow_upload`/`allow_download` here, and the server ignores
  // them entirely — the callout in `shares/create.rs` explains why.

  createShare: (
    root: string,
    path: string,
    opts: {
      target_kind: "public" | "guest";
      password?: string;
      allow_upload: boolean;
      allow_download: boolean;
      share_type: ShareType;
      expires_in: number | null;
    },
  ) =>
    apiFetch<{
      id: string;
      token: string;
      url: string;
      created_at: number;
      expires_at: number | null;
    }>(`/api/shares`, {
      method: "POST",
      body: JSON.stringify({
        root_key: root,
        path,
        ...opts,
      }),
    }),

  listShares: () =>
    apiFetch<{
      shares: Array<{
        id: string;
        root_key: string;
        relative_path: string;
        is_directory: boolean;
        target_kind: string;
        allow_upload: boolean;
        allow_download: boolean;
        expires_at: number | null;
        created_at: number;
        revoked_at: number | null;
        share_type: ShareType;
        access_count: number;
        last_accessed_at: number | null;
      }>;
    }>("/api/shares"),

  getShare: (id: string) =>
    apiFetch<Record<string, unknown>>(`/api/shares/${encodeURIComponent(id)}`),

  revokeShare: (id: string) =>
    apiFetch<{ ok: boolean }>(`/api/shares/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),

  // ---- SFTP ----

  listSftpKeys: () => apiFetch<{ keys: SftpKey[] }>("/api/sftp/keys"),

  addSftpKey: (public_key: string, label?: string) =>
    apiFetch<SftpKey>("/api/sftp/keys", {
      method: "POST",
      body: JSON.stringify({ public_key, label }),
    }),

  revokeSftpKey: (id: string) =>
    apiFetch<{ ok: boolean }>(`/api/sftp/keys/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),

  listSftpTempUsers: () =>
    apiFetch<{ users: SftpTempUser[] }>("/api/admin/sftp-temp-users"),

  listSftpAccessLog: () =>
    apiFetch<{
      entries: SftpAccessLogEntry[];
      total: number;
      limit: number;
      offset: number;
    }>("/api/admin/sftp-access-log?limit=200"),

  createSftpTempUser: (body: {
    display_name: string;
    root_key: string;
    path: string;
    can_write: boolean;
    expires_in: number;
    public_key: string;
  }) =>
    apiFetch<SftpTempUser & { login: string }>("/api/admin/sftp-temp-users", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  extendSftpTempUser: (id: string, expires_in: number) =>
    apiFetch<{ ok: boolean; expires_at: number }>(
      `/api/admin/sftp-temp-users/${encodeURIComponent(id)}/extend`,
      {
        method: "POST",
        body: JSON.stringify({ expires_in }),
      },
    ),

  revokeSftpTempUser: (id: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/admin/sftp-temp-users/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    ),

  listActiveSftpSessions: () =>
    apiFetch<{ sessions: ActiveSftpSession[] }>("/api/admin/sftp-sessions"),

  listIpBlocklist: () =>
    apiFetch<{ entries: IpBlocklistEntry[] }>("/api/admin/ip-blocklist"),

  addIpBlocklistEntry: (ip: string, reason?: string) =>
    apiFetch<{ ok: boolean; ip: string }>("/api/admin/ip-blocklist", {
      method: "POST",
      body: JSON.stringify({ ip, reason }),
    }),

  removeIpBlocklistEntry: (ip: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/admin/ip-blocklist/${encodeURIComponent(ip)}`,
      { method: "DELETE" },
    ),

  // ---- Local user admin ----

  listAdminUsers: () =>
    apiFetch<{ users: AdminUserDetails[] }>("/api/admin/users"),

  createLocalUser: (body: {
    username: string;
    display_name?: string;
    is_admin: boolean;
    has_home: boolean;
    folder_permissions: Record<string, FolderCaps>;
  }) =>
    apiFetch<{
      id: string;
      username: string;
      display_name: string;
      password: string;
    }>("/api/admin/users", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  updateLocalUser: (
    id: string,
    body: {
      display_name?: string;
      is_admin?: boolean;
      has_home?: boolean;
      folder_permissions?: Record<string, FolderCaps>;
    },
  ) =>
    apiFetch<{ ok: boolean }>(`/api/admin/users/${encodeURIComponent(id)}`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),

  resetLocalUserPassword: (id: string) =>
    apiFetch<{ ok: boolean; password: string }>(
      `/api/admin/users/${encodeURIComponent(id)}/reset-password`,
      {
        method: "POST",
      },
    ),

  listAdminPasskeys: (id: string) =>
    apiFetch<{ passkeys: PasskeyInfo[] }>(
      `/api/admin/users/${encodeURIComponent(id)}/passkeys`,
    ),

  revokeAdminPasskey: (userId: string, passkeyId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/admin/users/${encodeURIComponent(userId)}/passkeys/${encodeURIComponent(passkeyId)}`,
      { method: "DELETE" },
    ),

  listAdminTrustedDevices: (id: string) =>
    apiFetch<{ devices: TrustedDevice[] }>(
      `/api/admin/users/${encodeURIComponent(id)}/trusted-devices`,
    ),

  revokeAdminTrustedDevice: (userId: string, deviceId: string) =>
    apiFetch<{ ok: boolean }>(
      `/api/admin/users/${encodeURIComponent(userId)}/trusted-devices/${encodeURIComponent(deviceId)}`,
      { method: "DELETE" },
    ),

  // ---- Public share access ----

  shareMetadata: (token: string) =>
    apiFetch<{
      name: string;
      is_directory: boolean;
      requires_password: boolean;
      owner_display_name: string;
      allow_upload: boolean;
      allow_download: boolean;
      share_type: ShareType;
      expires_at: number | null;
    }>(`/api/public/shares/${encodeURIComponent(token)}`),

  galleryJobs: () => apiFetch<{ jobs: GalleryJob[] }>("/api/gallery-jobs"),

  shareGalleryFeedback: (shareId: string) =>
    apiFetch<{ items: GalleryItem[] }>(
      `/api/shares/${encodeURIComponent(shareId)}/gallery/feedback`,
    ),

  shareAuth: (token: string, password?: string) =>
    apiFetch<{ bearer: string; expires_in: number }>(
      `/api/public/shares/${encodeURIComponent(token)}/auth`,
      {
        method: "POST",
        body: JSON.stringify({ password: password || null }),
      },
    ),

  shareList: (token: string, bearer: string, path: string = "") =>
    apiFetch<DirectoryListing>(
      `/api/public/shares/${encodeURIComponent(token)}/list?path=${encodeURIComponent(path)}`,
      {
        headers: { Authorization: `Bearer ${bearer}` },
      },
    ),

  publicGallery: (token: string, bearer: string) =>
    apiFetch<{ items: GalleryItem[] }>(
      `/api/public/shares/${encodeURIComponent(token)}/gallery`,
      {
        headers: { Authorization: `Bearer ${bearer}` },
      },
    ),

  publicGalleryAssetUrl: (
    token: string,
    bearer: string,
    itemId: string,
    asset: "thumbnail" | "preview",
  ) =>
    `/api/public/shares/${encodeURIComponent(token)}/gallery/${encodeURIComponent(itemId)}/asset/${asset}?t=${encodeURIComponent(bearer)}`,

  updatePublicGalleryFeedback: (
    token: string,
    bearer: string,
    itemId: string,
    body: { marked: boolean; note: string | null },
  ) =>
    apiFetch<{ ok: boolean }>(
      `/api/public/shares/${encodeURIComponent(token)}/gallery/${encodeURIComponent(itemId)}/feedback`,
      {
        method: "PUT",
        headers: { Authorization: `Bearer ${bearer}` },
        body: JSON.stringify(body),
      },
    ),

  shareDownloadUrl: (token: string, bearer: string, path: string) =>
    `/api/public/shares/${encodeURIComponent(token)}/download?path=${encodeURIComponent(path)}&t=${encodeURIComponent(bearer)}`,

  // Forces a real download even for browser-viewable types (images, PDFs, etc.),
  // which the server serves with `Content-Disposition: inline`.
  shareDownloadFile: (
    token: string,
    bearer: string,
    path: string,
    onProgress?: (progress: DownloadProgress) => void,
  ): AbortableDownload => {
    const xhr = new XMLHttpRequest();
    const promise = new Promise<void>((resolve, reject) => {
      xhr.open(
        "GET",
        `/api/public/shares/${encodeURIComponent(token)}/download?path=${encodeURIComponent(path)}`,
      );
      xhr.setRequestHeader("Authorization", `Bearer ${bearer}`);
      xhr.responseType = "blob";
      xhr.addEventListener("progress", (e) =>
        notifyDownloadProgress(e, onProgress),
      );

      xhr.addEventListener("load", () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          notifyDownloadComplete(xhr.response, onProgress);
          const disposition =
            xhr.getResponseHeader("Content-Disposition") || "";
          const match = disposition.match(/filename="?([^"]+)"?/);
          const filename = match?.[1] || path.split("/").pop() || "download";

          const url = URL.createObjectURL(xhr.response);
          const a = document.createElement("a");
          a.href = url;
          a.download = filename;
          document.body.appendChild(a);
          a.click();
          document.body.removeChild(a);
          URL.revokeObjectURL(url);
          resolve();
        } else {
          readXhrErrorBody(xhr).then((body) => {
            reject(new ApiError(xhr.status, xhr.statusText, body));
          });
        }
      });

      xhr.addEventListener("error", () =>
        reject(new ApiError(0, "Network error", null)),
      );
      xhr.addEventListener("abort", () => reject(new DownloadAbortedError()));
      xhr.send();
    });
    return { promise, abort: () => xhr.abort() };
  },

  shareInfo: (token: string, bearer: string, path: string) =>
    apiFetch<FileEntry & { path: string }>(
      `/api/public/shares/${encodeURIComponent(token)}/info?path=${encodeURIComponent(path)}`,
      {
        headers: { Authorization: `Bearer ${bearer}` },
      },
    ),

  sharePreviewUrl: (
    token: string,
    bearer: string,
    path: string,
    session?: string,
  ) => {
    const params = new URLSearchParams({ path, t: bearer });
    if (session) params.set("session", session);
    return `/api/public/shares/${encodeURIComponent(token)}/preview?${params.toString()}`;
  },

  sharePreviewStatus: (
    token: string,
    bearer: string,
    path: string,
    session: string,
  ) => {
    const params = new URLSearchParams({ path, session });
    return apiFetch<PreviewStatus>(
      `/api/public/shares/${encodeURIComponent(token)}/preview-status?${params.toString()}`,
      {
        headers: { Authorization: `Bearer ${bearer}` },
      },
    );
  },

  shareDownloadZip: (
    token: string,
    bearer: string,
    paths: string[],
    onProgress?: (progress: DownloadProgress) => void,
  ): AbortableDownload => {
    const xhr = new XMLHttpRequest();
    const promise = new Promise<void>((resolve, reject) => {
      xhr.open("POST", `/api/public/shares/${encodeURIComponent(token)}/zip`);
      xhr.setRequestHeader("Content-Type", "application/json");
      xhr.setRequestHeader("Authorization", `Bearer ${bearer}`);
      xhr.setRequestHeader("X-NasFiles-Request", "1");
      xhr.responseType = "blob";
      xhr.addEventListener("progress", (e) =>
        notifyDownloadProgress(e, onProgress),
      );

      xhr.addEventListener("load", () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          notifyDownloadComplete(xhr.response, onProgress);
          const disposition =
            xhr.getResponseHeader("Content-Disposition") || "";
          const match = disposition.match(/filename="?([^"]+)"?/);
          const filename = match?.[1] || "download.zip";

          const url = URL.createObjectURL(xhr.response);
          const a = document.createElement("a");
          a.href = url;
          a.download = filename;
          document.body.appendChild(a);
          a.click();
          document.body.removeChild(a);
          URL.revokeObjectURL(url);
          resolve();
        } else {
          readXhrErrorBody(xhr).then((body) => {
            reject(new ApiError(xhr.status, xhr.statusText, body));
          });
        }
      });

      xhr.addEventListener("error", () =>
        reject(new ApiError(0, "Network error", null)),
      );
      xhr.addEventListener("abort", () => reject(new DownloadAbortedError()));
      xhr.send(JSON.stringify({ paths }));
    });
    return { promise, abort: () => xhr.abort() };
  },

  shareUpload: (
    token: string,
    bearer: string,
    path: string,
    files: File[],
    onProgress?: (pct: number) => void,
  ): {
    promise: Promise<{ ok: boolean; files_uploaded: number }>;
    abort: () => void;
  } => {
    const xhr = new XMLHttpRequest();
    const promise = new Promise<{ ok: boolean; files_uploaded: number }>(
      (resolve, reject) => {
        const formData = new FormData();
        for (const file of files) {
          formData.append("file", file, file.name);
        }
        xhr.open(
          "POST",
          `/api/public/shares/${encodeURIComponent(token)}/upload?path=${encodeURIComponent(path)}`,
        );
        xhr.setRequestHeader("Authorization", `Bearer ${bearer}`);
        xhr.setRequestHeader("X-NasFiles-Request", "1");

        xhr.upload.addEventListener("progress", (e) => {
          if (e.lengthComputable && onProgress) {
            onProgress(Math.round((e.loaded / e.total) * 100));
          }
        });

        xhr.addEventListener("load", () => {
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(
              (parseJsonResponse(xhr.responseText) as {
                ok: boolean;
                files_uploaded: number;
              }) ?? { ok: true, files_uploaded: files.length },
            );
          } else {
            reject(
              new ApiError(
                xhr.status,
                xhr.statusText,
                parseJsonResponse(xhr.responseText),
              ),
            );
          }
        });

        xhr.addEventListener("error", () =>
          reject(new ApiError(0, "Network error", null)),
        );

        xhr.addEventListener("abort", () => reject(new UploadAbortedError()));

        xhr.send(formData);
      },
    );
    return { promise, abort: () => xhr.abort() };
  },
};

function parseJsonResponse(text: string): unknown {
  if (!text.trim()) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

async function readXhrErrorBody(xhr: XMLHttpRequest): Promise<unknown> {
  const response = xhr.response;
  if (response instanceof Blob) {
    const text = await response.text().catch(() => "");
    return parseJsonResponse(text);
  }
  if (typeof response === "string") return parseJsonResponse(response);
  if (xhr.responseText) return parseJsonResponse(xhr.responseText);
  return null;
}

export { ApiError, formatApiError, formatApiErrorDetails };
export default api;
