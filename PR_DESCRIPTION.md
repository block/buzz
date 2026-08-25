# desktop: channel-feature registry (settings sections + channel classification)

Implements #3280.

## Problem

Adding a channel-scoped feature to the desktop client means editing several
centrally-owned files at once. Two concrete cases of this exist in current
upstream `main` today:

1. **Settings-section wiring** (`SettingsPanels.tsx` / `SettingsView.tsx`):
   a `SettingsSection` union type, a `SETTINGS_SECTION_VALUES` array,
   `isSettingsSection`, a `settingsSections` descriptor array, a
   `renderSettingsSection` switch with a `never` exhaustiveness gate, and
   `SettingsView`'s separate `settingsNavGroups` map. Five parallel
   structures a new settings section has to touch.
2. **Channel classification is scattered.** `ChatHeader`'s `ChannelIcon`
   re-derives "what kind of channel is this" (dm, private, forum, hash) as
   an inline if-chain, and `ChannelScreen` separately re-checks
   `activeChannel.channelType === "forum"` in four different places to
   decide what to render.

This PR is the client-side companion the RFC describes, ported fresh onto
current upstream. Our original implementation lived in a fork whose git
history isn't connected to this repo (a content-snapshot import, not a real
fork), so it couldn't be cherry-picked. See "Provenance" below.

## How

**Commit 1, settings-section registry.** `settingsSections` is now the
single source of truth: each descriptor carries `value`/`label`/`icon`/
`featureGate` as before, plus `group`, `order`, and a `render(props)`
closure lifted from the old switch case. `SettingsView` derives nav
grouping and panel rendering directly from the registry. Behavior,
`data-testid`s, and section order/grouping are unchanged, including the
pre-existing `"moderation"` section, which isn't wired into any nav group
either before or after this change.

**Commit 2, `shared/channel-features` registry.** A `ChannelFeaturePlugin`
registry modeled on the existing `shared/features/` flag manifest (typed
definition list, resolver hook, gate):

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
existing dm, private, forum, hash cascade exactly, and now back two call
sites that used to derive it independently:

- `ChatHeader`'s `ChannelIcon` calls `channelGlyph({channelType, visibility})`
  instead of the inline if-chain.
- `ChannelScreen` computes `isActiveChannelForum` once, via
  `classifyChannel(activeChannel)?.pluginId === "forum"`, and reuses it for
  the forum/chat content dispatch, the single-panel-view check, the
  transparent-chrome check, the timeline-loading gate, and the "manage
  channel" action's forum branch. Five sites that used to check
  `channelType === "forum"` on their own.

Registration happens at module scope in `shared/channel-features/index.ts`,
the same way `shared/features/manifest` loads at import time, so any call
site that imports from the barrel gets the built-ins for free.

## Not in this PR

The RFC's fuller proposed surface, `tabs`, `settingsPanel`, `sidebar`
group/create-actions, `headerAction`, is not in this PR. Our original
implementation had those because our fork added new channel types
(product/repo/board-hierarchy) that needed their own tab bars, sidebar
groups, and settings panels. Current upstream `main` has no second
consumer yet: the only two dispatch points that exist today (`ChatHeader`'s
glyph, `ChannelScreen`'s forum/chat split) are a binary classification, not
a multi-tab surface. Adding the tab/sidebar-group machinery now would be
speculative for zero real callers. See Follow-ups below for what would
justify porting the rest.

## Relationship to #3275

#3275 ("host MCP Apps as channel tabs") is the motivating case for the RFC:
it extends `ChannelScreen`'s shared shells directly to add a new tab type.
This PR doesn't touch #3275's code, and its content dispatch (forum vs.
chat) is orthogonal to MCP-App tabs, which install within a channel that's
already classified as a normal chat channel. There's no merge conflict or
ordering dependency between the two.

The natural follow-up once both land: an MCP-App-tabs plugin would extend
`ChannelFeaturePlugin` with a `tabs` field, as the RFC describes, and
register its tab bar for channels with an installed app. This PR doesn't
ship that field pre-emptively, since there's no real second tab-contributing
plugin in this repo yet to shape it against. It answers the same question
by proving the `parseBinding`/`glyph`/priority-cascade mechanics on two real
call sites, so that field has something real to build on.

## Testing

- `pnpm typecheck`: clean.
- `pnpm lint` (`biome check`): clean on all touched/added files (two
  pre-existing warnings and two pre-existing infos elsewhere in the repo,
  unrelated and unchanged).
- `pnpm test` (unit, `node --test`): 5463 passed, 0 failed, including the
  new `desktop/src/shared/channel-features/registry.test.mjs` (11 tests:
  built-in plugin registration order, idempotent re-registration, the
  dm/private/forum/stream classification cascade and its precedence, and
  `registerChannelFeature`'s sort/dedup behavior).
- `pnpm build:e2e`: clean build.
- `pnpm exec playwright test --project=smoke --grep "forum|Forum|settings|Settings|sidebar|Sidebar"`:
  112 passed, 1 skipped, 0 failed (covers `settings-section-layout.spec.ts`,
  `sidebar.spec.ts`, `sidebar-offcanvas-rail.spec.ts`,
  `sidebar-relay-card.spec.ts`, `sidebar-snapshot.spec.ts`,
  `sidebar-more-unread-overlap.spec.ts`,
  `hosted-communities-settings-screenshots.spec.ts`,
  `invites-settings-screenshots.spec.ts`, `profile-backup-settings.spec.ts`,
  `voice-settings.spec.ts`, and the forum-touching cases in the broader
  channel specs).
- Did not run the full Playwright suite or the Rust side (`just ci`). This
  PR has no Rust changes and no diff outside `desktop/src`, and the full
  JS/TS suite plus the targeted grep above already cover every spec that
  touches channel tabs, sidebar hierarchy, and settings. A reviewer with
  more time may want the full suite for extra confidence.

### Screenshots

Captured locally against a dev build and the mock bridge:

- Settings, Appearance panel, showing the registry-rendered nav groups
  (Personal/Communities/App) and unchanged panel content.
- The `#general` channel, showing the `stream` plugin's Hash glyph in the
  channel header, the same icon the old inline cascade produced for a
  plain open stream channel.

These weren't posted through `scripts/post-screenshots.sh`, since that
script needs an open PR and none was opened for this branch. Whoever opens
the PR from `upstream-pr/channel-feature-registry-seam-b` should recapture
and post screenshots through that script at PR-creation time.

## Follow-ups

- Port the `tabs`/`ChannelFeatureShell` surface once a second real
  tab-contributing plugin exists (an MCP-App-tabs plugin building on
  #3275, or a future Sequence/board/docs-style channel type). A second
  concrete caller is the right trigger, shaping it against one hypothetical
  consumer risks guessing wrong.
- Sidebar group/create-action surface (`ChannelFeatureSidebar`), same
  reasoning. Upstream has no repo/product-style sidebar hierarchy today to
  drive the design.
- `headerAction`, same reasoning. No plugin needs to contribute a header
  action yet (our fork needed this for a "New idea" dialog action that
  doesn't exist upstream).
- Consider whether `channelGlyph`'s stream-vs-Hash styling special case in
  `ChatHeader.tsx` is worth generalizing into a per-plugin style hook, or
  left as the one acknowledged wart of an otherwise uniform glyph lookup.

## Duplicate check

`gh api search/issues -f q='repo:block/buzz channel feature registry ChannelFeaturePlugin'`
returns exactly two results: issue #3280 (this RFC) and PR #3275 (the MCP
Apps host, discussed above). No other open or closed issue or PR implements
a channel-feature or settings-section registry for the desktop client.

## Provenance

The design was originally implemented and reviewed in a fork whose git
history isn't connected to this upstream repo (a content-snapshot import,
not a real fork), so it couldn't be cherry-picked or rebased. This PR is a
fresh port of the mechanism onto current `upstream/main`, adapted to what
upstream's `ChannelScreen`/`ChatHeader`/settings code actually look like
today, which has drifted from our fork's snapshot: the settings-section
descriptor shape, the channel dispatch's actual branch points, and the
absence of any custom channel-type/tab surface all differ from what the
original diff assumed. See "Not in this PR" above for the scope that drift
and the behavior-preserving constraint together produced.
