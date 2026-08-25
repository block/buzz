## Problem

Implements https://github.com/block/buzz/issues/3280.

Adding a channel-scoped feature to the desktop client means editing a spread
of centrally-owned files in lockstep. Two concrete instances of this exist in
current upstream `main` today:

1. **The settings-section wiring** (`SettingsPanels.tsx` / `SettingsView.tsx`):
   a `SettingsSection` union type, a `SETTINGS_SECTION_VALUES` array,
   `isSettingsSection`, a `settingsSections` descriptor array, a
   `renderSettingsSection` `switch` with a `never` exhaustiveness gate, and
   `SettingsView`'s separate `settingsNavGroups` map — five parallel
   structures a new settings section has to touch.
2. **Channel classification** is scattered: `ChatHeader`'s `ChannelIcon`
   re-derives "what kind of channel is this" (dm → private → forum → hash)
   as an inline `if`-chain, and `ChannelScreen` independently re-checks
   `activeChannel.channelType === "forum"` in four separate places to decide
   what to render and how to lay it out.

This PR is the client-side companion the RFC describes, ported fresh onto
current upstream (our original implementation lived in a disconnected
fork-snapshot repo and couldn't be cherry-picked — see "Provenance" below).

## How

**Commit 1 — settings-section registry.** `settingsSections` is now the
single source of truth: each descriptor carries `value`/`label`/`icon`/
`featureGate` as before, plus `group`, `order`, and a `render(props)` closure
lifted from the old `switch` case. `SettingsView` derives nav grouping and
panel rendering directly from the registry (`SETTINGS_NAV_GROUPS` controls
group order/labels; each descriptor's `group`/`order` controls placement).
Behavior, `data-testid`s, and section order/grouping are unchanged, including
the pre-existing `"moderation"` section, which isn't wired into any nav group
both before and after this change.

**Commit 2 — `shared/channel-features` registry.** A `ChannelFeaturePlugin`
registry modeled on the existing `shared/features/` flag manifest ("typed
definition list + resolver hook + gate"):

```ts
interface ChannelFeaturePlugin<T> {
  id: string;
  parseBinding: (channel: ChannelClassifyInput) => T | null;
  glyph?: LucideIcon;
  priority?: number; // lower runs first; ties keep registration order
}
```

Four built-in plugins (`dm`, `private-channel`, `forum`, `stream`,
registered in priority order in `builtins.ts`) reproduce `ChatHeader`'s
exact dm → private → forum → hash cascade, and now back two call sites that
used to independently re-derive it:

- `ChatHeader`'s `ChannelIcon` calls `channelGlyph({channelType, visibility})`
  instead of the inline `if`-chain.
- `ChannelScreen` computes `isActiveChannelForum` once via
  `classifyChannel(activeChannel)?.pluginId === "forum"` and reuses it for
  the forum/chat content dispatch, the single-panel-view check, the
  transparent-chrome check, the timeline-loading gate, and the "manage
  channel" action's forum branch — five sites that previously re-checked
  `channelType === "forum"` independently.

Registration happens at module scope in `shared/channel-features/index.ts`
(mirroring how `shared/features/manifest` loads its manifest at import time),
so any call site that imports from the barrel gets the built-ins for free.

### What's intentionally *not* ported

The RFC's fuller proposed surface — `tabs`, `settingsPanel`, `sidebar`
group/create-actions, `headerAction` — is **not** in this PR. Our original
implementation had those because our fork added genuinely new channel types
(product/repo/board-hierarchy channels) that needed tab bars, sidebar
groups, and settings panels of their own. Current upstream `main` has no
such second consumer yet: the only two dispatch points that exist today
(`ChatHeader`'s glyph, `ChannelScreen`'s forum/chat split) are a binary
classification, not a multi-tab surface, so forcing in `ChannelFeatureTab`/
`ChannelFeatureShell`/sidebar-group machinery now would be speculative
plugin-surface for zero real callers — exactly the kind of premature
abstraction the RFC's own "behavior-preserving, not a new privileged lane"
framing argues against. See Follow-ups below for the concrete trigger that
would justify porting the rest.

## Relationship to #3275

#3275 ("host MCP Apps as channel tabs") is the motivating concrete case for
the RFC: it extends `ChannelScreen`'s shared shells directly to add a new
tab type. This PR does not touch #3275's code and its content dispatch
(forum vs. chat) is orthogonal to MCP-App tabs (which install *within* a
channel already classified as a normal chat channel), so there's no merge
conflict or ordering dependency between the two.

The natural follow-up once both land: an MCP-App-tabs plugin would extend
`ChannelFeaturePlugin` with a `tabs` field (as the RFC describes) and
register its tab bar for channels with an installed App — the same
extension point `#3275`'s review question 4 asks about ("Does the current
typed channel-surface seam compose cleanly with the behavior-preserving
registry proposed in #3280?"). This PR doesn't answer that by shipping the
`tabs` field pre-emptively (no real second tab-contributing plugin exists
in this repo yet to shape it against); it answers it by proving out the
`parseBinding`/`glyph`/priority-cascade mechanics on two real call sites so
that field has a proven foundation to extend.

## Testing

- `pnpm typecheck` — clean.
- `pnpm lint` (`biome check`) — clean on all touched/added files (two
  pre-existing warnings and two pre-existing infos elsewhere in the repo,
  unrelated to this change, unchanged by it).
- `pnpm test` (unit, `node --test`) — **5463 passed**, 0 failed, including
  the new `desktop/src/shared/channel-features/registry.test.mjs` (11 tests:
  built-in plugin registration order, idempotent re-registration, the
  dm/private/forum/stream classification cascade and its precedence rules,
  and `registerChannelFeature`'s sort/dedup behavior).
- `pnpm build:e2e` — clean build.
- `pnpm exec playwright test --project=smoke --grep "forum|Forum|settings|Settings|sidebar|Sidebar"`
  — **112 passed, 1 skipped, 0 failed** (covers
  `settings-section-layout.spec.ts`, `sidebar.spec.ts`,
  `sidebar-offcanvas-rail.spec.ts`, `sidebar-relay-card.spec.ts`,
  `sidebar-snapshot.spec.ts`, `sidebar-more-unread-overlap.spec.ts`,
  `hosted-communities-settings-screenshots.spec.ts`,
  `invites-settings-screenshots.spec.ts`,
  `profile-backup-settings.spec.ts`, `voice-settings.spec.ts`, and the
  forum-touching cases inside the broader channel specs).
- Did **not** run the full Playwright suite (`pnpm test:e2e`) or the Rust
  side (`just ci` / `cargo test --manifest-path desktop/src-tauri/Cargo.toml`)
  — this PR has no Rust changes and no diff outside `desktop/src`, and the
  full JS/TS suite plus the targeted grep above already cover every spec
  that touches channel tabs, sidebar hierarchy, and settings. Re-running the
  targeted grep is fast (~3 min); a reviewer with more time budget may want
  the full suite for extra confidence.

### Screenshots

Captured locally via a dev build + Playwright against the mock bridge
(`pnpm build:e2e` + a throwaway script driving `installMockBridge`/
`openSettings`, not committed):

- Settings → Appearance, showing the registry-rendered nav groups
  (Personal/Communities/App) and panel content unchanged from before.
- The `#general` channel, showing the `stream` plugin's `Hash` glyph in the
  channel header — the same icon `ChatHeader`'s old inline cascade produced
  for a plain open stream channel.

These aren't posted via `scripts/post-screenshots.sh` because that script
posts to an open PR (`gh pr create` was intentionally not run for this
branch — see the task instructions this branch was prepared under). Whoever
opens the actual PR from `upstream-pr/channel-feature-registry-seam-b` should
re-capture and post screenshots through that script at PR-creation time.

## Duplicate check (re-verified today)

`gh api search/issues -f q='repo:block/buzz channel feature registry ChannelFeaturePlugin'`
returns exactly two results: issue #3280 (this RFC) and PR #3275 (the MCP
Apps host, discussed above). No other open or closed issue/PR implements a
channel-feature/settings-section registry for the desktop client.

## Follow-ups

- **Port the `tabs`/`ChannelFeatureShell` surface** once a second real
  tab-contributing plugin exists in this repo (e.g. an MCP-App-tabs plugin
  building on #3275, or a future Sequence/board/docs-style channel type).
  Shaping `ChannelFeatureTab<T>` against a hypothetical single consumer
  risks guessing wrong; a second concrete caller is the right trigger.
- **Sidebar group/create-action surface** (`ChannelFeatureSidebar`) — same
  reasoning; upstream has no repo/product-style sidebar hierarchy today to
  drive the design.
- **`headerAction`** — same; no plugin needs to contribute a header action
  yet (our fork's needed this for a "New idea" dialog action that doesn't
  exist upstream).
- Consider whether `channelGlyph`'s `stream`-vs-`Hash`-styling special case
  in `ChatHeader.tsx` (the `Glyph === Hash` check, preserved from the
  original inline code's distinct `CHANNEL_HASH_ICON_CLASS`/`color="gray"`
  treatment) is worth generalizing into a per-plugin style hook, or left as
  the one acknowledged wart of an otherwise uniform glyph lookup.

## Provenance

The design was originally implemented and reviewed in a fork whose git
history isn't connected to this upstream (a content-snapshot import, not a
real fork), so it couldn't be cherry-picked or rebased. This PR is a fresh
port of the mechanism onto current `upstream/main`, adapted to what
upstream's `ChannelScreen`/`ChatHeader`/settings code actually look like
today (which has drifted from our fork's snapshot — the settings-section
descriptor shape, the channel dispatch's actual branch points, and the
absence of any custom channel-type/tab surface all differ from what the
original diff assumed). See the "What's intentionally not ported" section
above for the specific scope this drift and the "behavior-preserving"
constraint together produced.
