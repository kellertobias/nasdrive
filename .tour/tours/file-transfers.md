---
slug: file-transfers
title: Copying and moving files
components: [web, fs, core]
order: 3
defaultSnippetLines: 24
---

Drag a folder onto another folder. That gesture becomes a durable, resumable,
cancellable background job — and this tour follows every stage of it.

The architecture is worth stating up front, because the HTTP handler is
deliberately almost empty:

```mermaid
flowchart TD
  A["drag: setFileDragPayload"] --> B["drop: handleFileDrop"]
  B -->|"same root"| C["executeTransfer('move')"]
  B -->|"cross root"| M["copy-or-move modal"]
  M --> C
  C --> D["POST /api/files/:root/transfer"]
  D --> E["create_transfer_job<br/>(one row, status=queued)"]
  E --> F["spawn_file_job"]
  F --> G["plan_copy_move_job<br/>→ file_operation_items"]
  G --> H["execute_copy_like_job<br/>→ per-item copy/rename"]
  H --> I["recompute_progress"]
  I -.->|"polled every 1s"| J["TopBar transfer-jobs query"]
```

The handler validates *nothing*. Every ACL and path check is deferred to the
worker, on purpose: a job recovered after a server restart must re-run those
checks with the original caller's permissions, so the worker has to be able to
do them anyway. Storing the whole `AuthUser` on the job row is what makes that
possible.

Also note that progress is never incremented — it is always **recomputed** from
an aggregate query over the item table, so a crash mid-copy cannot leave the
counters skewed.
