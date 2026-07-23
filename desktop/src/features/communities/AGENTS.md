# Community-switching instructions

Read [`../../../../AGENTS.md`](../../../../AGENTS.md) and
[`../../../AGENTS.md`](../../../AGENTS.md) first.

Community switching intentionally remounts the desktop subtree with a community
key, but module-level state survives that remount. When adding a
community-scoped cache, map, class instance, connection, or long-lived store,
provide a reset and call it from `resetCommunityState()` in
[`useCommunityInit.ts`](useCommunityInit.ts). Preserve the initialization gate:
the app must not render relay-connected content until the active community is
applied to the backend.
