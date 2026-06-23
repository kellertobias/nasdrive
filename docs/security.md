# Security Model & Operational Notes

This page documents intentional security behaviors that are easy to mistake for
bugs. They are by design; the notes exist so the reasoning does not have to be
rediscovered each time.

## Path containment

All filesystem access funnels through a single chokepoint
(`nasfiles_core::safe_path`). It canonicalizes the requested path and the root,
then requires the result to stay inside the root, rejecting `..`, absolute
paths, NUL bytes, and symlinks that escape the root. Authorization
(`resolve_root`) is checked *before* any path is resolved, on every file
endpoint, and is re-applied inside the SFTP server and background jobs.

## Usernames and home directories

Personal ("home") folders are keyed on a sanitized form of the username, where
`/`, `\`, and `..` are collapsed to `_`. To keep that mapping collision-free —
so two distinct identities can never resolve to the same home directory —
usernames containing `/`, `\`, `..`, or control characters are rejected at the
source:

- Local user creation and the `SETUP_ADMIN_USER` bootstrap reject such names.
- SSO logins whose username claim contains them are denied with an
  "Access Denied" page rather than being silently sanitized.

## File-operation jobs use an enqueue-time permission snapshot

Move/copy/delete and other file operations run as persistent, resumable jobs.
When a job is enqueued, the requesting user's permission set is captured and
stored with the job. The background worker re-runs the full path-safety and
authorization chokepoint at execution time **against that captured snapshot**,
not against the user's current permissions.

**Consequence:** if a user's access to a folder is revoked *after* they have
already submitted an operation, an in-flight, paused, queued, or
restart-recovered job for that operation can still complete using the
permissions that were in effect when the command was issued.

**Why this is acceptable:** the command was authorized at the moment the user
issued it — the snapshot represents the rights they legitimately held when they
asked for the work. The snapshot still fails *closed* for path safety (a
corrupt/empty snapshot resolves to "no access"), so it can never *grant* more
than the user had at enqueue time; it only declines to retroactively cancel work
that was already authorized. New operations always use current permissions.

## Share bearer tokens have a bounded lifetime

After a guest authenticates to a password-protected share, they receive a
short-lived stateless bearer token (HMAC-signed, ~30 minute TTL). Every share
file operation first calls `resolve_share`, which re-checks the share in the
database, so **revoking or expiring a share takes effect immediately** even for
a guest holding a live bearer token.

**Consequence:** the one thing that is *not* re-checked per request is the share
*password*. A guest who already holds a valid bearer continues to have access
until that bearer expires (up to its TTL), even if the password is rotated in
the meantime. To cut off an active guest immediately, revoke or delete the share
rather than only changing its password.

> This guarantee depends on every share file handler calling `resolve_share`
> before trusting a bearer token. Keep that ordering when adding new share
> endpoints.

## SFTP guest "extend" re-enables a revoked key

Extending a temporary SFTP guest clears its `revoked_at` timestamp. If the guest
was previously revoked, extending it **re-enables the key** and restores access
until the new expiry. The admin UI surfaces this in a tooltip on the Extend
control. To keep a guest disabled, do not extend it; create a fresh guest
instead.

## Login rate limiting

Failed local logins are throttled on two independent keys within a rolling
5-minute window: per normalized username, and per client IP (to throttle
password-spraying across many usernames from one source). The per-IP limit is
higher than the per-username limit so a shared/NAT'd source is not locked out by
a few users. Client IP is taken from `X-Forwarded-For` / `X-Real-IP`, so it is
only meaningful when the server runs behind a trusted reverse proxy that sets
those headers.

## S3 API credential storage

S3 secret keys (both personal API tokens and share credentials) are stored **encrypted** in the database using AES-256-GCM with a per-value random nonce. The encryption key is derived from `SESSION_SECRET`. AWS Signature Version 4 (SigV4) authentication requires the raw secret key for HMAC re-computation, so the keys cannot be hashed the way user passwords are — encryption is the next-best protection.

**Consequence:** the encrypted secrets are only as safe as `SESSION_SECRET`. If both the database and the session secret are compromised, the S3 secrets are exposed. Protect both with appropriate file-system permissions, and ensure backups of each are encrypted and stored separately.

**Protections in place:**
- Secret keys are encrypted at rest; the database alone is not sufficient to recover them.
- Access keys and secret keys use cryptographically random generation (same as share tokens).
- Personal API tokens record `last_used_at`, making silent credential theft visible in the profile page.
- Users can revoke tokens from the profile page at any time; revoked tokens are rejected immediately.
- Share credentials are short-lived (maximum one hour) and are invalidated if the underlying share is revoked.
- Header-based SigV4 authentication rejects requests whose timestamp differs from server time by more than 15 minutes, preventing replay attacks.
- Presigned URLs carry an explicit `X-Amz-Expires` value and are rejected once that window passes.

## S3 permissions are re-checked on every request

Unlike file-operation jobs (which snapshot permissions at enqueue time), S3 requests re-check the authenticated user's live permissions from the database on every call. If a user's read or write access to a folder is revoked, their S3 token immediately loses access to the corresponding bucket — no need to revoke the token itself.

## Dev mode bypasses authentication

When `NASFILES_DEV` is set together with a configured dev user, the auth
middleware injects a synthetic admin user and skips real authentication, and a
weak default session secret is permitted. This is for local development only.
The UI shows a persistent red warning bar whenever this bypass is active. Never
set `NASFILES_DEV` on a publicly reachable instance.
