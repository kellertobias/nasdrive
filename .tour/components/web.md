---
slug: web
title: Web frontend
summary: React + TanStack Router SPA, compiled into the server binary.
order: 70
---

[`web/src`](web/src/main.tsx) is a React single-page app using TanStack Router
and TanStack Query, built by Vite and embedded into the Rust binary at compile
time.

- [`api/client.ts`](web/src/api/client.ts) — every HTTP call in the app. The
  single `apiFetch` chokepoint attaches the CSRF header and the session cookie.
- [`routes/`](web/src/routes/__root.tsx) — `__root.tsx` owns the `["me"]`
  identity query; `r.$root.$.tsx` is the file browser; `s.$token.$.tsx` is the
  public share viewer.
- [`components/`](web/src/components/ShareDialog.tsx) — `ShareDialog`,
  `FileGrid`/`FileList`, `TransferProgressIndicator`, `UploadZone`.
- [`lib/`](web/src/lib/fileDrag.ts) — drag payload encoding, transfer-job
  helpers, WebAuthn and TOTP client code.

There is no websocket: progress and identity are polled with TanStack Query
intervals.
