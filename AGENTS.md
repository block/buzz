# AGENTS.md — AI Agent Contributor Guide

This guide is for AI agents contributing to the Buzz codebase. It covers
agent-specific context and conventions. For general contributor info (setup,
code style, PR process, architecture), see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Product Contract

Before planning or reviewing a non-trivial change:

1. Read [VISION.md](VISION.md).
2. Read the `VISION_*.md` documents relevant to the affected product surface.
3. Read the applicable guidance in [TESTING.md](TESTING.md) and any
   package-local `TESTING.md`.
4. Check that the proposed design advances, or at least does not contradict,
   that product intent. Call out any intentional tension explicitly.

Implementation describes the product today; the vision documents describe the
product it is becoming. A locally correct change can still be wrong if it works
against that direction. Scale validation to the change's risk and exercise the
real workflow for user-visible or integration behavior when practical; green CI
and runtime evidence answer different questions.

---

## Ecosystem

Buzz spans five repos. This one (`block/buzz`) is the OSS source for the relay, desktop, mobile, and CLI. The others handle internal builds and deployment:

| Repo | Purpose |
|------|---------|
| [block/buzz](https://github.com/block/buzz) | OSS source — relay, desktop app, mobile app, CLI, agent harness |
| [squareup/buzz-releases](https://github.com/squareup/buzz-releases) | Buildkite pipelines producing Block-signed macOS + iOS builds with `-block` desktop version suffix |
| [squareup/sprout-oss](https://github.com/squareup/sprout-oss) | CI pipeline building the relay Docker image and pushing to internal ECR |
| [squareup/block-coder-tf-stacks](https://github.com/squareup/block-coder-tf-stacks) | Terraform + ArgoCD deploying the relay to the staging Kubernetes cluster |
| [squareup/sprout-backend-blox](https://github.com/squareup/sprout-backend-blox) | Desktop backend provider script connecting Blox workstation agents to the relay |

```
block/buzz (source)
  ├─► buzz-releases      (desktop + mobile builds → Artifactory, GitHub, Mobile Releases)
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
  buzz-search         # Postgres FTS full-text search
  buzz-audit          # Hash-chain audit log
  buzz-media          # Blossom/S3 media storage
  # Agent surface
  buzz-acp            # ACP harness bridging Buzz events to AI agents
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

Run `just ci` before every PR — it runs repository-wide formatting, lint,
and static checks; Rust, Tauri, desktop, and mobile tests; and desktop and web
builds. Clippy passing does not mean fmt passes; run both.

Run `just test` for integration tests if you touched `buzz-relay`,
`buzz-db`, or `buzz-auth` — these require a running Postgres and Redis.

**Pre-commit hooks** are installed automatically by `just setup` and auto-fix
formatting via `stage_fixed`. Pre-commit runs fix variants in parallel (Rust
fmt, Tauri Rust fmt, desktop biome fix, web biome fix, mobile dart format).
Auto-fixable issues are fixed and re-staged; unfixable lint issues block the
commit. **Pre-push hooks** run the repository-wide differential file-size gate,
clippy (workspace + Tauri), desktop TypeScript typechecking (`tsc --noEmit`),
and fast unit tests in parallel (Rust, desktop JS, Tauri Rust, mobile Flutter)
— no overlap with pre-commit. Builds are CI-only. Run `just fix-all` to auto-fix
all formatting in one shot. Run `just ci` for the full local gate. Run `just
hooks` to re-install hooks after env changes. Each globbed pre-push lane is
scoped to the branch's merge-base diff against `origin/main` (`git diff
origin/main...HEAD`), matching CI's paths-filter — so a lane only fires when this
branch actually changed a file it covers, never because `origin/main` moved.
These lanes validate the checked-out HEAD; pushing a non-HEAD ref (explicit
refspec, `--all`) gets a non-fatal `push-head-scope` warning and relies on CI for
its path-scoped checks.
Before agents run Git or hooks, activate the repo's Hermit environment
(`. ./bin/activate-hermit`) so `./bin` leads `PATH` and the pinned toolchain
(flutter, dart, lefthook) wins over any Homebrew version; do not
rewrite hook commands to compensate for an unconfigured shell `PATH`. The
pre-push hook self-pins regardless: `bin/.lefthookrc` (sourced by the generated
`.git/hooks/*`) prepends the Hermit `bin/` to `PATH` and pins `LEFTHOOK_BIN`, so
lane subprocesses resolve the pinned flutter/dart/lefthook even when an
unactivated shell has Homebrew first. Activating Hermit remains recommended for
non-hook commands.

**Commit with `git commit -s`.** The required **DCO Check** fails any PR with a commit missing a `Signed-off-by` trailer, and `just hooks` installs a `commit-msg` hook that adds it to commits you create locally (`git rebase` and `git cherry-pick` still need `--signoff`) — if you build commit commands programmatically, include `-s` every time. To repair a branch that already has unsigned commits: `git rebase --signoff main`, then force-push.

Additional rules:
- No `unsafe` code
- Do not introduce new `unwrap()` or `expect()` in production paths — use `?` and proper error types
- New public API must have doc comments

---

## Review-Proven Rules

These rules distill the recurring findings from the last 25 PRs' review
threads — 53% of substantive review findings were repeats of the clusters
below, and reviewed PRs averaged ~5 review rounds. A second, independent
mining pass over 71 agent-review rooms (303 findings, Aug 18–29) confirmed
the same clusters and measured how often authors actually fix each class
once flagged: test-seam binding and unbounded-resource findings were fixed
**100%** of the time, swallowed-error findings **90%**, stale-state races
**70%** — these are not style opinions, they are defects authors agree
with on sight. Apply the rules **before writing code**; each cites the
PRs where reviewers litigated it.

1. **Every caught failure must leave a durable retry record or propagate.**
   Never catch-log-and-return-success (opt-out revocation permanently
   abandoned, PR #6269), never convert a terminal failure into an
   authoritative success/empty result (cold-history `error` → `success`
   with `[]`, PR #7013), and never delete the durable journal an operation
   depends on before its retry has actually succeeded (PR #6269). If a
   partial failure can orphan committed state (installations, endpoints),
   schedule its cleanup/renewal durably (PRs #6269, #6996, #7013).

2. **Fence async results by generation; clear derived metadata on every
   removal path.** A completing in-flight probe or fetch must verify it is
   still the newest before writing its result (stale login-shell probe
   recached a false-negative PATH, PR #6904). Provenance/ownership metadata
   attached to synthetic state must be updated or cleared on *all* paths
   that remove or refresh that state — typed deletion, toolbar removal,
   profile/name refresh; enumerate the paths and test each (PR #6956 burned
   4 rounds on this one class). Backfill and live subscriptions must
   overlap — a gap between a finite history REQ and the live subscription
   silently drops events (PR #3995); a retired chunk must not keep a stale
   scope fence (PR #6996). (PRs #3995, #6904, #6956, #6996)

3. **Regression tests must bind the production seam and be falsifiable.**
   See "Review-Proven Test Standards" in [TESTING.md](TESTING.md) for the
   full rule — in short: a guard whose removal doesn't fail any test
   protects nothing; bind regression tests to the production code path,
   not test-only helpers. (PRs #6807, #6980, #6996, #7013)

4. **Bound every resource, loop, and process tree.** Cap captured
   output (unbounded discovery temp files exhausted disk and overran the
   deadline, PR #6904). Containment failures are errors, not warnings — a
   tolerated Job Object creation failure or a `setsid` escape leaks whole
   process trees (PR #6904). Retry/re-subscribe loops need backoff and a
   terminal state: a persistent failure must not self-amplify into an
   unbounded refresh loop (PR #6996), and check zero-delay edge cases
   (`remainingMs()==0` selected the wrong fallback window, PR #6996).
   (PRs #6904, #6996)

5. **One user action = one atomic persist.** Implementing a single user
   commit as N independent durable writes leaves torn state on partial
   failure (theme "Set" as three independent notifier persists, PR #6944;
   relay-commit vs. local-save recovery gap, PR #6269). Persist one
   snapshot, or order the writes so every prefix is consistent and the
   remainder is durably retried per rule 1. (PRs #6269, #6944)

6. **A guard that hides the only recovery affordance is a functional
   failure.** Before adding a visibility predicate or state fence, ask:
   if the state it assumes goes wrong, does the user still have a way
   back? A fence that permanently suppresses "jump to latest" after a
   bounded correction fails strands the user silently — two reviewers
   flagged this independently (PR #6807).

7. **Audit assistive semantics on every new visual component.** The
   agent-review lanes flagged accessibility defects on 44 findings across
   the Aug 18–29 window — the second-largest cluster — and authors fixed
   the concrete ones (duplicate VoiceOver stops on native controls,
   actionable labels owned by two widgets at once, PR #6680; missing or
   decorative-leaking semantics on new UI, PRs #6611, #6702, #6885, #6905,
   #6908). New UI ships with: one owner per actionable label, no duplicate
   screen-reader stops, and explicit semantics for every interactive
   element. (PRs #6611, #6680, #6702, #6885, #6905, #6908, #6980)

8. **Every input modality is a first-class seam.** Keyboard, pointer, and
   hotkey paths must not silently diverge: `Shift+Space` treated as plain
   `Space` because the guard omitted `shiftKey` (PR #6862), keyboard
   ownership not released on blur, modifier keys dropped on the non-mouse
   path (PRs #5958, #6793, #6860, #6908, #7006). When adding an input
   handler, enumerate the modalities that can reach it and test the
   non-primary ones — that's where the defects were. (PRs #5958, #5972,
   #6793, #6860, #6862, #6908, #7006)

---

## Key Patterns

**Nostr-first HTTP surface**: Buzz's primary API is NIP-29 over WebSocket. The relay also exposes a narrow HTTP surface: NIP-11/NIP-05 metadata, `POST /events`, `POST /query`, `POST /count`, workflow webhooks at `/hooks/{id}`, Blossom media, git smart HTTP, git policy hooks, and health probes. These HTTP paths all preserve the same host-derived community boundary.

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
thread. Pass the link directly to the CLI:

```bash
buzz --format compact messages thread --link '<buzz://message?...>'
```

The selected message ID is authoritative: `messages thread` verifies its
channel and derives its containing root. An optional `thread` parameter is
accepted only when it matches that derived root. The explicit
`--channel <uuid> --event <hex>` form remains available.

All event reads return normalized JSON arrays. Normal output preserves the seven
canonical signed Nostr event fields (`id`, `pubkey`, `kind`, `content`,
`created_at`, `tags`, `sig`); all writes return
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
- `e2e_media.rs` — media upload/download (Blossom)
- `e2e_media_extended.rs` — extended media scenarios
- `e2e_nostr_interop.rs` — Nostr interop (NIP-50 search, NIP-10 threads, NIP-17 gift wraps)

Desktop E2E: `cd desktop && pnpm test:e2e:smoke` for mock-bridge smoke
coverage, or `pnpm test:e2e:integration` for relay-backed coverage. These
scripts build the required E2E bridge before running Playwright.

See [TESTING.md](TESTING.md) for the full multi-agent E2E guide.

### PR Screenshots

> **Do NOT use `buzz upload`, the relay media endpoint, or any third-party
> image host for PR screenshots.** Relay media URLs fail through GitHub's camo
> proxy. Always use `scripts/post-screenshots.sh` for PNGs before linking them
> from a PR body/comment. If you hand-edit PR markdown, run
> `scripts/check-pr-image-urls.sh <markdown-file>` first to catch relay URLs.

For mobile simulator screenshots, save the PNGs in a local directory and run
`./scripts/post-screenshots.sh <PR-number> <png-dir>` or use the third argument
with a markdown template containing `{{filename}}` placeholders.

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
gh pr view <pr> --repo block/buzz --json comments \
  --jq '.comments[] | select(.body | test("pr-<pr>--")) | {id, url}'
gh api -X DELETE repos/block/buzz/issues/comments/<stale-comment-id>
```

Branch cleanup when fully done: `git push origin --delete agent-screenshots/<username>`.

### Writing E2E Screenshot Specs

When screenshots need seeded state, live messages, or UI interaction before
capture, write a Playwright spec instead of using `just desktop-screenshot`.
Add specs to `desktop/tests/e2e/` and register them in `playwright.config.ts`
(`smoke` project `testMatch`). Every test calls `installMockBridge(page)` for
mock Tauri IPC. Mock pubkey, channel names, and UUIDs live in `e2eBridge.ts`.

**Always build with `pnpm build:e2e`, never `pnpm run build`.** The mock Tauri
bridge is compiled in only for `--mode e2e` (see `installE2eBridgeIfConfigured`
in `desktop/src/main.tsx`). A plain `pnpm run build` strips it, so
`window.__TAURI_INTERNALS__` is never defined and **every** mock-mode spec fails
with `Cannot read properties of undefined (reading 'invoke')` — the app renders
"Community connection failed" instead of the UI under test. That looks exactly
like a product bug rather than a build mistake, so it burns real time.
`pnpm test:e2e:smoke` and `pnpm test:e2e:integration` run the right build for
you; prefer them over a manual build plus `playwright test`.

**Stale server:** `reuseExistingServer: true` means a previous build's server
serves old code. Kill port 4173 and re-run `pnpm build:e2e` before re-running
tests after code changes.

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
description. See [PR #803](https://github.com/block/buzz/pull/803).

---

## Common Gotchas

1. **Kind `39000` for channel metadata, not `41`** — kind 41 is NIP-01 (unused). All kinds defined in `buzz-core/src/kind.rs`.
2. **Relay queries must specify `kinds`** — omitting `kinds` triggers the p-gate (403). Always include explicit kind filters.
3. **`messages search` chooses its own supported kinds** — do not add a `--kinds` option; the current command does not accept one. This differs from raw relay filters, which still need explicit kinds.
4. **Worktrees: `cd` in the same command** — shell CWD doesn't persist between tool calls. Use `cd /path && cargo build` as one command.
5. **Desktop crate excluded from root workspace** — `cargo test` at repo root does NOT run desktop tests. Use `cargo test --manifest-path desktop/src-tauri/Cargo.toml` explicitly.
6. **React render perf: `React.memo` is all-or-nothing** — it only skips a re-render when *every* prop is reference-stable; one unstable prop (inline arrow/JSX, or a hook returning a fresh `{}`/`[]`/`Map` each render) defeats it. Two repeat offenders: (a) React Query results (`useMutation`/`useQuery`) are a **new object each render** — depend on the stable method (`mutation.mutateAsync`), not the object; (b) derived `Map`/array state that recomputes on a version bump — wrap in a content-equality ref cache (`shared/hooks/useStableReference.ts`). When chasing interaction lag, **measure with DevTools closed and no perf probes** (an open Web Inspector + per-keystroke `console.log` inflate the numbers), and isolate by removing one suspect at a time rather than guessing.
7. **`pgschema` omits seed DML and some storage parameters** — Fresh desired-state bootstraps use `./bin/pgschema apply`, which does not execute `INSERT` statements or preserve every table storage parameter from `schema/schema.sql`. Put each unsupported invariant in `scripts/reconcile-schema-after-pgschema.sql` as an idempotent convergence statement plus a live catalog or data assertion. Every `pgschema apply` caller must run that script. A string assertion against `schema.sql` alone does not prove the pgschema-created database has the intended state.

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

- ✅ Stock rem tokens (`text-base`, `text-sm`, `text-xs`, …) for general
  interface text. All of these derive from the virtual typography rem and
  therefore follow the user's font-size preference and Cmd +/- zoom.
- ✅ Conversation text uses the named `text-message` token. Its
  **Smaller / Default / Larger contract is 13 / 14 / 15px** before keyboard
  zoom. Author names use the same conversation-size step; timestamps, system
  rows, code, and reactions are deliberate neighboring steps on the shared
  virtual-rem ramp. Keep those relationships tokenized rather than restoring a
  fixed 16px chat baseline or hardcoding preference-specific values in
  components.
- ✅ The `text-2xs` (0.6875rem / 11px at a 16px virtual rem) and `text-3xs`
  (0.5rem / 8px at a 16px virtual rem) meta-text
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

`resetCommunityState()` is the canonical inventory of community-scoped
singletons. **If you add a new module-level cache, Map, or class instance that
holds community-scoped data, add its reset there in the same change.** Failure
to do so causes data from the old community to leak into the new one. Avoid
duplicating its complete reset list here; the implementation is the source of
truth.

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
- Agents may build and run the Flutter app when it materially helps implement,
  debug, or validate mobile changes. Prefer the smallest relevant command and
  reuse an already-running simulator/emulator and the app's configured staging
  or production community when that is sufficient. Do not start or rebuild
  local relay services unless the task specifically requires relay-side or
  isolated integration behavior.
- For iOS runtime validation, prefer `just mobile-dev`; it applies the
  worktree-specific debug identity and runs `flutter run`. Direct `flutter run`
  or IDE workflows are also allowed. Use `just mobile-build-android` only when
  an APK build is relevant to the task.
- Do not rebuild, reinstall, or relaunch merely for ceremony. Preserve Flutter's
  incremental build cache and use hot reload/restart where appropriate. Use
  `flutter clean` only when stale build artifacts are a credible cause. Run
  `flutter upgrade` only when the task explicitly requires a toolchain change.
- For user-visible or integration changes, exercise the affected workflow in a
  real app when practical and report the device/simulator, connected community,
  and workflow actually tested.
- **Do NOT use `print()`** — use `debugPrint()` or structured logging.
- Prefer `context.colors` and `context.textTheme` (via theme extensions)
  over raw `Theme.of(context)` calls.
- **Keep widgets small and composable.** One public widget per file; push
  private sub-widgets (`_Foo`) into sibling `part` files under a
  `<page>/` folder rather than growing the page file. Mobile's hard ceiling is
  **1200 lines/file**, enforced with the other surface-specific limits by the
  repository-level `just file-size-check` gate (`just check`, CI, and every
  pre-push). If an individual file trips the guard, **split the file — never
  bump a surface limit or add an override merely to admit that file.**
  Deliberate repository-wide policy revisions must update the enforced rules,
  tests, and guidance together.
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

To run the app locally with a worktree-specific debug identity and a
started or reused iOS Simulator:

```bash
just mobile-dev
```

This runs `flutter run` against the app's configured community; it does not
start Docker or local relay services.

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
- [TESTING.md](TESTING.md) — multi-agent E2E test guide
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design and component relationships
- [RELEASING.md](RELEASING.md) — release process: `release-desktop`, `release-relay`, `scripts/mobile-release.sh`, candidate tags, internal builds
- [README.md](README.md) — project overview and quick start

### Mention editor contract

Autocomplete inserts a literal full label and a separator, including multi-word
names. Only autocomplete settlement may move the caret past that separator;
internal label spaces and deliberate ArrowLeft/click movement must be respected.
See `docs/mention-editor.md` and `desktop/tests/e2e/mention-spacing.spec.ts`.
