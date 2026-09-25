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

## Before opening a PR

Open work in progress as a draft PR. Mark it ready for review only when this
checklist holds for the current change. Scale it to what changed:
documentation-only changes need content, link, and diff checks plus human
confirmation, not app runs.

1. **Agent review ran** under [Reviewing](#reviewing), and its recommended
   blockers were fixed or explicitly declined by the human author. Optional
   suggestions do not gate readiness.
2. **An agent exercised the changed behavior.** Client changes: the affected flow
   in the app, using the native app or a device when the behavior needs it
   (browser or headless Playwright counts only for what it can exercise). Relay
   changes: a local relay, exercising the changed events or endpoints. CLI or
   tooling changes: the affected command or workflow.
3. **A human then tested it themselves**: in the app, against the local relay
   (through the app or `curl`), or by running the changed command. Agent testing
   does not substitute. Agents give the human exact steps and what working looks
   like, then wait for explicit confirmation. Never mark this step done yourself.
4. **Add `buzz-review-completed` to the PR description** once steps 1–3 hold. It
   attests the checklist, and automated reviewers may skip review because of it.
   If later edits change behavior, remove it and return the PR to draft until the
   affected steps are redone.

## Reviewing

- Before reviewing, read [VISION.md](VISION.md), the `VISION_*.md` docs for the affected surface, and the PR's stated goal and linked
  issue. Review the change against what it is trying to do.
- Check the change against the [Review-Proven Rules](#review-proven-rules): the defects reviewers here find most often.
- Judge minimalism, elegance, and correctness, aiming for 9/10 on each. A score
  below 9 names the concrete defect and the fix.
- Recommend blocking only for concrete correctness, security, or agreed-contract
  defects with a realistic failure scenario: state the defect, how it fails, and
  the fix. Label everything else (nits, wording, speculative hardening,
  out-of-scope improvements) as optional.
- Put all findings in the first review. Later reviews check prior blockers and
  defects the fixes introduced; reopen other areas only on new evidence of a
  material defect.
- Agents post reviews as comments, never Request Changes. Humans decide which
  findings must be fixed.

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
For changes limited to Flutter/Dart code in the mobile app, run
`just mobile-install mobile-check mobile-test` instead of `just ci`.
Native code and build-configuration changes also require the corresponding
platform checks.

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
with on sight. Authors apply them **before writing code**, and reviewers check
against them; each cites the PRs where reviewers litigated it.

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

---

## Common Gotchas

1. **Kind `39000` for channel metadata, not `41`** — kind 41 is NIP-01 (unused). All kinds defined in `buzz-core/src/kind.rs`.
2. **Relay queries must specify `kinds`** — omitting `kinds` triggers the p-gate (403). Always include explicit kind filters.
3. **`messages search` chooses its own supported kinds** — do not add a `--kinds` option; the current command does not accept one. This differs from raw relay filters, which still need explicit kinds.
4. **Worktrees: `cd` in the same command** — shell CWD doesn't persist between tool calls. Use `cd /path && cargo build` as one command.
5. **Desktop crate excluded from root workspace** — `cargo test` at repo root does NOT run desktop tests. Use `cargo test --manifest-path desktop/src-tauri/Cargo.toml` explicitly.
6. **`pgschema` omits seed DML and some storage parameters** — Fresh desired-state bootstraps use `./bin/pgschema apply`, which does not execute `INSERT` statements or preserve every table storage parameter from `schema/schema.sql`. Put each unsupported invariant in `scripts/reconcile-schema-after-pgschema.sql` as an idempotent convergence statement plus a live catalog or data assertion. Every `pgschema apply` caller must run that script. A string assertion against `schema.sql` alone does not prove the pgschema-created database has the intended state.

---

## Desktop App

The desktop app is Tauri 2 + React 19 + Vite + Tailwind CSS. Features are
organized under `desktop/src/features/`. Biome handles linting and formatting.

```bash
just desktop-dev   # web-only dev server (faster iteration)
just dev           # full Tauri app with native shell
```

---

## Surface guides

Surface-specific guidance lives beside the code. The root guide still applies.

- Before changing `desktop/`, including PR screenshots and E2E screenshot specs, read [desktop/AGENTS.md](desktop/AGENTS.md).
- Before changing `desktop/src/features/agents/`, also read [desktop/src/features/agents/AGENTS.md](desktop/src/features/agents/AGENTS.md).
- Before changing `mobile/`, read [mobile/AGENTS.md](mobile/AGENTS.md).
- Before posting PR screenshots for any surface, including mobile, read [PR Screenshots](desktop/AGENTS.md#pr-screenshots).

---

## See Also

- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, code style, PR process, how to add event kinds / CLI subcommands / HTTP endpoints
- [TESTING.md](TESTING.md) — multi-agent E2E test guide
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design and component relationships
- [RELEASING.md](RELEASING.md) — release process: `release-desktop`, `release-relay`, `scripts/mobile-release.sh`, candidate tags, internal builds
- [README.md](README.md) — project overview and quick start
