---
slug: shares
title: Shares
summary: Creating, redeeming, auditing, and revoking public share links.
order: 30
---

A **share** exposes one file or directory to someone who has no NASDrive
account, addressed by an unguessable token.

- [`shares/create.rs`](crates/nasfiles-server/src/shares/create.rs) — validation,
  permission derivation from the share type, token generation, the INSERT.
- [`shares/access.rs`](crates/nasfiles-server/src/shares/access.rs) — resolving a
  raw token back to a share, and collapsing "missing", "expired" and "revoked"
  into an indistinguishable 404.
- [`shares/bearer.rs`](crates/nasfiles-server/src/shares/bearer.rs) — short-lived
  HMAC [bearer tokens](glossary:share-bearer) issued after a password check.
- [`api/public.rs`](crates/nasfiles-server/src/api/public.rs) — the
  unauthenticated endpoints a visitor actually hits.
- [`auth/share_reconcile.rs`](crates/nasfiles-server/src/auth/share_reconcile.rs)
  and [`auth/share_audit.rs`](crates/nasfiles-server/src/auth/share_audit.rs) —
  automatic revocation when the owner loses the underlying permission, and the
  nightly job that re-checks it.
