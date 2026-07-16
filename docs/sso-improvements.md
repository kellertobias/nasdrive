# Plan: Live SSO permissions, per-folder R/W/Share, optional personal folders

## Context

Today NASDrive has a single permission concept per user: `allowed_common_folders: HashSet<String>`, computed once at OIDC callback from group→folder env vars and stored in the session. There is no read/write/share split — anyone with a folder in their set can do anything in it. Group claims are never refreshed: a user who is removed from an SSO group keeps full access for up to 24 h (session lifetime). There is no way to disable personal folders selectively, and a user with no group mapping silently gets a session with zero visible roots rather than being rejected.

We want:
1. **Live group refresh** from the IdP via a periodic userinfo poll, so permission changes take effect without re-login.
2. **Three capabilities per folder per group**: read / write / share (share implies read).
3. **Per-group personal folder gate** — only members of designated SSO groups get `~`.
4. **Reject login** when the resulting user has no read access anywhere and no personal folder (and isn't admin).

## Approach

- Replace `AuthUser.allowed_common_folders: HashSet<String>` with `folder_permissions: HashMap<String, FolderCaps>` where `FolderCaps { read, write, share }`. `share` implies `read`.
- Extend env-var parsing to discover `SSO_GROUP_<NAME>_FOLDERS_READ`, `_WRITE`, `_SHARE` and compute capabilities by union across the user's groups. Keep the existing `SSO_GROUP_<NAME>_COMMON_FOLDERS` and `SSO_DEFAULT_COMMON_FOLDERS` as a backwards-compatible alias granting **read+write+share** (matches today's behavior). Add `SSO_DEFAULT_FOLDERS_READ/_WRITE/_SHARE`.
- Add `SSO_PERSONAL_FOLDER_GROUPS` (comma list). When set, only users whose groups intersect get `has_home = true`. When unset, today's behavior (everyone with `HOME_FOLDER_ROOT` set gets one) is preserved.
- Persist OIDC tokens in the session at callback time (`access_token`, optional `refresh_token`, `id_token_expires_at`). Add a refresh helper that the auth middleware calls when stale per `SSO_GROUPS_REFRESH_INTERVAL_SECS` (default 300; `0` disables). The helper hits the userinfo endpoint, recomputes permissions, and updates the session `AuthUser`. If the user now has no access, the session is cleared and the request returns 401.
- At callback, after computing permissions, reject if the user has zero read-capable folders AND no personal folder AND is not admin: respond with a friendly HTML page (`/auth/oidc/callback` returns 403 + a small "no access" view) and never store an `AuthUser`.
- Replace ACL check in `resolve_root` with a `resolve_root_with_cap(config, user, root_key, RequiredCap)` that takes the capability the caller needs. Provide thin wrappers per call site so the change is mechanical.

## Data model & types

**`crates/nasfiles-core/src/models.rs`** — change `AuthUser`:

```rust
pub struct AuthUser {
    pub user_id: String,
    pub external_id: String,
    pub username: String,
    pub display_name: String,
    pub picture_url: Option<String>,
    pub folder_permissions: HashMap<String, FolderCaps>, // replaces allowed_common_folders
    pub has_home: bool,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct FolderCaps { pub read: bool, pub write: bool, pub share: bool }
```

Helper methods on `AuthUser`: `can_read(key) / can_write(key) / can_share(key)`, `readable_folders() -> impl Iterator<&String>`, `effectively_no_access() -> bool`.

No DB migration needed (permissions stay in env, not DB).

## Config (`crates/nasfiles-server/src/config.rs`)

- Replace `group_folder_mapping: HashMap<String, Vec<String>>` with `group_folder_caps: HashMap<String, HashMap<String, FolderCaps>>`.
- Replace `default_common_folders: Vec<String>` with `default_folder_caps: HashMap<String, FolderCaps>`.
- Update `discover_group_folder_mapping` → `discover_group_folder_caps`: scan env for the three new suffixes plus the legacy `_COMMON_FOLDERS`. Legacy → all three caps true. Union if multiple suffixes set for same group/folder.
- Rewrite `compute_allowed_folders` → `compute_folder_permissions(config, user_groups) -> HashMap<String, FolderCaps>`. Filter to folders that exist in `config.common_folders`. Normalize: `share` implies `read`.
- Add fields: `personal_folder_groups: Option<Vec<String>>` (None = no gating), `groups_refresh_interval_secs: u64`. Add helper `fn personal_folder_allowed(config, user_groups) -> bool`.

## OIDC callback (`crates/nasfiles-server/src/auth/oidc.rs`)

- After verifying ID token: compute caps, compute `has_home` via new helper (still respect `home_folder_root` presence), compute `is_admin`.
- New gate: if `folder_permissions` is empty AND `!has_home` AND `!is_admin` → return 403 with a minimal HTML page ("Your account has no access to NASDrive. Contact your administrator.") and **do not** insert `AuthUser` into the session. Log structured event with username + groups.
- Store `oidc_access_token` (string) and, if present, `oidc_refresh_token` and `oidc_token_expires_at` (i64 epoch) in session. Store `oidc_groups_refreshed_at` (i64).
- Persist `last_groups: Vec<String>` in session as well so refresh logic can detect membership changes cheaply (optional but useful for log lines).

## Live refresh (`crates/nasfiles-server/src/auth/middleware.rs` + new helper)

- New module `auth::refresh` with `pub async fn maybe_refresh_groups(state, session, user) -> Result<AuthUser, RefreshOutcome>`.
- Inside `require_auth`, before injecting the user, call `maybe_refresh_groups`:
  - If `groups_refresh_interval_secs == 0` → skip.
  - If `now - oidc_groups_refreshed_at < interval` → skip.
  - Otherwise: GET userinfo using stored access token (reuse `reqwest::Client`, hit URL from provider metadata — store the userinfo URL in `OIDC_CLIENT` state at init via `provider_metadata.userinfo_endpoint()`).
  - On 401 from userinfo: try refresh token grant if available; otherwise clear session and return 401.
  - Recompute caps, `has_home`, `is_admin`. If `effectively_no_access()` → clear session and return 401.
  - Update session's `AuthUser` and `oidc_groups_refreshed_at`. Return updated user.
- Dev-bypass branch: skip refresh entirely.

Backwards-compat note: `OIDC_CLIENT` is currently a `OnceCell<ConfiguredClient>`. Add a sibling `OnceCell<reqwest::Url>` for the userinfo endpoint set during `init_oidc_client`. (Cleaner alternative: wrap both in a single struct.)

## File API capability enforcement

**`crates/nasfiles-server/src/fs/roots.rs`**: change `resolve_root` to take a `RequiredCap` enum (`Read`, `Write`, `Share`) and check the corresponding bit on the user's `FolderCaps` for `root_key` (home folder always has full caps for its owner). `visible_roots` now lists folders where `caps.read` is true.

**`crates/nasfiles-server/src/api/files.rs`** — pass the appropriate cap at each call site:
- `list_directory`, `list_tree`, `download_file`, `file_info`, `download_zip`, `thumbnails::get_thumbnail` → `Read`.
- `mkdir`, `rename`, `move_entries`, `delete_entries`, `upload_file` → `Write`. These currently route through an `ops::*` module — update `ops::*` signatures (or have handlers resolve the root themselves and pass the resolved path down).

**`crates/nasfiles-server/src/api/shares.rs`** — `create_share` (POST `/api/shares`): require `Share` cap on the share's root. List/get/revoke remain owner-scoped (no cap check needed — owner-only by `user_id`).

Public shares (`/api/public/...`) are unchanged: they don't go through the auth middleware and don't depend on the owner's current caps (existing token validity governs access).

## Frontend (`web/`)

Minimal changes — most enforcement is server-side, but we should hide actions the user can't perform:

- **`web/src/api/`**: extend the `/api/me` and `/api/roots` response types to expose per-root caps. Server: `api::me::me` returns the new `folder_permissions`; `list_roots` returns `Root { key, display_name, kind, caps: FolderCaps }`.
- **`web/src/routes/index.tsx`** (or wherever the toolbar lives): hide/disable Upload/New folder/Rename/Delete/Move when `!caps.write` and "Share" when `!caps.share`. Pass caps from the current root into the relevant components.
- Add a "no access" landing page rendered when the user is logged out due to no permissions (the backend's 403 HTML already covers this; nothing further required if we're OK with server-rendered).

## Critical files to modify

- `crates/nasfiles-core/src/models.rs` — `AuthUser`, new `FolderCaps`.
- `crates/nasfiles-server/src/config.rs` — env parsing, `compute_folder_permissions`, `personal_folder_allowed`, new fields.
- `crates/nasfiles-server/src/auth/oidc.rs` — store tokens, store userinfo URL, no-access rejection at callback.
- `crates/nasfiles-server/src/auth/middleware.rs` — call refresh helper.
- `crates/nasfiles-server/src/auth/mod.rs` — add `pub mod refresh;`.
- `crates/nasfiles-server/src/auth/refresh.rs` — **new file**, userinfo poll + recompute + session update.
- `crates/nasfiles-server/src/fs/roots.rs` — `RequiredCap`, `resolve_root` signature, `visible_roots` filter on `caps.read`.
- `crates/nasfiles-server/src/api/files.rs`, `api/thumbnails.rs`, `api/shares.rs`, `fs/ops.rs` (if it calls `resolve_root`) — pass capability at each call site.
- `crates/nasfiles-server/src/api/me.rs` — expose per-folder caps.
- `web/src/api/*`, `web/src/routes/index.tsx`, `web/src/components/*` — gate UI actions on caps.
- `README.md` — document the new env vars.

## New env vars (summary)

| Var | Purpose | Default |
|---|---|---|
| `SSO_GROUP_<NAME>_FOLDERS_READ` | Comma list of folders readable by group | — |
| `SSO_GROUP_<NAME>_FOLDERS_WRITE` | Comma list writable | — |
| `SSO_GROUP_<NAME>_FOLDERS_SHARE` | Comma list shareable (implies read) | — |
| `SSO_DEFAULT_FOLDERS_READ/_WRITE/_SHARE` | Defaults for all users | — |
| `SSO_GROUP_<NAME>_COMMON_FOLDERS` (legacy) | Folders with full R/W/Share | — |
| `SSO_PERSONAL_FOLDER_GROUPS` | Comma list of groups granted `~`; unset = all users (current behavior) | unset |
| `SSO_GROUPS_REFRESH_INTERVAL_SECS` | Live-refresh cadence; `0` disables | `300` |

## Verification

1. **Unit tests** in `config.rs` for `compute_folder_permissions`:
   - Legacy `_COMMON_FOLDERS` grants RWS.
   - Multiple groups union correctly.
   - `share` implies `read` even if `_FOLDERS_READ` omits the folder.
   - Folders not in `common_folders` are filtered out.
2. **Integration test** (or manual via dev bypass): set `NASFILES_DEV_USER` with various groups and confirm `/api/roots` and `/api/me` reflect correct caps; confirm write endpoints return 403 when the cap is missing.
3. **Live refresh**: configure short interval (e.g., 30 s), log in via OIDC, change the user's group at the IdP, wait 30 s, hit any authenticated endpoint, observe groups reread and folder visibility updated. Removing all groups → next request returns 401 and clears session.
4. **No-access reject**: configure a user with no group mapping → callback returns 403 page; `/api/me` returns 401; public shares still work.
5. **Personal folder gate**: set `SSO_PERSONAL_FOLDER_GROUPS=STAFF`. User in STAFF gets `~`; user not in STAFF does not. With var unset, both still get `~` (preserves existing behavior).
6. **UI smoke test**: `npm run dev` in `web/`, log in as a read-only user, confirm Upload/New/Delete are hidden or disabled; as a read+share user, confirm Share button is shown but Upload is not; as full RWS, all actions present.
