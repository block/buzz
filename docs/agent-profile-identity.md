# Agent profile identity

## An explicit key is exact

Opening a public key from a message, member, DM, deep link, or Instances row
always opens that identity. Active, stopped, archived, and relay-only keys obey
the same rule. A local managed record may supply controls only for that exact
key. An owner-signed kind 30177 `persona_id` (or an archive request's historical
persona link) is **not** an identity alias and does not grant access to a local
sibling's controls, definition, configuration, or Start action.

Only explicit persona navigation (for example, the persona card in My Agents)
selects an archive-aware representative using `pickProfileAgent`. With no live
representative, it remains a persona-only surface and may offer Start. An
explicit relay-only key must not turn into that persona-only surface, even if a
matching definition exists locally. There is no synthetic/secretless
`ManagedAgent` record.

This intentionally supersedes the old historical-message redirect: a message
from stopped A no longer opens running B merely because they share persona P.
The requested key takes precedence even if the caller also supplies persona
context. Local instance profiles may still show their own linked definition and
an explicitly navigable Instances list. Ownership alone permits owner-scoped
relay reads, not local management.

Implementation: `useCanonicalManagedAgentProfile` and `UserProfilePanel`.
The obsolete historical-persona relay lookup and inactive-instance redirect
helper have been removed instead of adding another exception flag.

## Regression gates

- `profile/lib/resolveCanonicalManagedAgent.test.mjs`: exact identity across
  active/stopped/archived/relay-only keys, and persona representative selection.
- `profile/lib/useCanonicalManagedAgentProfile.test.mjs`: remote A cannot borrow
  persona P/local B, including persona-only navigation and returning to A.
- `tests/e2e/exact-key-profile.spec.ts`: relay-only profile controls versus
  explicit persona navigation, with and without a local sibling; exact archived
  keys with a live sibling and persona-only navigation when all are archived.
- `tests/e2e/profile.spec.ts`: historical messages match the exact Instances
  selection rather than the current persona card; profile ingress parity.
- `tests/e2e/identity-archive.spec.ts`: archive/unarchive authority and flair.
