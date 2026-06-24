# AGENTS.md — AI Agent Contributor Guide

This guide is for AI agents contributing to the Buzz codebase. It covers
agent-specific context and conventions. For general contributor info (setup,
code style, PR process, architecture), see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Ecosystem

Buzz spans five repos. This one (`block/sprout`) is the OSS source for the relay, desktop, mobile, and CLI. The others handle internal builds and deployment:

| Repo | Purpose |
|------|---------|
| [block/sprout](https://github.com/block/sprout) | OSS source — relay, desktop app, mobile app, CLI, agent harness |
| [squareup/sprout-releases](https://github.com/squareup/sprout-releases) | Buildkite pipeline producing Block-signed macOS + iOS builds with `-block` version suffix |
| [squareup/sprout-oss](https://github.com/squareup/sprout-oss) | CI pipeline building the relay Docker image and pushing to internal ECR |
| [squareup/block-coder-tf-stacks](https://github.com/squareup/block-coder-tf-stacks) | Terraform + ArgoCD deploying the relay to the staging Kubernetes cluster |
| [squareup/sprout-backend-blox](https://github.com/squareup/sprout-backend-blox) | Desktop backend provider script connecting Blox workstation agents to the relay |

```
block/sprout (source)
  ├─► sprout-releases    (desktop + mobile builds → Artifactory, GitHub, Mobile Releases)
  ├─► sprout-oss         (relay Docker image → ECR)
  │     └─► block-coder-tf-stacks  (Helm chart → ArgoCD → staging cluster)
  └─── sprout-backend-blox         (Blox compute provider for Desktop agent launch)
```

See [RELEASING.md](RELEASING.md) for the desktop release flow and
[CONTRIBUTING.md § Ecosystem](CONTRIBUTING.md#ecosystem) for contributor
access information.

---

## Repo Structure

```
crates/
  # Relay + core
  buzz-relay          # WebSocket relay server — main entry point; also hosts git + huddle audio
  buzz-core           # Core types, event verification, filter matching, kind registry
  buzz-db             # Postgres event store and data access layer
  buzz-auth           # Authentication and authorization
  buzz-pubsub         # Redis pub/sub fan-out, presence, typing indicators
  buzz-search         # Typesense-backed full-text search
  buzz-audit          # Hash-chain audit log
  buzz-media          # Blossom/S3 media storage
  # Agent surface
  buzz-acp            # ACP harness bridging Buzz events to AI agents
  buzz-agent          # Minimal ACP-compliant agent (non-streaming, tool-calls-as-output)
  buzz-dev-mcp        # Developer MCP server — shell + file-edit tools
  buzz-persona        # Agent persona packs
  buzz-workflow       # YAML-as-code workflow engine (evalexpr conditions)
  # Clients + interop
  buzz-proxy          # Nostr client compatibility proxy (NIP-28)
  buzz-pair-relay     # Ephemeral sidecar relay for NIP-AB device pairing
  buzz-pairing-cli    # CLI for NIP-AB device pairing interop testing
  git-sign-nostr      # Sign git objects with a Nostr key
  git-credential-nostr # Git credential helper for Nostr-authed push/fetch
  # Tooling + shared
  buzz-cli            # Agent-first CLI
  buzz-sdk            # Typed Nostr event builders
  buzz-admin          # Operator CLI for relay administration
  buzz-ws-client      # Shared NIP-42 WebSocket client (connect, auth, publish)
  buzz-test-client    # Integration test client and E2E test suite
  sprig               # All-in-one harness bundling ACP, agent, and dev MCP

desktop/              # Tauri 2 + React 19 desktop app
web/                  # Browser web client (repo browser, served by the relay)
mobile/               # Flutter mobile app
migrations/           # SQL migrations (auto-applied on relay startup)
scripts/              # Dev tooling
.env.example          # Config template — copy to .env before running
```

---

## Getting Started

```bash
. ./bin/activate-hermit   # activate hermit toolchain (Rust, Node, etc.)
cp .env.example .env      # configure local environment
just setup                # install deps, run migrations
just relay                # start relay at ws://localhost:3000
just ci                   # run before any PR
```

See CONTRIBUTING.md for full setup details and dependency requirements.

---

## Quality Gates

Run `just ci` before every PR — it runs `fmt` + `clippy` + desktop lint +
unit tests + builds. Clippy passing does not mean fmt passes; run both.

Run `just test` for integration tests if you touched `buzz-relay`,
`buzz-db`, or `buzz-auth` — these require a running Postgres and Redis.

**Pre-commit hooks** are installed automatically by `just setup` and auto-fix
formatting via `stage_fixed`. Pre-commit runs fix variants in parallel (Rust
fmt, Tauri Rust fmt, desktop biome fix, web biome fix, mobile dart format).
Auto-fixable issues are fixed and re-staged; unfixable lint issues block the
commit. **Pre-push hooks** run clippy (workspace + Tauri) and fast unit tests
in parallel (Rust, desktop JS, Tauri Rust, mobile Flutter) — no overlap with
pre-commit. Builds are CI-only. Run `just fix-all` to auto-fix all formatting
in one shot. Run `just ci` for the full local gate. Run `just hooks` to
re-install hooks after env changes.

Additional rules:
- No `unsafe` code
- Do not introduce new `unwrap()` or `expect()` in production paths — use `?` and proper error types
- New public API must have doc comments

---

## Key Patterns

**Dual API surface**: Buzz exposes both a REST API and a NIP-29 WebSocket
relay. Both paths converge on shared DB functions in `buzz-db`. When adding
a feature, implement the shared DB logic first, then wire up both surfaces.

**Prefer Nostr events over new REST endpoints**: For new feature work, model
the operation as a Nostr event (new kind in `buzz-core/src/kind.rs`, handler
in `buzz-relay`) rather than adding a new REST endpoint. REST is reserved
for things that genuinely need an HTTP-only surface: media upload/download
(Blossom), OAuth callbacks, health checks, and the existing read endpoints
that proxy DB queries. Two helpful endpoints already exist and rarely need
to be duplicated:

- `POST /events` — submit any signed event (same path the WebSocket uses).
- `POST /query` — Nostr REQ filters over HTTP. NIP-50 `search` filters
  are routed to `buzz-search` (Typesense-backed) automatically.
- `POST /count` — Nostr COUNT filters over HTTP.

If you find yourself reaching for a new REST endpoint, first check whether
an event kind would do the job — it usually will, and you get realtime
fan-out, NIP-29 scoping, and the existing auth pipeline for free.

Reference https://github.com/nostr-protocol/nips

**Event kinds**: All event kind integers are defined in
`buzz-core/src/kind.rs`. New features get new kind integers — add them here
first, then implement handling in the relay.

**Channel scoping**: Channels use `h` tags (NIP-29 group tag), not `e` tags.
Filters and queries must scope to `h` tags when operating within a channel.

**Agent-facing operations go in `buzz-cli`**: New agent-facing features belong in `buzz-cli` — add a subcommand there first, then wire the REST/WebSocket call in `client.rs`. `buzz-dev-mcp` (shell + file tools for `buzz-agent`) is separate.

**Workflow conditions**: `buzz-workflow` uses
[evalexpr](https://docs.rs/evalexpr) for condition evaluation. Keep expressions
simple and testable.

**Thread counters**: `reply_count` and `descendant_count` are materialized on
thread root events. Any code that inserts replies must update these counters —
check existing reply handlers for the pattern.

---

## Agent CLI (`buzz-cli`)

`buzz` is the agent-first CLI. Auth env vars
(`BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG`) are auto-injected
by the ACP harness into managed agent subprocesses. In development, set
`BUZZ_PRIVATE_KEY` and `BUZZ_RELAY_URL` in your environment manually.

### Building the CLI

```bash
cargo build --release -p buzz-cli
```

Binary location: `./target/release/buzz`. Add `./target/release` to `PATH`
or invoke with the full path.

### Deep Links

`buzz://message?channel=<uuid>&id=<hex>` links reference a specific message
thread. To read the linked thread:

```bash
buzz messages thread --channel <uuid> --event <hex> --format compact
```

Extract `channel` and `id` from the URL query parameters. The optional
`thread` parameter (root event ID) can be ignored — `messages thread` resolves
the full thread from the event ID alone.

All reads return sig-stripped JSON arrays; all writes return
`{event_id, accepted, message}`; creates add the entity ID. Exit codes:
0=ok, 1=input error, 2=network/relay, 3=auth, 4=other, 5=write conflict (NIP-33 LWW).

`--format compact` is a **global** flag — it goes before the subcommand:
`buzz --format compact channels list`, NOT `buzz channels list --format compact`.

See `crates/buzz-cli/TESTING.md` for the full live-testing runbook.

---

## Testing

```bash
just test-unit    # unit tests, no infrastructure needed
just test         # full integration suite (requires Postgres + Redis)
```

E2E tests live in `crates/buzz-test-client/tests/`:
- `e2e_relay.rs` — WebSocket relay protocol
- `e2e_rest_api.rs` — REST endpoint coverage
- `e2e_tokens.rs` — auth token flows
- `e2e_workflows.rs` — workflow engine
- `e2e_media.rs` — media upload/download (Blossom)
- `e2e_media_extended.rs` — extended media scenarios
- `e2e_nostr_interop.rs` — Nostr interop (NIP-50 search, NIP-10 threads, NIP-17 gift wraps)

Desktop E2E: `cd desktop && pnpm exec playwright test`

See [TESTING.md](TESTING.md) for the full multi-agent E2E guide.

### Desktop Screenshots (Playwright)

> **Do NOT use `buzz upload`, the relay media endpoint, or any third-party
> image host for PR screenshots.** Relay media URLs fail through GitHub's camo
> proxy. Always use `scripts/post-screenshots.sh` — see the `desktop-screenshot`
> skill for the full workflow.

The desktop app requires the E2E mock bridge to render — it cannot run in a plain
browser. Use `just desktop-screenshot` to capture screenshots (builds frontend,
starts preview server, runs Playwright automatically):

```bash
just desktop-screenshot --name home
just desktop-screenshot --name channel --route /channels/general
just desktop-screenshot --name search --click open-search
just desktop-screenshot --name settings --click open-settings
```

Options: `--name` (filename), `--route` (client route), `--active-channel`
(channel to view), `--click` (left-click data-testid or CSS selector),
`--right-click` (right-click for context menus), `--hover` (hover before
capture), `--clip` (crop region as `x,y,w,h` — e.g. `0,0,256,720` for sidebar
only), `--wait` (ms, default 2000), `--viewport` (WxH, default 1280x720),
`--outdir` (default `test-results/screenshots`), `--messages` (JSON file path).
Output is a PNG path on stdout.

Use `--messages` to inject content into a channel before capture. The JSON file
is an array of objects — `channelName` and `content` are required, all other
fields are optional and passed through to `__BUZZ_E2E_EMIT_MOCK_MESSAGE__`:

```json
[
  {
    "channelName": "random",
    "content": "Hey @tyler check this out",
    "pubkey": "953d...",
    "kind": 40002,
    "mentionPubkeys": ["deadbeef..."],
    "extraTags": [["broadcast", "1"], ["e", "some-root-id"]],
    "parentEventId": "abc123"
  }
]
```

Without `--active-channel`, all messages must target the same channel and the
helper navigates to that channel (useful for showing message content). With
`--active-channel`, messages can target multiple channels while the "camera"
stays on the specified channel (useful for unread indicators, badges, etc.).

```bash
# Messages in the channel you're viewing (code blocks, formatting, etc.)
just desktop-screenshot --name code-blocks --messages /tmp/msgs.json

# Messages in OTHER channels to trigger unread state
just desktop-screenshot --name unread-dot \
  --active-channel general --messages /tmp/badge-msgs.json

# Cropped to sidebar only (256px wide)
just desktop-screenshot --name sidebar-unread \
  --active-channel general --messages /tmp/badge-msgs.json \
  --clip 0,0,256,720

# Context menu on an unread channel (wider crop to include popup)
just desktop-screenshot --name ctx-mark-read \
  --active-channel general --messages /tmp/badge-msgs.json \
  --right-click channel-random --clip 0,200,320,300

# Hover state (e.g. copy button reveal)
just desktop-screenshot --name copy-hover \
  --messages /tmp/code-msgs.json --hover "[data-testid='copy-code']"
```

Available mock channels: `general`, `random`, `design`, `sales`, `engineering`,
`agents`, `watercooler`, `announcements`, `alice-tyler`, `bob-tyler`.

`scripts/post-screenshots.sh` hosts PNGs on a per-developer branch
(`agent-screenshots/<github-username>`) and posts a PR comment with
commit-SHA-based image URLs (immutable — safe from later overwrites):

```bash
./scripts/post-screenshots.sh 803 test-results/screenshots
./scripts/post-screenshots.sh 803 test-results/screenshots body.md  # custom body prepended
```

The body file supports `{{filename}}` placeholders (without `.png`) to inline
images at specific positions. Images not referenced by any placeholder are
appended at the end. Without placeholders, all images are appended (backward
compatible).

```markdown
### Unread dot
A message arrives in `#random`.

{{01-unread-dot}}

### Context menu
Right-click shows "Mark as read".

{{02-context-menu}}
```

Re-runs overwrite the image blobs on the `agent-screenshots/<username>`
branch, but the script **appends a new PR comment** — it does not edit or
delete the previous one. After reposting, delete the superseded comment so
only the current set remains, otherwise reviewers still see the stale images:

```bash
# List screenshot comments to find the stale one's id
gh pr view <pr> --repo block/sprout --json comments \
  --jq '.comments[] | select(.body | test("pr-<pr>--")) | {id, url}'
gh api -X DELETE repos/block/sprout/issues/comments/<stale-comment-id>
```

Branch cleanup when fully done: `git push origin --delete agent-screenshots/<username>`.

### Writing E2E Screenshot Specs

When screenshots need seeded state, live messages, or UI interaction before
capture, write a Playwright spec instead of using `just desktop-screenshot`.
Add specs to `desktop/tests/e2e/` and register them in `playwright.config.ts`
(`smoke` project `testMatch`). Every test calls `installMockBridge(page)` for
mock Tauri IPC. Mock pubkey, channel names, and UUIDs live in `e2eBridge.ts`.

**Stale server:** `reuseExistingServer: true` means a previous build's server
serves old code. Kill port 4173 and `pnpm run build` before re-running tests
after code changes.

**`addInitScript` before bridge:** `page.addInitScript` (localStorage seeding)
must run BEFORE `installMockBridge(page)` — React reads state on mount, the
bridge triggers mount.

**Live messages:** Call `waitForMockLiveSubscription(page, channelName)` before
`__BUZZ_E2E_EMIT_MOCK_MESSAGE__` — messages are silently dropped without a
subscription. Navigate to the channel first (triggers subscription), then away
(so unread indicators appear), then inject.

**Animation timing:** Radix components animate in via CSS. `toBeVisible()`
resolves mid-animation — wait for completion before screenshotting. Use the
shared helper (mandatory before any `page.screenshot()` or
`locator.screenshot()` in specs):

```ts
import { waitForAnimations } from "../helpers/animations";

// ... after the element is visible but before capturing:
await waitForAnimations(page);
await page.screenshot({ path: "...", clip: { ... } });
```

The `just desktop-screenshot` path (`screenshot.mjs`) calls
`waitForAnimations` automatically — no manual step needed there.

For per-element waits (rare — prefer the page-level helper above):

```ts
await menuItem.evaluate((el) =>
  Promise.all(
    el.closest("[data-state]")?.getAnimations().map((a) => a.finished) ?? [],
  ),
);
```

**Cropping:** Use `clip` — full-window (1280x720) screenshots are unreadable
for sidebar features. Sidebar = 256px; context menus ~450px.

**Distinct states — verify before posting:** when one view renders many
elements at once (e.g. all team cards in a single grid), an unscoped
full-page `page.screenshot()` captures the *same* pixels for every shot, so
multiple PNGs come out byte-identical. Scope each shot to its subject with
`locator.screenshot()` (full-page `clip` only when an overlay like an open
dropdown must be included). Then gate on hash distinctness before posting:

```bash
shasum -a 256 test-results/<dir>/*.png   # every hash must be unique
```

Identical hashes mean two shots captured the same state — fix the spec, do
not post. This catches the most common screenshot regression.

**`general` has pre-seeded messages** making `hasUnread` always true. Use
`engineering` for "muted + no unread" visual states.

**PR comments:** Use a body template (3rd arg to `post-screenshots.sh`) with
`{{filename}}` placeholders. Each screenshot gets a `###` heading + one-line
description. See [PR #803](https://github.com/block/sprout/pull/803).

---

## Common Gotchas

1. **Kind `39000` for channel metadata, not `41`** — kind 41 is NIP-01 (unused). All kinds defined in `buzz-core/src/kind.rs`.
2. **Relay queries must specify `kinds`** — omitting `kinds` triggers the p-gate (403). Always include explicit kind filters.
3. **`messages search` must include `--kinds`** — an open-ended search (no kinds) hits the relay p-gate and returns 403. Pass at least `--kinds 9,45001,45003` to scope the query.
4. **Worktrees: `cd` in the same command** — shell CWD doesn't persist between tool calls. Use `cd /path && cargo build` as one command.
5. **Desktop crate excluded from root workspace** — `cargo test` at repo root does NOT run desktop tests. Use `cargo test --manifest-path desktop/src-tauri/Cargo.toml` explicitly.
6. **Desktop Tauri fmt fails in worktrees and blocks commits** — the pre-commit hook runs `just desktop-tauri-fmt`, which fails in git worktrees because `cargo fmt` resolves workspace paths relative to the worktree root. Run `just desktop-tauri-fmt` from the main checkout to apply the fix, then re-stage and commit. CI is unaffected.

---

## Desktop App

The desktop app is Tauri 2 + React 19 + Vite + Tailwind CSS. Features are
organized under `desktop/src/features/`. Biome handles linting and formatting.

```bash
just desktop-dev   # web-only dev server (faster iteration)
just dev           # full Tauri app with native shell
```

### Text sizing & zoom (use rem, never px)

The desktop app implements Cmd +/- zoom by scaling the root `<html>`
font-size (`desktop/src/app/useWebviewZoomShortcuts.ts`) and pinning the native
webview zoom. **Only rem-based text scales with zoom — hardcoded px text sizes
are frozen.**

So for any readable text, reach for rem-based Tailwind tokens, never arbitrary
px:

- ✅ Stock rem tokens (`text-base`, `text-sm`, `text-xs`, …). **Chat body/author
  text === `text-base` (16px) — chat is the app's base type size**, and the
  surrounding timeline elements (timestamps, system rows, code, reactions) are
  deliberate steps on that same stock ramp.
- ✅ The `text-2xs` (0.6875rem / 11px) and `text-3xs` (0.5rem / 8px) meta-text
  tokens (in `desktop/tailwind.config.js` under `theme.extend.fontSize`) for the
  sub-`text-xs` ramp — timestamps, count badges, tracking labels, tiny glyphs.
  These replaced the dozens of arbitrary `text-[…rem]` literals that had drifted
  apart pixel-by-pixel; keep meta text on these two tokens, not new arbitrary
  values.
- ❌ `text-[15px]`, `text-[13px]`, CSS `font-size: 15px` — px froze against zoom
  and caused the message-timeline regression (PR #891).
- ❌ Arbitrary rem literals too: `text-[0.6875rem]`, `text-[0.9rem]`, etc. They
  zoom fine but re-fragment the scale we consolidated. Use a named token.

Prefer stock tokens — they're rem and zoom-safe. Only if a design genuinely
needs a size the stock/`2xs`/`3xs` scale can't express should you **add a
rem-based token** (in `desktop/tailwind.config.js` under `theme.extend.fontSize`)
rather than an arbitrary literal. A CI guard (`pnpm check:px-text`, in
`desktop/scripts/check-px-text.mjs`) scans all of `desktop/src` and fails on any
new arbitrary text-size literal — px **or** rem/em. Genuinely decorative glyphs
(e.g. the `text-[6rem]` avatar emoji) are allowlisted by `path:line` in that
script.

### Workspace Switching

The desktop app supports multiple workspaces (each backed by a different relay).
Switching workspaces does **not** reload the page — it uses React key-based
remounting. `<AppReady key={workspaceKey} />` in `App.tsx` forces the entire
workspace-scoped subtree to unmount and remount with fresh state.

**Module-level singletons must be explicitly reset.** React remounting only
clears React state (useState, useRef, context). Module-level variables (Maps,
class instances, cached promises) survive across remounts. Every workspace-scoped
singleton needs a reset function wired into `resetWorkspaceState()` in
`desktop/src/features/workspaces/useWorkspaceInit.ts`.

Current singletons that are reset on workspace switch:
- `relayClient.disconnect()` — WebSocket teardown + promise rejection
- `resetMediaCaches()` — proxy port and relay origin caches
- `clearSearchHitEventCache()` — search result event cache
- `clearAllDrafts()` — message draft cache

**If you add a new module-level cache, Map, or class instance that holds
workspace-scoped data, you must add its reset to `resetWorkspaceState()`.**
Failure to do so causes data from the old workspace to leak into the new one.

Key files:
- `desktop/src/app/App.tsx` — workspace key, init gate, remount boundary
- `desktop/src/features/workspaces/useWorkspaceInit.ts` — `resetWorkspaceState()`, applies config to Tauri backend
- `desktop/src/features/workspaces/useWorkspaces.tsx` — `WorkspacesProvider` context (shared state for App + AppShell)
- `desktop/src/main.tsx` — provider hierarchy (`QueryClientProvider` > `WorkspacesProvider` > `App`)

---

## Mobile App (Flutter)

The mobile app lives in `mobile/` — a Flutter app using Riverpod + Hooks.

### Architecture

- **State management:** Riverpod + `flutter_hooks` (`HookConsumerWidget`)
- **Theme:** Catppuccin Latte (light) / Macchiato (dark) — matches desktop
- **Features:** Isolated under `lib/features/`, shared code in `lib/shared/`
- **Nostr models:** `lib/shared/relay/nostr_models.dart` — event kinds must
  stay in sync with `desktop/src/shared/constants/kinds.ts`

### Rules

- **NEVER use `StatefulWidget`** — always use `HookConsumerWidget` or
  `ConsumerWidget` with `flutter_hooks` for local state.
- **NEVER run `flutter run`, `flutter build`, `flutter clean`, or
  `flutter upgrade`** — only `flutter test`, `flutter analyze`, and
  `dart format` are safe for agents to run.
- **Do NOT use `print()`** — use `debugPrint()` or structured logging.
- Prefer `context.colors` and `context.textTheme` (via theme extensions)
  over raw `Theme.of(context)` calls.
- Keep widgets small and composable.
- Feature modules must not import from other feature modules — only from
  `shared/`.
- Use `Grid` tokens for spacing, `Radii` for border radius.

### Quality Checks

```bash
cd mobile
dart format --output=none --set-exit-if-changed .
flutter analyze
flutter test
```

Or from repo root: `just mobile-fmt` (auto-fix), `just mobile-check` (lint + fmt check), `just mobile-test` (tests).

To run the app locally (starts Docker, relay, iOS simulator automatically):

```bash
just mobile-dev
```

### Testing Conventions

- Prefer **widget tests** over unit tests for UI components — test the
  whole widget tree, not individual methods.
- Use `ProviderScope(overrides: [...])` to inject fake notifiers.
- Fake notifiers should extend the real notifier class and override `build()`.
- Use the `WidgetHelpers.testable()` wrapper for simple widget tests or
  build a custom `ProviderScope` + `MaterialApp` when you need specific overrides.

---

## See Also

- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, code style, PR process, how to add event kinds / CLI subcommands / API endpoints
- [TESTING.md](TESTING.md) — multi-agent E2E test guide
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design and component relationships
- [RELEASING.md](RELEASING.md) — release process: `release-desktop`, `release-relay`, `release-mobile`, auto-tag, internal builds
- [README.md](README.md) — project overview and quick start
