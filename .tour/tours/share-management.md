---
slug: share-management
title: The life of a share link
components: [web, shares, core]
order: 2
defaultSnippetLines: 24
---

A share is the only part of NASDrive a stranger can reach. This tour follows one
from the dialog that creates it, through token generation and persistence, to a
visitor redeeming the link, and finally to the two very different ways a share
can die: someone revokes it, or the owner quietly loses permission to the folder
underneath it.

Two ideas carry most of the weight:

- **Permissions are derived, never accepted.** The browser sends
  `allow_download` and `allow_upload`; the server ignores them and recomputes
  both from `share_type`. A client cannot promote a drop box into a download.
- **Absence is indistinguishable from expiry.** Missing, expired and revoked
  tokens all return the same 404 with the same body, so a token cannot be
  probed for existence.

The tour also flags a genuine wrinkle: `tokens::hash_token` documents that raw
tokens are never stored, and the `token_hash` column honours that — but a second
column, `display_token`, holds the plaintext so the owner can recover the link
later. Worth knowing before you reason about database compromise.
