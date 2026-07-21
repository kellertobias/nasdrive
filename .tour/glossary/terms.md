## Root

A configured storage location a user may be granted access to — either a
**common folder** from `config.common_folders`, or the special key `~` meaning
that user's personal home folder under `home_folder_root`. Roots are the unit of
permission: `AuthUser` carries per-root read/write/share capabilities, and
[`fs::roots::resolve_root`](crates/nasfiles-server/src/fs/roots.rs) checks them
*before* any filesystem access. The web UI, SFTP, and the S3 API all go through
this same function, which is why they cannot drift apart on permissions.

## Safe path

The containment guarantee provided by
[`nasfiles_core::safe_path`](crates/nasfiles-core/src/safe_path.rs). `resolve`
rejects NUL bytes, absolute paths and Windows drive prefixes, then canonicalizes
*both* the root and the joined path and asserts the result still starts with the
root — canonicalizing after the join is what defeats `../` sequences and
symlinks in one step. `resolve_parent` is the variant for paths that do not
exist yet: it canonicalizes the parent, validates the final filename separately,
and rejects a final component that is already a symlink (even a dangling one),
because a later `create` or `rename` would follow it out of the root.

## CSRF header

NASDrive's cross-site-request-forgery defence is the custom request header
`X-NasFiles-Request: 1`, attached by `apiFetch` to every `POST`/`PUT`/`DELETE`/
`PATCH`. A custom header cannot be set by a cross-origin form or image, so its
presence proves the request came from application code rather than from an
attacker's page. It is enforced twice: once in `require_auth` for everything
under `/api`, and again inside `require_local_auth_header` for the `/auth/local/*`
routes, which are registered *outside* the middleware and would otherwise be
unprotected.

## Session revalidation

Rather than trusting the `AuthUser` serialized into the session store,
`local::current_session_user` reloads the user row from the database on every
request. Three behaviours fall out of this: a deleted user is logged out
immediately; a user whose `password_changed_at` is newer than the session's
`local_auth_at` is logged out (this is "sign out all other devices"); and
permission or admin-flag changes take effect on the very next request without
re-login.

## Session fixation

An attack where the attacker fixes a victim's session identifier *before* login
and then reuses it afterwards. The defence is to issue a brand-new session ID at
the moment authentication succeeds — `finish_login` calls `session.cycle_id()`
for exactly this reason, on every successful path (password, TOTP, trusted
device, passkey).

## TOTP replay guard

A six-digit TOTP code is valid for a whole time window plus a skew allowance, so
without extra state the same code could be used twice. NASDrive records the
*matched* counter step in `local_totp.last_used_step` using a conditional
`UPDATE ... WHERE last_used_step IS NULL OR last_used_step < $1`, and treats
`rows_affected() == 0` as failure. Because the matched counter is recorded rather
than the wall-clock one, a code is single-use across the entire skew window,
with no lock required.

## Trusted device

The "remember this computer" feature. The browser stores a second, silent TOTP
secret in `localStorage` alongside a server-issued opaque hash; on login it
submits device id, hash, and a freshly computed code. The server verifies the
hash with a constant-time `ct_eq` and runs the code through the same replay
guard as a normal second factor. The hash is an HMAC keyed by `session_secret`,
so rotating that secret invalidates every trusted device at once.

## Share bearer

The short-lived credential a share visitor receives after passing the password
check. `bearer::issue_bearer` produces an HMAC over `{share_id}:{iat}:{exp}`
with a 30-minute TTL. Deliberately, `verify_bearer` does **not** check whether
the share has been revoked — its doc comment says so — which is safe only
because every public handler re-resolves the share from the token *before*
verifying the bearer. A new endpoint that skipped that step would honour tokens
for up to half an hour past revocation.

## Share type

The four kinds of share — `Typical`, `Gallery`, `Dropbox`, `Collaboration` —
from which `permissions_for_share_type` derives the `(allow_download,
allow_upload)` pair. Typical and Gallery are download-only, Dropbox is
upload-only, Collaboration is both. The client also sends these two booleans,
but `CreateShareRequest` has no fields for them, so serde silently drops them
and the server's derivation is authoritative.

## Permission grace

A two-strike rule guarding automatic share revocation. When an SSO group refresh
observes that a user has lost access to a root, `confirm_permission_loss` does
*not* revoke on the first sighting — it records a `permission_loss_grace` row and
returns `false`. Only a second consecutive observation confirms the loss. This
prevents a single truncated or transient identity-provider response from
destroying every share on a root.

## File job

A row in `file_operation_jobs` describing a copy or move, plus one row per
planned item in `file_operation_items`. Because both the item list and the whole
originating `AuthUser` are persisted, a job survives a server restart: the worker
skips re-planning when items already exist, resumes from the first item whose
status is not `done`, and re-runs every ACL and path check with the original
caller's permissions. Cancellation crosses the task boundary through the database
(`cancel_requested`) rather than through a channel.

## Atomic move

`fs::rename` cannot cross filesystems, so the transfer engine branches on
`job.source_root == job.dest_root`. A same-root move collapses to a single
rename per top-level entry and completes instantly regardless of size; a
cross-root move degrades to a full recursive copy followed by
`cleanup_move_sources`. Source deletion is deferred until every item has copied,
so a mid-job failure leaves the sources completely intact.

## Virtual root

What an SFTP client sees at `/`. There is no single directory on disk holding
every root, so `resolve_user_path` short-circuits `/` to
`ResolvedPath::VirtualRoot` and `opendir` synthesizes a listing from
`roots::visible_roots`. The first path segment is then matched against each
root's display name *or* its key, so both `/Personal/x` and `/~/x` work.
`reject_root_write` makes the root entries themselves immutable — otherwise a
client could `rmdir` a configured common folder.

## SigV4

AWS Signature Version 4, the request-signing scheme every S3 client speaks.
Verification means reconstructing the canonical request (method, URI-encoded
path, sorted canonical query string, sorted trimmed headers, signed-header list,
payload hash), hashing it into a string-to-sign, deriving a signing key through
four chained HMACs from `AWS4{secret}`, and comparing the result in constant
time. Implemented framework-free in
[`nasfiles_core::sigv4`](crates/nasfiles-core/src/sigv4.rs); the Axum-facing half
that gathers the inputs lives in
[`api/s3/auth.rs`](crates/nasfiles-server/src/api/s3/auth.rs).

## S3 principal

The two-variant enum every S3 handler is written against. A `UserToken` is a
normal NASDrive user authenticating with an API token from the profile page —
its buckets are that user's roots. A `ShareCredential` is scoped to exactly one
share, may only address the literal bucket name `share`, and is bounded by the
parent share's own expiry and revocation state.
