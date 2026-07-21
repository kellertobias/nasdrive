---
slug: authentication
title: Authentication, end to end
components: [web, auth, core]
order: 1
defaultSnippetLines: 24
---

Follow a single login from the moment someone types a password, through rate
limiting, a second factor, and session creation — then keep going into the
*next* request to see how the server decides, over and over, that this person is
still who the cookie says they are.

The interesting design decision here is that NASDrive **does not trust its own
session cookie**. The serialized user in the session is treated as a cache, not
as authority: `current_session_user` reloads the row from the database on every
single request, which is what makes "log out everywhere on password change" and
"permission changes apply immediately" fall out for free.

Watch for three security mechanisms that are easy to miss on a first read:

- a [CSRF header](glossary:csrf-header) that is enforced in two independent
  places, because the login routes sit outside the auth middleware,
- a TOTP [replay guard](glossary:totp-replay-guard) built out of a conditional
  `UPDATE` rather than a lock,
- and [session fixation](glossary:session-fixation) defence via `cycle_id()`.
