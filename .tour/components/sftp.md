---
slug: sftp
title: SFTP server
summary: A russh-based SSH server exposing the same roots over SFTP.
order: 50
---

[`crates/nasfiles-server/src/sftp/`](crates/nasfiles-server/src/sftp/mod.rs) runs
an SSH server inside the same process as the HTTP API, sharing its config and
database pool.

- [`server.rs`](crates/nasfiles-server/src/sftp/server.rs) — listener startup,
  public-key authentication, the `sftp` subsystem, and the full
  `russh_sftp::server::Handler` implementation.
- [`keys.rs`](crates/nasfiles-server/src/sftp/keys.rs) — key normalization, so
  the fingerprint stored by the web UI is byte-identical to the one computed
  during SSH auth.
- [`sessions.rs`](crates/nasfiles-server/src/sftp/sessions.rs) — the live
  registry backing the admin "active connections" view.
- [`api.rs`](crates/nasfiles-server/src/sftp/api.rs) — the HTTP endpoints for
  managing your own keys and, for admins, temporary guest accounts.

Authentication is **public key only**. Passwords are never accepted. Clients see
a [virtual root](glossary:virtual-root) directory listing their accessible roots.
