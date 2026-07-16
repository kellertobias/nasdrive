# Plan: NASDrive — Web UI Cloud File Manager

## Context

A greenfield Rust + React app that exposes common folders and per-user home folders from a host filesystem through a browser UI, while those same directories stay usable by SFTP/SMB services running alongside. Unlike Nextcloud-style apps that take ownership of their data dir, NASDrive is a thin shell over the real filesystem.

Auth is SSO-only (SAML2 + OIDC) — no local user database to manage. Users are JIT-provisioned on first login, and SSO groups map to common-folder visibility via env-var config. The only passwords in the system are on guest shares. Shares can target internal users, guests-with-password, or the public, with configurable upload/download/expiry, and every access is logged.

v1 targets: grid + list views, thumbnails for images/videos/PDFs, in-browser preview, streaming ZIP download, HTTP Range requests, drag-and-drop folder upload. **WebDAV is explicitly out of v1 scope** — SFTP/SMB running alongside cover the "mount from desktop" need for now; we can add WebDAV later without rearchitecting.

Two cross-cutting priorities run through every phase of this plan — they are not optional polish:

- **UI quality is Apple-grade.** Not "clean enough." Every surface — spacing, typography, motion, empty states, hover/focus, keyboard nav, dark mode, loading shimmer, drag-over affordances — must feel considered. A file manager is used for hours; every rough edge compounds. We budget design time into every milestone, not a "polish pass" at the end.
- **Security is a first-class requirement.** This app exposes a user's entire home directory and shared folders to the public internet with SSO and cryptographic share links. A single path-traversal, an ACL bypass on the share path, a CSRF hole on upload, or a leaked share token in a log is a full breach. We design for this from day one, not after a pentest.

Dedicated sections for both below.

---

## UI Quality Bar (Apple-grade finish)

This is a product requirement, not a stylistic preference. Concrete definition of "done":

- **Design system first, components second.** Before building screens, commit a tokens file: an 8px spacing scale, a type scale (SF Pro or Inter with tabular-nums for file sizes/dates), radii, elevation, a semantic color palette with full light + dark variants, and motion tokens (durations 120/200/320ms, `ease-out` for enters, `ease-in-out` for state, spring for drag). Every component consumes tokens — no ad-hoc hex or px values in component files.
- **Typography.** Tabular numerals for sizes and timestamps. File names truncate with middle-ellipsis, never end-ellipsis (so `.jpg` vs `.png` stays visible). Line-height and letter-spacing tuned per size.
- **Motion.** Thumbnails fade in, never pop. View-mode toggle animates between grid and list. Drag-over state lifts target with shadow + scale, not just a border. Respect `prefers-reduced-motion`.
- **Interaction feel.** Selection updates at 60fps on 10k-item directories (virtualize; debounce nothing that should feel immediate). Keyboard nav works everywhere: arrow keys in grid/list, Enter to open, Space to preview, `⌘A`/`⌘D`/`⌘C`/`⌘V`/Delete, `/` focuses search, `Esc` closes previews. Focus rings are visible and beautiful.
- **Loading and empty states are designed.** Skeleton shimmer for listings, blurhash or dominant-color placeholder for thumbnails, illustrated empty states for "no shares yet" / "empty folder" / "no matches." Never a raw spinner over blank space.
- **Previews feel native.** PDF scrolls and zooms like Preview.app. Image viewer supports pinch-zoom and pan. Video has clean controls, scrubbing works (backed by Range). Quick-look style: tap `Space` to open a floating preview over the listing.
- **Share dialog is the hero screen.** This is what users send to colleagues; it sets the perception of the product. Get it right: live link preview, copy button with success state, clear expiry/permission toggles, password strength indicator on guest passwords, existing shares listed inline.
- **Dark mode is a peer, not an afterthought.** Built into the token system from day one. All screenshots and design reviews done in both modes.
- **Accessibility.** WCAG AA contrast across both themes, every action reachable by keyboard, proper ARIA on the tree and grid, screen-reader labels on icon-only buttons.
- **Responsive down to tablet.** Phone is out of v1 scope but don't make choices that preclude it.

Process: pick a designer (or dedicate design time) before coding UI. Build a Storybook / Ladle with every component in every state. Review screens in Figma or screenshots before shipping — not just "it renders." Treat the bar as "would this look out of place next to macOS Finder?"; if yes, it's not done.

---

## Security Requirements

Threat model: authenticated users may try to escape their folder scope; guest-share visitors may try to escalate; public internet may try credential stuffing, token guessing, and exploiting upload endpoints. Assume the attacker reads all our source.

Hard rules, enforced in code review:

- **Path traversal — single chokepoint.** All filesystem access — API, thumbnails, ZIP, share paths — goes through `fs/safe_path.rs`. That module canonicalizes (resolves symlinks) and asserts the canonical result `starts_with` the canonical root. No other module ever touches `std::fs::canonicalize` or joins untrusted paths directly. Enforced by a `clippy` deny + code-owner rule on that file.
- **Symlink policy.** By default, do not follow symlinks out of a root. If a symlink's target canonicalizes outside the root, reject. Configurable only via an explicit env var for power users who know what they're doing.
- **ACL on every request.** The resolver takes the authenticated principal (session user OR share token) and the requested root, and verifies the principal has access before returning a path. No handler is allowed to skip this — path resolution and ACL check are the same function call.
- **Share tokens.** 24 bytes from `OsRng`, base64url-encoded (~32 chars). Constant-time comparison against the DB (hash the token at rest with SHA-256 and compare the hashes; plaintext token never in DB or logs). Never logged — scrub from `tracing` spans and access logs (log only token prefix + hash).
- **Share passwords.** Argon2id with OWASP-recommended parameters. Constant-time verify. Rate-limited per token: exponential backoff after 5 failed attempts, `auth_fail` audit entry each time.
- **Session cookies** (logged-in SSO users only). `HttpOnly`, `Secure` (when `BASE_URL` is https — refuse to start if http and not explicitly dev), `SameSite=Lax`, signed with `SESSION_SECRET` (64 bytes, refuse to start if weak/default). Session fixation prevention: rotate session ID on login. Guest shares never set a cookie (bearer token in `sessionStorage` instead — see auth flow).
- **Guest-share bearer tokens.** Stateless HMAC (SHA-256) over `{share_id, iat, exp}` keyed by `SESSION_SECRET`. 30-min expiry. Server re-validates signature + expiry + share-not-revoked on every request. Bound to a single share — presenting a bearer for share A on share B's endpoints fails. Not stored in the DB (stateless), but revocation still works: revoking the share sets `revoked_at`, which the validator checks. Tokens travel in `Authorization: Bearer …` or `?t=…`; scrubbed from all logs.
- **CSRF.** State-changing endpoints (`POST`/`PUT`/`DELETE`) require either a custom header (`X-NasFiles-Request: 1`) that browsers won't set on cross-origin forms, or an anti-CSRF token for forms. Same-site cookies are a layer, not the only layer.
- **SSO security.** OIDC: PKCE mandatory, state + nonce verified, ID token signature + issuer + audience + expiry verified (let `openidconnect` do it, don't roll it). SAML: verify assertion signature against IdP cert, check `NotBefore`/`NotOnOrAfter`, bind to `InResponseTo`, reject reused `AssertionID`s (replay cache with TTL).
- **Upload hardening.** Enforce configurable max file size and max-per-request size. Stream to disk, never buffer full files in memory. Reject filenames containing NUL / path separators / control chars. Strip any client-supplied path components; reconstruct server-side from the target folder + sanitized basename. No execution bit on uploaded files.
- **Serving files.** `Content-Disposition: attachment` by default for untrusted-ish types. `Content-Type` from server-side sniff, not client-provided. `X-Content-Type-Options: nosniff`. Serve user content from a distinct origin path (or ideally a cookieless subdomain once DNS is available) to mitigate stored-XSS pivoting into session theft. CSP: `default-src 'self'`, no inline scripts (enforced by Vite build), no `unsafe-eval`.
- **Headers everywhere.** `Strict-Transport-Security` (when https), `X-Frame-Options: DENY` (except share viewer, where document-specific), `Referrer-Policy: strict-origin-when-cross-origin`, `Permissions-Policy` tight.
- **Audit log integrity.** Every share access — successful or failed — is logged with timestamp, IP, user agent, action, path. Admins can view per-share and global audit. Log redaction: never write share tokens, session cookies, passwords, or Authorization headers into any log.
- **Dependencies.** `cargo-deny` + `cargo-audit` in CI, fail build on advisories. `npm audit --omit=dev` in CI. Lockfiles committed. Renovate/dependabot on.
- **Secrets.** Never baked into the binary. `SESSION_SECRET` refuses weak/empty values. Log scrubber filters env-var values from error messages.
- **Fuzzing.** At minimum: `cargo fuzz` targets for `safe_path::resolve` and the share-token parser. Run in CI on a schedule.
- **Release gate.** No v1 ship without an external security review of: path resolver, share access handler, SSO callback handlers, upload handler. Budgeted, not optional.

---

## Architecture Overview

```
┌─────────────────────┐        ┌───────────────────────────────┐
│  React SPA (Vite)   │ ─HTTP─▶│  Rust backend (axum)          │
│  - Grid/List/Tree   │        │  - /api/*       authed JSON   │
│  - Previews/Upload  │        │  - /s/{token}   share viewer │
└─────────────────────┘        │  - /assets/*    embedded SPA  │
                               └──────┬──────────────┬─────────┘
                                      │              │
                          ┌───────────▼───┐   ┌──────▼─────────┐
                          │ sqlx (Any)    │   │ Filesystem     │
                          │ SQLite | PG   │   │ COMMON_FOLDERS │
                          └───────────────┘   │ HOME_FOLDER_ROOT│
                                              └────────────────┘
```

One static binary. Frontend assets embedded via `rust-embed` so deployment is a single file + env vars + mount points. SFTP/SMB/etc. run outside this process against the same host paths.

---

## Configuration (all via env vars)

```
# Server
BIND_ADDR=0.0.0.0:8080
BASE_URL=https://files.example.com        # used to build share links
SESSION_SECRET=<64 random bytes hex>
DATA_DIR=/var/lib/nasfiles                # thumbnail cache, tmp

# Database — scheme dispatches driver (sqlite:// or postgres://)
DB_URL=sqlite:///var/lib/nasfiles/app.db
# DB_URL=postgres://user:pw@host/nasfiles

# Folder mounts — single JSON env var so display names can contain spaces / mixed case.
# Keys are the user-facing folder names; values are absolute host paths.
COMMON_FOLDERS={"Media":"/mnt/data/media","Documents":"/mnt/ssd/documents","Share with Casing":"/mnt/data/other"}
HOME_FOLDER_ROOT=/mnt/homes               # optional; unset = no home folders

# SSO (enable one or both)
SSO_OIDC_ISSUER_URL=https://idp.example.com
SSO_OIDC_CLIENT_ID=...
SSO_OIDC_CLIENT_SECRET=...
SSO_SAML_METADATA_URL=https://idp.example.com/metadata
SSO_SAML_ENTITY_ID=https://files.example.com
# claim names
SSO_USERNAME_CLAIM=preferred_username
SSO_DISPLAY_NAME_CLAIM=name
SSO_PICTURE_CLAIM=picture
SSO_GROUPS_CLAIM=groups

# Group → folder mapping (one env var per group). Values are comma-separated folder names
# that must exist as keys in COMMON_FOLDERS. Folder names may contain spaces — use commas only
# as the separator (no escaping needed because folder names can't themselves contain commas).
SSO_GROUP_STAFF_COMMON_FOLDERS=Media,Documents,Share with Casing
SSO_GROUP_GUESTS_COMMON_FOLDERS=Media
# Default folders are granted to EVERY authenticated user, in addition to any group-mapped folders.
# Final set = union(defaults, folders mapped from user's groups). Empty = no defaults.
SSO_DEFAULT_COMMON_FOLDERS=Media
SSO_ADMIN_GROUPS=ADMINS                   # optional — admins see share audit across users

# Shares / thumbnails
SHARE_TOKEN_BYTES=24
THUMBNAIL_CACHE_DIR=${DATA_DIR}/thumbs
LOG_LEVEL=info
```

---

## Backend Design (Rust)

### Crate selection
| Concern | Crate |
|---|---|
| HTTP framework | `axum` + `tower` + `tower-http` |
| Async runtime | `tokio` |
| DB (both SQLite + Postgres from one URL) | `sqlx` with `any`, `sqlite`, `postgres` features |
| Sessions | `tower-sessions` with `tower-sessions-sqlx-store` |
| OIDC | `openidconnect` |
| SAML2 | `samael` |
| Password hashing (share passwords) | `argon2` |
| Random tokens | `rand` + `base64url` |
| JSON / serde | `serde`, `serde_json` |
| Errors | `thiserror` (lib) + `anyhow` (bin) |
| Tracing | `tracing`, `tracing-subscriber` |
| Image thumbnails | `image` + `fast_image_resize` |
| Video thumbnails | shell out to `ffmpeg` (no good pure-Rust option) |
| PDF thumbnails | `pdfium-render` (bundled pdfium binary) |
| Streaming ZIP | `async_zip` |
| Embedded SPA | `rust-embed` |
| Config | `figment` or `envy` (env-first) |

### Source layout

```
nasfiles/
├── Cargo.toml                (workspace)
├── crates/
│   ├── nasfiles-server/      binary
│   │   └── src/
│   │       ├── main.rs
│   │       ├── config.rs       all env parsing
│   │       ├── state.rs        AppState (db, config, thumbnailer)
│   │       ├── auth/
│   │       │   ├── oidc.rs
│   │       │   ├── saml.rs
│   │       │   ├── session.rs
│   │       │   └── middleware.rs   extract User from session
│   │       ├── fs/
│   │       │   ├── roots.rs        root resolution (common vs personal)
│   │       │   ├── safe_path.rs    canonicalize + prefix check
│   │       │   ├── listing.rs
│   │       │   ├── stream.rs       Range, ZIP
│   │       │   └── ops.rs          mkdir/move/delete/upload
│   │       ├── thumb/
│   │       │   ├── cache.rs
│   │       │   ├── image.rs
│   │       │   ├── video.rs
│   │       │   └── pdf.rs
│   │       ├── shares/
│   │       │   ├── model.rs
│   │       │   ├── create.rs
│   │       │   ├── access.rs       token → share + permission check
│   │       │   ├── bearer.rs       HMAC bearer issued after guest-password auth
│   │       │   └── audit.rs
│   │       ├── api/
│   │       │   ├── files.rs
│   │       │   ├── shares.rs
│   │       │   ├── public.rs       /s/{token} + /api/public/*
│   │       │   └── me.rs
│   │       ├── db.rs               sqlx::Any pool builder
│   │       └── assets.rs           rust-embed of dist/
│   └── nasfiles-core/        pure logic (models, path safety) — unit-testable
├── migrations/               sqlx migrations (both drivers)
└── web/                      React app (below)
```

### Database abstraction

Use `sqlx::Any` pool. URL scheme (`sqlite://` vs `postgres://`) selects the driver at runtime. Keep all SQL dialect-compatible:

- use `BIGINT` / `TEXT` / `BOOLEAN` (not `SERIAL`, not `INTEGER PRIMARY KEY AUTOINCREMENT`)
- use `BLOB` where needed (SQLite `BLOB` / PG `BYTEA` — sqlx maps both)
- UUIDs as `TEXT` (not PG `uuid`) for portability
- timestamps as `TEXT` ISO8601 or `BIGINT` unix-ms — pick unix-ms to dodge driver quirks
- avoid stored procedures, triggers, upserts beyond `INSERT ... ON CONFLICT` which both support

Two parallel migration trees (`migrations/sqlite/`, `migrations/postgres/`) only if a statement truly can't be shared. Aim for one shared tree.

### Data model

```sql
users (
  id              TEXT PRIMARY KEY,       -- uuid v4
  external_id     TEXT NOT NULL UNIQUE,   -- SSO subject / issuer:sub
  username        TEXT NOT NULL UNIQUE,
  display_name    TEXT NOT NULL,
  picture_url     TEXT,
  is_admin        BOOLEAN NOT NULL DEFAULT FALSE,
  created_at      BIGINT NOT NULL,
  last_login_at   BIGINT NOT NULL
)

-- Folder access is NOT stored; recomputed from SSO groups + env vars each login,
-- cached in session. Only home-folder presence is implied by HOME_FOLDER_ROOT.

shares (
  id              TEXT PRIMARY KEY,
  token           TEXT NOT NULL UNIQUE,   -- base64url, SHARE_TOKEN_BYTES
  owner_user_id   TEXT NOT NULL REFERENCES users(id),
  root_kind       TEXT NOT NULL,          -- 'common' | 'home'
  root_key        TEXT NOT NULL,          -- common folder name, or home-owner username
  relative_path   TEXT NOT NULL,          -- within root; "" = root
  is_directory    BOOLEAN NOT NULL,
  target_kind     TEXT NOT NULL,          -- 'user' | 'guest' | 'public'
  target_user_id  TEXT REFERENCES users(id),
  password_hash   TEXT,                   -- argon2id, null unless target='guest'
  allow_upload    BOOLEAN NOT NULL,
  allow_download  BOOLEAN NOT NULL,       -- controls bulk/folder download button
  expires_at      BIGINT,                 -- nullable
  created_at      BIGINT NOT NULL,
  revoked_at      BIGINT
)
CREATE INDEX shares_owner_idx ON shares(owner_user_id);
CREATE INDEX shares_token_idx ON shares(token);

share_access_log (
  id              TEXT PRIMARY KEY,
  share_id        TEXT NOT NULL REFERENCES shares(id),
  occurred_at     BIGINT NOT NULL,
  ip              TEXT,
  user_agent      TEXT,
  action          TEXT NOT NULL,          -- 'open'|'download'|'upload'|'list'|'auth_fail'
  path            TEXT                    -- file touched within the share
)
CREATE INDEX sal_share_idx ON share_access_log(share_id, occurred_at);

-- sessions table managed by tower-sessions-sqlx-store
-- (app_tokens deferred — was only needed for WebDAV, which is out of v1)
```

### Path safety (critical file: `fs/safe_path.rs`)

Every filesystem request runs through a resolver that:
1. Looks up the root path from `AppState.config` (common folder by name, or `HOME_FOLDER_ROOT/{username}`).
2. Joins with `relative_path`, `canonicalize()`s, then asserts the canonical result `starts_with` the canonical root. Reject otherwise.
3. Applies per-user ACL: for common folders, the user's session must list that folder in their allowed set (computed as `union(SSO_DEFAULT_COMMON_FOLDERS, folders from each of the user's SSO groups via SSO_GROUP_*_COMMON_FOLDERS)`). For home folders, the root_key must equal the session username.

This is the single chokepoint for path traversal — every file/thumbnail/zip/share handler calls it.

### Roots visible to a user

```
if user has >1 common folder OR home folder present:
    roots = [each allowed common folder, "Personal"(home)]
    show tree with those as top-level
else if user has exactly one common folder and no home:
    roots = [that folder]           -- UI hides the roots level
else if user has only home folder:
    roots = [home]                  -- UI hides the roots level
```

### Auth flow

- **OIDC**: standard auth code flow via `openidconnect`. After token exchange, read claims using the `SSO_*_CLAIM` names. Upsert user by `external_id = issuer + ":" + sub`. Compute allowed common folders as `union(SSO_DEFAULT_COMMON_FOLDERS, folders mapped from each of the user's SSO groups)` — defaults apply to every user, group mappings add to that set. Store in session: `user_id`, `username`, `allowed_common_folders`, `has_home`, `is_admin`.
- **SAML2**: `samael` SP init. ACS POST handler verifies assertion, reads same claims (configurable attribute names). Same post-login path.
- **Session cookie** (for logged-in SSO users only): signed with `SESSION_SECRET`, HttpOnly, SameSite=Lax, Secure when `BASE_URL` is https.
- **Guest shares — no cookie.** `/s/{token}` loads the viewer SPA. If the share has a `password_hash`, the viewer shows a password form and `POST`s to `/api/public/shares/{token}/auth`. On success the server returns a stateless, HMAC-signed bearer token (payload: `share_id`, `iat`, `exp` — 30 min) signed with `SESSION_SECRET`. The client holds it in `sessionStorage` (cleared on tab close) and sends it as `Authorization: Bearer …` on every subsequent API call. For direct download URLs that can't set headers (`<a href>`), the client appends `?t=…`. Reload = re-enter password (acceptable for a guest share). Rationale: no cookie means no ambient auth and no cross-tab/cross-site surface; a stateless HMAC avoids running Argon2id on every request (which would be both slow and a DoS vector). Password is never stored on the client after the initial submit.
- **Public shares (no password)**: same bearer-token flow but skip the password step — the `/auth` endpoint mints a bearer immediately based on token possession. Keeps the server path uniform; avoids special-casing "is there a password?" everywhere.

### Shares — key paths

- Create: validates user owns (or has access to) the path, generates token (`rand::rngs::OsRng` → 24 bytes → base64url), hashes password if `target=guest`.
- Access: `/api/public/shares/{token}/*` routes resolve `token → share`, enforce expiry/revoked, require a valid bearer token (see auth flow) for any operation other than `/auth`, call the same `fs/*` handlers but scoped under the share's root+relative path. Every successful call writes to `share_access_log`; so does every failed bearer validation (`auth_fail`).
- Folder download button surfaces only if `allow_download=true`.
- Guest/public upload requires `allow_upload=true` and `is_directory=true`.

### Thumbnails

- Cache key: `sha256(root_kind + root_key + relative_path + mtime_ns + size_enum)` → cached as JPEG/WebP under `THUMBNAIL_CACHE_DIR`.
- Generation runs on a bounded `tokio::task::spawn_blocking` pool. Per-file mutex so concurrent requests for the same thumbnail generate once.
- Image: decode via `image`, downscale via `fast_image_resize`, encode JPEG quality 80.
- Video: `ffmpeg -ss 00:00:03 -i IN -vframes 1 -vf scale=… OUT.jpg`. Shell out; gracefully skip if ffmpeg missing.
- PDF: `pdfium-render` renders page 1 at target size.
- Other file types: frontend ships static SVG icons per extension family — no backend call.

### WebDAV

`dav-server` crate mounted at `/dav/`. Filesystem impl wraps the same path resolver. Basic-auth middleware verifies app token. Read + write. Supports the folder structure: `/dav/{root}/…` where `root` is the same identifier used in the API.

---

## Frontend Design (React)

### Stack
- Vite + React 18 + TypeScript
- **TanStack Router** (typed nested routes) + **TanStack Query** (server state)
- **Tailwind CSS** + **shadcn/ui** components
- **react-dropzone** for upload (supports folder DnD via `webkitdirectory`)
- **pdfjs-dist** for PDF preview
- Native `<img>`/`<video>` with range-supporting API for media preview
- **zustand** for view-mode/selection state

### Layout

```
┌────────────────────────────────────────────────────────────┐
│ TopBar  [ breadcrumb ]          [search]  [avatar ▾]       │
├──────────┬─────────────────────────────────────────────────┤
│          │                                                 │
│ Tree     │  Grid ▭ / List ☰   [upload] [new folder] [⋮]    │
│ ├ Media  ├─────────────────────────────────────────────────┤
│ ├ Docs   │                                                 │
│ ├ Books  │     <FileListing> OR <FilePreview>              │
│ └ Personal│                                                 │
│          │                                                 │
└──────────┴─────────────────────────────────────────────────┘
```

- Sidebar tree lazy-loads children on expand.
- Right pane is a route (`/r/:root/*path`) that shows listing or preview based on whether the path is a dir.
- View mode (grid/list) persisted in localStorage.
- Grid: virtualized tile grid using `@tanstack/react-virtual`; thumbnails requested via `<img loading="lazy" src="/api/files/.../thumbnail?...">` so the browser paces fetches.
- Multi-select via shift/cmd-click. `Download selected` calls `/api/files/{root}/zip?paths=...`.
- Upload zone overlays the pane when files are dragged over. Uses `webkitdirectory` to allow whole-folder drop; sends a single multipart with relative paths so the server recreates structure.
- Share dialog: choose target (user picker / guest-with-password / public), toggles for upload/download/expiry, generates link with copy button. Lists existing shares for the file.
- Admin page (if `is_admin`): global share list, access log viewer. (No user-facing account page in v1 — profile info comes from SSO; app tokens return when WebDAV lands.)

### Directory layout

```
web/
├── package.json
├── vite.config.ts
├── src/
│   ├── main.tsx
│   ├── router.tsx
│   ├── api/                generated or hand-written typed client
│   ├── routes/
│   │   ├── __root.tsx
│   │   ├── index.tsx             redirects to default root
│   │   ├── r.$root.$.tsx         listing or preview
│   │   ├── shares.tsx
│   │   ├── admin.tsx
│   │   └── s.$token.$.tsx        public share viewer
│   ├── components/
│   │   ├── FileGrid.tsx
│   │   ├── FileList.tsx
│   │   ├── FolderTree.tsx
│   │   ├── Breadcrumb.tsx
│   │   ├── PreviewPane.tsx
│   │   ├── ShareDialog.tsx
│   │   ├── UploadZone.tsx
│   │   └── ui/              shadcn-generated
│   ├── state/
│   │   └── view.ts          zustand store
│   └── lib/
│       ├── icons.ts         extension → icon
│       └── preview.ts       mime → previewer
└── public/icons/            fallback file type SVGs
```

Build output `web/dist` is embedded by `rust-embed` into the Rust binary.

---

## Critical Files to Create

| Path | Purpose |
|---|---|
| `Cargo.toml` (workspace) | two-crate workspace |
| `crates/nasfiles-server/src/main.rs` | wire axum router, migrations, SSO init |
| `crates/nasfiles-server/src/config.rs` | env parsing: JSON `COMMON_FOLDERS` map + `SSO_GROUP_*_COMMON_FOLDERS` discovery + union with `SSO_DEFAULT_COMMON_FOLDERS` |
| `crates/nasfiles-server/src/shares/bearer.rs` | issue + verify HMAC bearer tokens for guest-share sessions |
| `crates/nasfiles-server/src/fs/safe_path.rs` | single chokepoint for path resolution + ACL |
| `crates/nasfiles-server/src/fs/stream.rs` | Range + streaming ZIP |
| `crates/nasfiles-server/src/auth/oidc.rs` | OIDC handlers |
| `crates/nasfiles-server/src/auth/saml.rs` | SAML handlers |
| `crates/nasfiles-server/src/shares/access.rs` | token→share, permission + audit |
| `crates/nasfiles-server/src/thumb/cache.rs` | thumbnail cache logic |
| `crates/nasfiles-server/src/db.rs` | `sqlx::Any` pool, DB_URL dispatch |
| `migrations/` | schema above |
| `web/src/routes/r.$root.$.tsx` | main listing/preview route |
| `web/src/components/FolderTree.tsx` | lazy tree |
| `web/src/components/ShareDialog.tsx` | share creation UI |
| `web/src/routes/s.$token.$.tsx` | public share viewer |

---

## Milestones

Every milestone includes design review + security review as gates — not a final "polish" or "hardening" phase.

0. **Design system** — tokens, type scale, motion, dark mode, Storybook scaffold, core primitives (Button, Input, Dialog, Menu, Tree, Grid cell, List row). Nothing shipped to users yet but every later milestone pulls from here.
1. **Skeleton** — workspace, axum hello-world, Vite app, rust-embed pipeline, `DB_URL` driver dispatch, first migration. CI set up with `cargo-deny`, `cargo-audit`, `npm audit`, `clippy -D warnings`, `cargo fuzz` target scaffolding.
2. **Auth** — OIDC login → session → `/api/me`. PKCE, state/nonce, session rotation, cookie hardening. Group→folders env-var mapping.
3. **Filesystem read path** — `fs/safe_path.rs` (with full test suite — traversal, symlinks, case-insensitive FS, unicode normalization), listing API, tree API, download with Range, frontend tree + list + grid (with skeletons, empty states, keyboard nav, dark mode). No thumbnails yet.
4. **Thumbnails** — cache dir, image generator, video (ffmpeg sandboxed via subprocess timeout + resource limits), PDF (pdfium). Blurhash placeholder during load.
5. **Write ops** — mkdir/rename/move/delete/upload incl. folder DnD. Upload hardening (size limits, streaming, filename sanitization). Multi-select with Finder-grade feel.
6. **Shares** — create/list/revoke, public viewer at `/s/{token}`, guest password flow (Argon2id, rate-limited), access log. Share dialog held to the "hero screen" bar.
7. **Streaming ZIP** — multi-file selection + folder download.
8. **SAML** — second SSO path with full assertion verification + replay cache.
9. **Admin views** — global share audit, access-log viewer.
10. **External security review** — path resolver, share access, SSO callbacks, upload. Block v1 on findings.
11. **Packaging** — Dockerfile (multi-stage: node build → cargo build → distroless runtime), signed release artifacts, SBOM.

**Post-v1 (tracked but not on the v1 critical path):** WebDAV (`dav-server` mount + app-token UI) — deferred per decision to rely on SFTP/SMB for mount use cases in v1.

---

## Verification

- **Path safety**: unit tests in `nasfiles-core` that feed `..`/symlinks/absolute paths into the resolver and assert rejection. One test per root type.
- **DB portability**: `cargo test --features sqlite` and `cargo test --features postgres` both run the same integration suite (spin PG via `testcontainers`).
- **Auth**: OIDC tested against a Keycloak in `docker-compose.test.yml`; assert user upsert and correct folder mapping for a multi-group user.
- **Shares**: integration test creates a guest share with password, asserts wrong password → 401 + audit `auth_fail`, correct password → 200 + audit `open`. Test expiry + revoked.
- **Range**: `curl -H "Range: bytes=1000-1999"` on a large file returns 206 with exactly 1000 bytes.
- **ZIP**: download ZIP of a nested folder, unzip, diff against source.
- **Thumbnails**: generate for a sample image/video/PDF; hit endpoint twice and verify second call is a cache hit (instrumented log).
- **End-to-end smoke**: `docker-compose up` with Keycloak + NASDrive + a bind-mounted folder; log in as two users in different groups and verify each sees only their assigned common folders + their own home folder.
- **Security-specific**:
  - Path resolver fuzz (`cargo fuzz`) running ≥1h in CI finds no escapes.
  - Automated test: user A cannot access user B's home folder via any endpoint (listing, download, thumbnail, ZIP, or by crafting a share for a path they don't own).
  - Automated test: expired/revoked share tokens return 404 with no timing leak vs non-existent tokens.
  - Automated test: bearer issued for share A is rejected on share B's endpoints. Revoking the share invalidates live bearers immediately.
  - Automated test: guest share enforces rate limit after 5 bad passwords and audits every attempt.
  - `curl` scan of response headers asserts CSP, HSTS, `X-Content-Type-Options`, `Referrer-Policy`, `X-Frame-Options` on every route class.
  - External security review sign-off (see milestone 11).
- **UI-specific**:
  - Design review in both light + dark mode for every route before it's considered done.
  - Storybook visual-regression snapshots on every component's key states.
  - Manual keyboard-only walkthrough: can a user upload, navigate, preview, share, and log out without touching the mouse?
  - Lighthouse a11y ≥95 on the main listing route.
  - 60fps scroll verified on a 10,000-item directory.
