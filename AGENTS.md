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

## Task-Specific Guidance

Load only the guidance relevant to the change:

- Before writing or reviewing production code, read
  [Review-Proven Rules](docs/agent-guides/review-proven-rules.md).
- Before changing `desktop/`, read [desktop/AGENTS.md](desktop/AGENTS.md) and
  its linked detailed guide.
- Before changing `mobile/`, read [mobile/AGENTS.md](mobile/AGENTS.md) and its
  linked detailed guide.
- For screenshots or visual evidence, read
  [PR Screenshots](docs/agent-guides/pr-screenshots.md).
- For general setup, architecture, and repository layout, use
  [CONTRIBUTING.md](CONTRIBUTING.md) and [ARCHITECTURE.md](ARCHITECTURE.md).

The scoped guides preserve detailed rules that previously lived in this root
file. Keep new guidance at the narrowest useful scope instead of growing this
always-loaded router.

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

## Common Gotchas

1. **Kind `39000` for channel metadata, not `41`** — kind 41 is NIP-01 (unused). All kinds defined in `buzz-core/src/kind.rs`.
2. **Relay queries must specify `kinds`** — omitting `kinds` triggers the p-gate (403). Always include explicit kind filters.
3. **`messages search` chooses its own supported kinds** — do not add a `--kinds` option; the current command does not accept one. This differs from raw relay filters, which still need explicit kinds.
4. **Worktrees: `cd` in the same command** — shell CWD doesn't persist between tool calls. Use `cd /path && cargo build` as one command.
5. **Desktop crate excluded from root workspace** — `cargo test` at repo root does NOT run desktop tests. Use `cargo test --manifest-path desktop/src-tauri/Cargo.toml` explicitly.
6. **React render perf: `React.memo` is all-or-nothing** — it only skips a re-render when *every* prop is reference-stable; one unstable prop (inline arrow/JSX, or a hook returning a fresh `{}`/`[]`/`Map` each render) defeats it. Two repeat offenders: (a) React Query results (`useMutation`/`useQuery`) are a **new object each render** — depend on the stable method (`mutation.mutateAsync`), not the object; (b) derived `Map`/array state that recomputes on a version bump — wrap in a content-equality ref cache (`shared/hooks/useStableReference.ts`). When chasing interaction lag, **measure with DevTools closed and no perf probes** (an open Web Inspector + per-keystroke `console.log` inflate the numbers), and isolate by removing one suspect at a time rather than guessing.
7. **`pgschema` omits seed DML and some storage parameters** — Fresh desired-state bootstraps use `./bin/pgschema apply`, which does not execute `INSERT` statements or preserve every table storage parameter from `schema/schema.sql`. Put each unsupported invariant in `scripts/reconcile-schema-after-pgschema.sql` as an idempotent convergence statement plus a live catalog or data assertion. Every `pgschema apply` caller must run that script. A string assertion against `schema.sql` alone does not prove the pgschema-created database has the intended state.

---

## See Also

- [CONTRIBUTING.md](CONTRIBUTING.md) — setup, code style, PR process, how to add event kinds / CLI subcommands / HTTP endpoints
- [TESTING.md](TESTING.md) — multi-agent E2E test guide
- [ARCHITECTURE.md](ARCHITECTURE.md) — system design and component relationships
- [RELEASING.md](RELEASING.md) — release process: `release-desktop`, `release-relay`, `scripts/mobile-release.sh`, candidate tags, internal builds
- [README.md](README.md) — project overview and quick start
