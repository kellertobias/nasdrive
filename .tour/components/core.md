---
slug: core
title: nasfiles-core
summary: Protocol- and framework-free primitives — path containment, tokens, SigV4.
order: 10
---

[`crates/nasfiles-core`](crates/nasfiles-core/src/lib.rs) holds the logic that
must not depend on Axum, `sqlx`, or any transport. It is deliberately small and
heavily unit-tested, because three separate front doors rely on it:

- [`safe_path`](crates/nasfiles-core/src/safe_path.rs) — the single path
  containment chokepoint. `resolve` for paths that exist, `resolve_parent` for
  paths about to be created.
- [`tokens`](crates/nasfiles-core/src/tokens.rs) — share-token generation,
  hashing, and HMAC bearer tokens.
- [`sigv4`](crates/nasfiles-core/src/sigv4.rs) — AWS Signature Version 4
  canonicalization and constant-time verification.
- [`models`](crates/nasfiles-core/src/models.rs) — `AuthUser` and the
  capability helpers (`can_read`, `can_write`, `can_share`) every gate consults.
