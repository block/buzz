# AGENTS.md — AI Agent Contributor Guide

This guide is for AI agents contributing to the Pkzz codebase. It covers
agent-specific context and conventions. For general contributor info (setup,
code style, PR process, architecture), see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Ecosystem

This repo (`kingkillery/pkzz`) is the OSS source for the relay, desktop, mobile, and CLI. The other four repos are internal build and deploy — not part of this tree:

| Repo | Purpose |
|------|---------|
| [squareup/buzz-releases](https://github.com/squareup/buzz-releases) | Block-signed macOS + iOS builds (`-block` desktop suffix) → Artifactory, GitHub, Mobile Releases |
| [squareup/sprout-oss](https://github.com/squareup/sprout-oss) | Relay Docker image → internal ECR |
| [squareup/block-coder-tf-stacks](https://github.com/squareup/block-coder-tf-stacks) | Helm chart from that image → ArgoCD → staging cluster |
| [squareup/sprout-backend-blox](https://github.com/squareup/sprout-backend-blox) | Blox compute provider for desktop agent launch |

Access: [CONTRIBUTING.md § Ecosystem](CONTRIBUTING.md#ecosystem). Release flow: [RELEASING.md](RELEASING.md).

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
  buzz-search         # Postgres FTS full-text search
  buzz-audit          # Hash-chain audit log
  buzz-media          # Blossom/S3 media storage
  # Agent surface
  buzz-acp            # ACP harness bridging Pkzz events to AI agents
  buzz-agent          # Minimal ACP-compliant agent (non-streaming, tool-calls-as-output)
  buzz-dev-mcp        # Developer MCP server — shell + file-edit tools
  buzz-persona        # Agent persona packs
  buzz-workflow       # YAML-as-code workflow engine (evalexpr conditions)
  # Clients + interop
  buzz-pair-relay     # Ephemeral sidecar relay for NIP-AB device pairing
  buzz-pairing-cli    # CLI for NIP-AB device pairing interop testing
  git-sign-nostr      # Sign git objects with a Nostr key
  git-credential-nostr # Git credential helper for Nostr-authed push/fetch
  # Tooling + shared
  buzz-cli            # Agent client surface — JSON in, JSON out
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

Run `just ci` before every PR — fmt, clippy, desktop lint, unit tests, and builds. Clippy passing does not mean fmt passes; run both. Run `just test` if you touched `buzz-relay`, `buzz-db`, or `buzz-auth` (needs Postgres and Redis).

Hook inventory is [lefthook.yml](lefthook.yml) (`just setup` installs it; `just hooks` reinstalls after env changes). Pre-commit auto-fixes formatting and re-stages it; unfixable lint blocks the commit. Pre-push runs clippy and fast unit tests only — no overlap with pre-commit. Builds are CI-only. `just fix-all` is the one-shot formatter.

Before Git or hooks, activate Hermit (`. ./bin/activate-hermit`). Do not rewrite hook commands to compensate for an unconfigured `PATH`.

**Commit with `git commit -s`.** The DCO check rejects a missing `Signed-off-by`. The `commit-msg` hook covers local `git commit` / `git merge` only — `git rebase` needs `--signoff`, `git cherry-pick` needs `-s`, and any commit command you build programmatically must include `-s`. Repair unsigned history with `git rebase --signoff main`, then force-push. Details: [CONTRIBUTING.md § Sign Your Commits](CONTRIBUTING.md#sign-your-commits).

Additional rules:
- No `unsafe` code
- Do not introduce new `unwrap()` or `expect()` in production paths — use `?` and proper error types
- New public API must have doc comments

---

## Key Patterns

**Nostr-first HTTP surface**: Pkzz's primary API is NIP-29 over WebSocket. The relay also exposes a narrow HTTP surface: NIP-11/NIP-05 metadata, `POST /events`, `POST /query`, `POST /count`, workflow webhooks at `/hooks/{id}`, Blossom media, git smart HTTP, git policy hooks, and health probes. These HTTP paths all preserve the same host-derived community boundary.

**Prefer Nostr events over new HTTP endpoints**: For new feature work, model
the operation as a Nostr event (new kind in `buzz-core/src/kind.rs`, handler
in `buzz-relay`) rather than adding endpoint-specific JSON APIs. HTTP is
reserved for things that genuinely need an HTTP-only surface: media upload/download
(Blossom), webhooks, git smart HTTP, NIP-11/NIP-05 metadata, health checks,
and the generic Nostr bridge endpoints:

- `POST /events` — submit any signed event (same path the WebSocket uses).
- `POST /query` — Nostr REQ filters over HTTP. NIP-50 `search` filters
  are routed to `buzz-search` (Postgres FTS) automatically.
- `POST /count` — Nostr COUNT filters over HTTP.

If you find yourself reaching for a new HTTP endpoint, first check whether
an event kind would do the job — it usually will, and you get realtime
fan-out, NIP-29 scoping, and the existing auth pipeline for free.

Reference https://github.com/nostr-protocol/nips

**Event kinds**: All event kind integers are defined in
`buzz-core/src/kind.rs`. New features get new kind integers — add them here
first, then implement handling in the relay.

**Channel scoping**: Channels use `h` tags (NIP-29 group tag), not `e` tags.
Filters and queries must scope to `h` tags when operating within a channel.
This applies to events *inside* a channel. Addressable events that describe a
channel carry its id in their `d` tag instead: kind:39000 (metadata),
kind:39001, kind:39002 (membership). `get_channels` resolves a user's channels
from the `d` tag of their kind:39002 events, not from `h`.

**CLI is the client surface**: New agent-facing operations belong in `buzz-cli` — a client of the relay, not a new HTTP endpoint and not `buzz-dev-mcp` (shell + file tools for `buzz-agent`). Add a subcommand there first, then wire the call in `client.rs`. See [CLI client surface](#cli-client-surface-buzz-cli).

**Workflow conditions**: `buzz-workflow` uses
[evalexpr](https://docs.rs/evalexpr) for condition evaluation. Keep expressions
simple and testable.

**Thread counters**: `reply_count` and `descendant_count` are materialized on
thread root events. Any code that inserts replies must update these counters —
check existing reply handlers for the pattern.

### Engagement discernment

[`docs/engagement-discernment-workbook.md`](docs/engagement-discernment-workbook.md) is the field workbook for when agents should speak, stay quiet, or escalate. It is an evidence instrument, not a design doc — use its setup ladder and §3–§5 templates, not this guide, for how to fill it in.

Hard constraints:

- Log a row only after real dogfood friction, while it is still traceable. Never invent entries from tests, hypotheticals, or memory. Reference events; do not copy secrets or sensitive conversation content.
- Lab channel only: `engagement = "thread"`. Leave every other channel on deterministic `mentions`. Change one knob at a time.
- **Do not implement `open` scheduling or the escalation kind until every §6 gate is checked** (≥2 weeks with ≥2 lab agents, ≥15 §3A–B entries, ≥5 §3D entries, named §5 failure shapes, one completed §4 knob cycle). At pickup, derive the first heuristic scheduler only from §5. Invariant: deterministic by default, experiments opt-in per channel, every new behavior emits a decision frame.

---

## CLI client surface (`buzz-cli`)

`buzz` is the agent **client** surface for the relay (JSON in, JSON out) — not a server API and not `buzz-dev-mcp`. New agent-facing client operations get a subcommand here first, then the relay call in `client.rs`. Command list: [`crates/buzz-cli/README.md`](crates/buzz-cli/README.md). Live runbook: [`crates/buzz-cli/TESTING.md`](crates/buzz-cli/TESTING.md).

Hard constraints:

- The ACP harness injects `BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, and `BUZZ_AUTH_TAG` into managed subprocesses. In development, set `BUZZ_PRIVATE_KEY` and `BUZZ_RELAY_URL` yourself. Release binary: `cargo build --release -p buzz-cli` → `./target/release/buzz`.
- `--format compact` is a **global** flag and goes before the subcommand: `buzz --format compact channels list`, not `buzz channels list --format compact`.
- Reads return sig-stripped JSON arrays; writes return `{event_id, accepted, message}`; creates add the entity ID. Exit codes: 0 ok, 1 input error, 2 network/relay, 3 auth, 4 other, 5 write conflict (NIP-33 LWW).
- `buzz://message?channel=<uuid>&id=<hex>` names a thread. Read it with `buzz --format compact messages thread --channel <uuid> --event <hex>`. Use `channel` and `id` from the query string. Ignore the optional `thread` parameter — `messages thread` resolves the thread from the event id alone.

---

## Testing

```bash
just test-unit    # unit tests, no infrastructure needed
just test         # full integration suite (requires Postgres + Redis)
```

E2E tests live in `crates/buzz-test-client/tests/`:

- `e2e_relay.rs` — WebSocket relay protocol
- `e2e_media.rs` — media upload/download (Blossom)
- `e2e_media_extended.rs` — extended media scenarios
- `e2e_nostr_interop.rs` — Nostr interop (NIP-50 search, NIP-10 threads, NIP-17 gift wraps)

Desktop E2E: `cd desktop && pnpm exec playwright test`

Live relay and ACP harness: [TESTING.md](TESTING.md). PR screenshots: [TESTING.md § PR Screenshots](TESTING.md#pr-screenshots). Do not use `buzz upload`, the relay media endpoint, or any third-party image host for PR screenshots — relay URLs fail through GitHub's camo proxy. Post PNGs with `scripts/post-screenshots.sh`.

---

## Common Gotchas

1. **Kind `39000` for channel metadata, not `41`** — kind 41 is NIP-01 (unused). All kinds defined in `buzz-core/src/kind.rs`.
2. **Relay queries must specify `kinds`** — omitting `kinds` triggers the p-gate (403). Always include explicit kind filters.
3. **`messages search` must include `--kinds`** — an open-ended search (no kinds) hits the relay p-gate and returns 403. Pass at least `--kinds 9,45001,45003` to scope the query.
4. **Worktrees: pin the directory on the command** — do not assume a previous working directory still applies. Run the command from the worktree you mean (`cd /path/to/worktree && cargo build`) or pass an explicit `--manifest-path` / working directory.
5. **Desktop crate excluded from root workspace** — `cargo test` at repo root does NOT run desktop tests. Use `cargo test --manifest-path desktop/src-tauri/Cargo.toml` explicitly.
6. **Desktop Tauri fmt fails in worktrees and blocks commits** — the pre-commit hook runs `just desktop-tauri-fmt`, which fails in git worktrees because `cargo fmt` resolves workspace paths relative to the worktree root. Run `just desktop-tauri-fmt` from the main checkout to apply the fix, then re-stage and commit. CI is unaffected.
7. **React render perf: `React.memo` is all-or-nothing** — it only skips a re-render when *every* prop is reference-stable; one unstable prop (inline arrow/JSX, or a hook returning a fresh `{}`/`[]`/`Map` each render) defeats it. Two repeat offenders: (a) React Query results (`useMutation`/`useQuery`) are a **new object each render** — depend on the stable method (`mutation.mutateAsync`), not the object; (b) derived `Map`/array state that recomputes on a version bump — wrap in a content-equality ref cache (`shared/hooks/useStableReference.ts`). When chasing interaction lag, **measure with DevTools closed and no perf probes** (an open Web Inspector + per-keystroke `console.log` inflate the numbers), and isolate by removing one suspect at a time rather than guessing.

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

### Community Switching

The desktop app supports multiple communities (each backed by a different relay).
Switching communities does **not** reload the page — it uses React key-based
remounting. `<AppReady key={communityKey} />` in `App.tsx` forces the entire
community-scoped subtree to unmount and remount with fresh state.

**Module-level singletons must be explicitly reset.** React remounting only
clears React state (useState, useRef, context). Module-level variables (Maps,
class instances, cached promises) survive across remounts. Every community-scoped
singleton needs a reset function wired into `resetCommunityState()` in
`desktop/src/features/communities/useCommunityInit.ts`.

Current singletons that are reset on relay boundary changes (same-relay
reconnects preserve pending avatar verification work):
- `relayClient.disconnect()` — WebSocket teardown + promise rejection
- `resetRateLimitGate()` — clears any active rate-limit window from the old relay
- `clearAllDrafts()` — message draft cache
- `resetAgentObserverStore()` — agent observer relay store
- `resetActiveAgentTurnsStore()` — active agent turn timers
- `resetAgentWorkingSignal()` — agent working indicator signal
- `resetAvatarProfileSync()` — pending verified-avatar profile writes
- `resetAvatarPresentations()` — avatar probes, previews, and Retry toasts
- `resetSidebarRelayConnectionCardState()` — sidebar relay card dismiss state
- `resetMediaCaches()` — proxy port and relay origin caches
- `resetVideoPlayerState()` — video player singleton
- `resetRenderScopedReactionHydration()` — reaction hydration cache
- `clearSearchHitEventCache()` — search result event cache
- `clearMarkdownNodeCache()` — markdown parse-node cache
- `resetLinkPreviewTitleCache()` — link preview title cache (Pkzz entity titles come from relay events)

**If you add a new module-level cache, Map, or class instance that holds
community-scoped data, you must add its reset to `resetCommunityState()`.**
Failure to do so causes data from the old community to leak into the new one.

Key files:
- `desktop/src/app/App.tsx` — community key, init gate, remount boundary
- `desktop/src/features/communities/useCommunityInit.ts` — `resetCommunityState()`, applies config to Tauri backend
- `desktop/src/main.tsx` — provider hierarchy (`QueryClientProvider` > `App`)

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

- **NEVER use `StatefulWidget`** — favor Riverpod for state and always use
  `HookConsumerWidget` or `ConsumerWidget` with `flutter_hooks` for local state.
- **NEVER run `flutter run`, `flutter build`, `flutter clean`, or
  `flutter upgrade`** — only `flutter test`, `flutter analyze`, and
  `dart format` are safe for agents to run.
- **Do NOT use `print()`** — use `debugPrint()` or structured logging.
- Prefer `context.colors` and `context.textTheme` (via theme extensions)
  over raw `Theme.of(context)` calls.
- **Keep widgets small and composable.** One public widget per file; push
  private sub-widgets (`_Foo`) into sibling `part` files under a
  `<page>/` folder rather than growing the page file. Hard ceiling:
  **1000 lines/file**, enforced by `mobile/scripts/check-file-sizes.mjs` via
  `just mobile-check` (runs in `just check` + pre-push, mirroring desktop/web).
  If the guard trips, **split the file — never bump the limit or add an
  override to slip under it.**
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

When run from a git worktree, `just mobile-dev` (and `just
mobile-build-android`) give the debug build a per-worktree app identifier
(keyed to the worktree directory name) and a branch-labelled app name via
`scripts/mobile-worktree-overrides.sh`, so builds from multiple worktrees
install side by side. Release builds are unaffected. `just mobile-clean`
removes stale worktree-suffixed installs from simulators/emulators. See
[mobile/README.md](mobile/README.md) for direct Xcode / Android Studio
usage.

### Testing Conventions

- Prefer **widget tests** over unit tests for UI components — test the
  whole widget tree, not individual methods.
- Use `ProviderScope(overrides: [...])` to inject fake notifiers.
- Fake notifiers should extend the real notifier class and override `build()`.
- Use the `WidgetHelpers.testable()` wrapper for simple widget tests or
  build a custom `ProviderScope` + `MaterialApp` when you need specific overrides.

---

## See Also

- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, code style, PR process, how to add event kinds / CLI subcommands / HTTP endpoints
- [TESTING.md](TESTING.md) — live relay, ACP harness, and PR screenshot runbooks
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design and component relationships
- [RELEASING.md](RELEASING.md) — release process: `release-desktop`, `release-relay`, `scripts/mobile-release.sh`, candidate tags, internal builds
- [README.md](README.md) — project overview and quick start
