---
slug: auth
title: Authentication
summary: Local passwords, TOTP, passkeys, OIDC/SSO, and the session middleware.
order: 20
---

[`crates/nasfiles-server/src/auth/`](crates/nasfiles-server/src/auth/mod.rs)
implements every way a human becomes an `AuthUser`:

- [`local.rs`](crates/nasfiles-server/src/auth/local.rs) — password login,
  rate limiting, TOTP, trusted devices, passkeys. By far the largest module.
- [`oidc.rs`](crates/nasfiles-server/src/auth/oidc.rs) /
  [`refresh.rs`](crates/nasfiles-server/src/auth/refresh.rs) — SSO login and
  periodic group re-synchronization against the identity provider.
- [`session.rs`](crates/nasfiles-server/src/auth/session.rs) — a
  database-backed `tower_sessions` store, so logout is a real server-side
  revoke rather than a cookie deletion.
- [`middleware.rs`](crates/nasfiles-server/src/auth/middleware.rs) —
  `require_auth`, applied once over the whole `/api` router, plus the
  `CurrentUser` extractor every handler uses.

Sessions are re-validated against the database on **every** request rather than
trusted from the cookie — see the [session revalidation](glossary:session-revalidation)
concept.
