# Buzz contributor instructions

Buzz is a monorepo for a self-hosted, Nostr-based collaboration platform: a Rust relay and shared crates, a Tauri desktop client, a Flutter mobile client, and supporting tools.

## Scope and routing

Instructions in this file apply everywhere unless a closer `AGENTS.md` adds to
or overrides them. Read the most specific applicable file before editing.
If instructions at the same scope conflict, follow the task request and ask
before choosing an interpretation that changes the requested scope.

| Before editing | Read first |
| --- | --- |
| Relay, event ingest, database, or auth | [`crates/buzz-relay/AGENTS.md`](crates/buzz-relay/AGENTS.md) |
| Agent-facing CLI | [`crates/buzz-cli/AGENTS.md`](crates/buzz-cli/AGENTS.md) |
| Desktop application | [`desktop/AGENTS.md`](desktop/AGENTS.md) |
| Desktop Playwright tests or screenshots | [`desktop/tests/e2e/AGENTS.md`](desktop/tests/e2e/AGENTS.md) and [`docs/pr-screenshots.md`](docs/pr-screenshots.md) when posting PR screenshots |
| Desktop community switching | [`desktop/src/features/communities/AGENTS.md`](desktop/src/features/communities/AGENTS.md) |
| Desktop agent configuration | [`desktop/src/features/agents/AGENTS.md`](desktop/src/features/agents/AGENTS.md) |
| Mobile client | [`mobile/AGENTS.md`](mobile/AGENTS.md) |

## Environment and completion

Activate the pinned toolchain before Git, hooks, or project commands:

```bash
. ./bin/activate-hermit
just setup       # bootstrap local dependencies and development services
just hooks       # install repository hooks (run separately from setup)
```

Choose checks proportional to the files changed. `just ci` is the standard
repository-wide gate; `just test` runs unit and integration coverage, and
`just test-integration` starts required services when needed. The area-specific
instructions name narrower checks. Before handing off, run applicable tests,
formatters, and linters; report what ran and what could not run.

For a focused baseline, `just check` runs repository formatting and lint checks,
and `just test-unit` runs infrastructure-free unit coverage. Do not use a broad
green check as a substitute for the affected area's targeted tests.

## Durable code rules

- Do not introduce `unsafe`.
- Do not introduce `unwrap()` or `expect()` in production paths; propagate or
  handle errors with appropriate types.
- Document new public APIs.

Buzz’s primary operation model is signed Nostr events. Before adding an API or
changing event behavior, read the architecture and contributor guidance; event
kinds live in [`crates/buzz-core/src/kind.rs`](crates/buzz-core/src/kind.rs).
Prefer an event and the existing ingest path over a new endpoint unless the
operation genuinely needs the documented HTTP surface.

## Reference documentation

- [Contributing](CONTRIBUTING.md): setup, style, and contribution workflow.
- [Architecture](ARCHITECTURE.md): system and protocol design.
- [Testing](TESTING.md): test environments and end-to-end coverage.
- [Releasing](RELEASING.md): release process.
- [README](README.md): project overview and quick start.
