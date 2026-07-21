---
slug: fs
title: Filesystem layer
summary: Roots, listings, streaming, and the durable copy/move job engine.
order: 40
---

[`crates/nasfiles-server/src/fs/`](crates/nasfiles-server/src/fs/mod.rs) is where
requests become disk I/O.

- [`roots.rs`](crates/nasfiles-server/src/fs/roots.rs) — `resolve_root` and
  `visible_roots`. The capability gate shared by HTTP, SFTP and S3.
- [`file_jobs.rs`](crates/nasfiles-server/src/fs/file_jobs.rs) — the
  [file job](glossary:file-job) engine: planning, execution, progress, resume,
  and cancellation for copy and move.
- [`listing.rs`](crates/nasfiles-server/src/fs/listing.rs),
  [`stream.rs`](crates/nasfiles-server/src/fs/stream.rs),
  [`zip.rs`](crates/nasfiles-server/src/fs/zip.rs) — directory listings,
  Range-capable file serving, archive download.
- [`ops.rs`](crates/nasfiles-server/src/fs/ops.rs) — simple synchronous
  operations (rename, delete, mkdir). Note that its transfer helpers are dead
  code superseded by `file_jobs.rs`.
