---
slug: sftp-server
title: Inside the SFTP server
components: [sftp, fs, core]
order: 4
defaultSnippetLines: 24
---

NASDrive runs a full SSH server in the same process as its web API, using
`russh` and `russh_sftp`. This tour walks from process startup and host-key
generation, through public-key authentication, to the SFTP protocol handlers
that turn `readdir` and `open` into real filesystem calls.

Three things distinguish it from a stock SFTP server:

- **There is no shell.** `subsystem_request` accepts the literal string `sftp`
  and nothing else; there is no `exec` and no PTY.
- **There is no real filesystem root.** `/` is a synthetic
  [virtual root](glossary:virtual-root) whose entries are the storage roots this
  principal can read. Every path is re-resolved against the ACL on every
  operation.
- **Authorization is re-checked continuously.** `ensure_active` runs at the top
  of nearly every handler and re-queries the database, so revoking an SSH key
  in the web UI terminates sessions that are already open.

A second kind of principal exists alongside normal users: an admin can mint a
temporary `guest` account pinned to one path with an expiry, which is how
NASDrive hands out time-limited SFTP drop points.
