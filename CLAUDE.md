# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

For comprehensive contributor guidance, see [AGENTS.md](AGENTS.md). This file covers what's not already there — commands, architecture overview, and context an AI agent needs on first interaction.

## Quick Start

```bash
. ./bin/activate-hermit   # activate pinned Hermit toolchain
cp .env.example .env      # one-time config
just setup                # Docker + migrations + deps
just relay                # start relay at ws://localhost:3000
just dev                  # relay + desktop app together
just ci                   # full CI gate before PR
```

## Project Overview

Buzz is a self-hostable team workspace where humans and AI agents collaborate. It's built on the Nostr protocol — every action (chat, reaction, workflow, canvas, git event) is a signed Nostr event with a `kind` integer. The relay is the single source of truth.

- **Relay**: Axum WebSocket server — Nostr NIP-01, auth, event pipelines, workflows
- **Desktop**: Tauri 2 + React 19 + Tailwind CSS
- **Mobile**: Flutter + Riverpod
- **Web**: Plain JS browser client served by the relay
- **CLI**: `buzz-cli` — agent-first, JSON in/out
- **Agent harness**: `buzz-acp` (ACP/JSON-RPC bridge), `buzz-agent`, `buzz-dev-mcp`

## Common Commands

### Build

| Command | Purpose |
|---------|---------|
| `just build` | Build Rust workspace |
| `just build-release` | Release mode |
| `cargo build --release -p buzz-cli` | Build just the CLI binary (`./target/release/buzz`) |

### Lint & Format

| Command | Purpose |
|---------|---------|
| `just check` | fmt + clippy + desktop/web/mobile checks |
| `just fmt` | Format all Rust code |
| `just clippy` | Warnings-as-errors |
| `just fix-all` | Fix all formatting and lint |
| `just desktop-fix` | Fix desktop Biome issues |
| `just mobile-fmt` | Format Dart code |
| `just mobile-check` | Dart format check + flutter analyze + file size check |

### Test

| Command | Purpose | Infra? |
|---------|---------|--------|
| `just test-unit` | Unit tests (buzz-core, buzz-auth, buzz-db lib, buzz-conformance, buzz-push-gateway) | None |
| `just test` | Full unit + integration suite | Docker (Postgres + Redis) |
| `cargo test -p buzz-test-client -- --ignored` | E2E tests | Running relay |
| `just desktop-test` | Desktop TS unit tests | None |
| `just desktop-tauri-test` | Desktop Tauri Rust tests | None |
| `just desktop-e2e-smoke` | Desktop Playwright smoke tests | Build |
| `just mobile-test` | Flutter tests | None |
| `cargo test -p buzz-db --lib` | DB crate unit tests only (sql parsing, no Postgres) | None |
| `cargo nextest run -p buzz-conformance` | Multi-tenant conformance tests | None |

### Run

| Command | Purpose |
|---------|---------|
| `just relay` | Start relay (auto-starts Docker) |
| `just dev` | Relay + desktop app |
| `just desktop-dev` | Vite dev only (faster iteration) |
| `just desktop-standalone` | Desktop only, no relay/deps |
| `just relay-web` | Relay with built web UI |
| `just web` | Web dev server |
| `just mobile-dev` | iOS simulator (starts Docker + relay) |

### Desktop Screenshots

```bash
just desktop-screenshot --name channel --route /channels/general
just desktop-screenshot --name search --click open-search
just desktop-screenshot --name sidebar-unread --active-channel general --messages /tmp/msgs.json --clip 0,0,256,720
```

### Release

| Command | Purpose |
|---------|---------|
| `just release-desktop` | Open/update desktop release PR |
| `just release-relay` | Open/update relay release PR |
| `scripts/mobile-release.sh candidate X.Y.Z` | Publish mobile candidate tag |

### Database

| Command | Purpose |
|---------|---------|
| `just migrate` | Apply pending migrations |
| `just down` | Stop Docker, keep data |
| `just reset` | ⚠️ Wipe all dev data |
| `just ps` | Show Docker service status |
| `just logs` | Tail Docker logs |

### Agents

```bash
cargo build --release -p buzz-acp       # ACP harness
cargo build --release -p buzz-agent     # Minimal ACP agent
cargo build --release -p buzz-cli       # Agent-first CLI
just goose relay=ws://localhost:3000 agents=1 prompt="Hello"   # Run goose agent
```

## Code Architecture

### Crate Dependency Hierarchy

```
buzz-core (zero I/O — types, verification, filter matching, kind registry)
  ├── buzz-db       (Postgres: events, channels, tokens, workflows, audit)
  ├── buzz-auth     (NIP-42/98, API tokens, scopes, rate limiting)
  ├── buzz-pubsub   (Redis pub/sub, presence, typing indicators)
  ├── buzz-search   (Postgres FTS)
  ├── buzz-audit    (Hash-chain tamper-evident log)
  └── buzz-workflow (YAML-as-code automation, evalexpr conditions)
       └── buzz-relay (ties everything together — the Axum server)
```

Subsystems are isolated from each other — `buzz-workflow` never calls `buzz-pubsub`. All cross-system coordination happens through `buzz-relay`.

### Event Pipeline (12 Steps)

When `["EVENT", <event>]` is received:

1. Auth check → 2. Pubkey match → 3. Kind 22242 rejection → 4. Ephemeral route (20000-29999) → 5. Schnorr sig verify → 6. Channel membership → 7. DB insert → 8. Redis publish → 9. Fan-out to subscribers → 10. Search index → 11. Audit log → 12. Workflow trigger

Steps 10-12 are fire-and-forget. Fan-out excludes global subscriptions from private channel events. Kinds 46001-46012 never trigger workflows.

### Connection Lifecycle

Each WebSocket: acquire semaphore → send NIP-42 auth challenge → client authenticates → run 3 concurrent loops (recv, send, heartbeat at 30s ping / 3 missed = disconnect). Max 1024 subs/conn, 500 results/filter, 64KB frame.

### Key Kind Ranges

| Range | Meaning |
|-------|---------|
| 0-9999 | Standard Nostr (NIP-01..NIP-XX) |
| 10000-19999 | Replaceable (NIP-16) |
| 20000-29999 | Ephemeral — not stored/audited |
| 30000-39999 | Parameterized replaceable |
| 40000-49999 | Buzz custom |

Key Buzz kinds: 9/40002 (stream messages), 7 (reactions), 45001/45003 (forum), 46001-46012 (workflow), 20001 (presence), 40100 (canvas), 43001 (agent jobs). All defined in `buzz-core/src/kind.rs`.

### Desktop App Architecture

- **Community switching** uses React key remounting. Module-level singletons must reset in `resetCommunityState()` (`useCommunityInit.ts`). See AGENTS.md for the full list.
- **Text sizing must use rem** (never px) for Cmd+/- zoom. Stock Tailwind tokens or `text-2xs`/`text-3xs` meta tokens. CI guard blocks new arbitrary px/rem text sizes.
- **React.memo** requires all props reference-stable. `useMutation`/`useQuery` return new objects each render.
- **Preview features** gated in `preview-features.json`.
- Key files: `desktop/src/app/App.tsx`, `desktop/src/main.tsx`, `desktop/src/features/communities/useCommunityInit.ts`

## Adding a New Event Kind

1. Define constant in `buzz-core/src/kind.rs`
2. Register scope in `buzz-relay/src/handlers/ingest.rs` (`required_scope_for_kind()`)
3. Add side effect handler in `buzz-relay/src/handlers/side_effects.rs`
4. Add DB queries in `buzz-db/src/` if queryable
5. Write tests

## Important Rules

- **No `unsafe` code**
- **No `unwrap()` or `expect()` in production** — use `?` and proper error types
- **Use `thiserror`** for library errors, `anyhow` for binaries
- **Use `tracing`** with structured fields (not string interpolation)
- **Conventional Commits**: `type(scope): description` — feat/fix/docs/refactor/test/chore
- **Public API must have doc comments**
- **Desktop crate excluded from root workspace** — use `cargo test --manifest-path desktop/src-tauri/Cargo.toml`
- **Run `just ci` before every PR**

## Graphify Knowledge Graph

This project has a [knowledge graph](graphify-out/GRAPH_REPORT.md) built via AST extraction (no LLM cost). It's automatically updated on `git commit` and `git checkout` via Git hooks.

- **Graph report**: [graphify-out/GRAPH_REPORT.md](graphify-out/GRAPH_REPORT.md) — God nodes, surprising connections, import cycles, named communities
- **Interactive viz**: open `graphify-out/graph.html` in a browser
- **Raw data**: `graphify-out/graph.json`
- **Query**: run `graphify query "<question>"` to traverse the graph
