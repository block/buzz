# Contributing to Buzz

Welcome, and thank you for your interest in contributing! Buzz is an
open-source project and we're glad you're here. This guide will help you
get from zero to a merged pull request.

If you have questions that aren't answered here, open a GitHub Discussion or
reach out in the community channels.

---

## Table of Contents

1. [Code of Conduct](#code-of-conduct)
2. [Setting Up the Development Environment](#setting-up-the-development-environment)
3. [Running Tests](#running-tests)
4. [Code Style](#code-style)
5. [Making a Pull Request](#making-a-pull-request)
6. [Architecture Overview](#architecture-overview)
7. [Ecosystem](#ecosystem)
8. [How to Add a New Event Kind](#how-to-add-a-new-event-kind)
9. [How to Add a New MCP Tool](#how-to-add-a-new-mcp-tool)
10. [How to Add a New API Endpoint](#how-to-add-a-new-api-endpoint)
11. [License and CLA](#license-and-cla)

---

## Code of Conduct

This project follows the [Contributor Covenant v2.1](CODE_OF_CONDUCT.md).
By participating you agree to uphold these standards. Please report
unacceptable behavior to **conduct@buzz-relay.org**.

---

## Setting Up the Development Environment

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.88+ | Install via [rustup](https://rustup.rs/) |
| Node.js | 24+ | Required for desktop app commands and `just ci` |
| pnpm | 10+ | Required for desktop app commands and `just ci` |
| Flutter | 3.41+ | Required for mobile app — install via [flutter.dev](https://docs.flutter.dev/get-started/install) |
| Docker | 24+ | For Postgres, Redis, MinIO |
| `just` | latest | Task runner — `cargo install just` |
| `lefthook` | 2.1.3 (Hermit-pinned) | Auto-installed by `just hooks` — no manual install needed |
| `sqlx` migrations | workspace crate | `just migrate` applies embedded migrations from `migrations/` |

This repo uses [Hermit](https://cashapp.github.io/hermit/) for toolchain
pinning. Activate it once per shell session:

```bash
. ./bin/activate-hermit
```

Hermit pins Rust, `just`, Node, pnpm, and other tools to the versions in
`bin/`. Each tool is downloaded on first use. You can also run `just bootstrap`
(which `just setup` calls automatically) to pre-download all required tools
upfront. If you don't use Hermit, ensure your toolchain meets the minimum
versions in the table above.

### First-Time Setup

```bash
# 1. Clone the repo
git clone https://github.com/block/buzz.git
cd buzz

# 2. Activate Hermit (optional but recommended)
. ./bin/activate-hermit

# 3. Bootstrap tools + infrastructure
just setup

# 4. Install Git hooks (optional, recommended)
just hooks
```

`just setup` runs `just bootstrap` first — it copies `.env.example` to `.env`
if it doesn't already exist, and invokes `cargo`, `node`, and `pnpm` to trigger
Hermit's lazy tool download (each tool is fetched once on first invocation and
cached thereafter). You can also run `just bootstrap` independently at any time;
it is safe to re-run.

`just setup` then starts Docker services (Postgres on `:5432`, Redis on `:6379`,
Adminer on `:8082`, Keycloak on `:8180` for local OAuth/OIDC testing, MinIO on
`:9000` for media storage, and Prometheus on `:9090` for metrics) and runs all
pending database migrations.

### Running the Relay and Desktop App

```bash
just dev   # starts the relay + desktop app in one command
```

`just dev` builds all agent tools, starts the relay (`ws://localhost:3000`) in
the background, and launches the Tauri desktop app. The relay process is
automatically killed when you quit the app or press Ctrl+C.

For a split-terminal workflow (relay logs visible separately from Vite output):

```bash
just relay        # terminal 1 — relay on ws://localhost:3000
just desktop-dev  # terminal 2 — Vite dev server only (no Tauri shell)
```

### Stopping / Resetting

```bash
just down    # Stop Docker services, keep data
just reset   # Wipe all dev state and recreate it; installed Buzz is preserved
```

Development desktop state uses separate bundle identifiers
(`xyz.block.buzz.app.dev` and per-worktree variants), a separate keyring service
(`buzz-desktop-dev`), and `~/.buzz-dev`. `just reset` removes those dev-only
locations and the local Docker volumes. It does not touch the installed app's
`xyz.block.buzz.app` data, `buzz-desktop` keyring service, or `~/.buzz` nest.

---

## Running Tests

### Unit Tests (no infrastructure required)

```bash
just test-unit
```

Unit tests are self-contained and run without Docker. They cover event
parsing, filter matching, auth logic, workflow YAML parsing, and more.

### Integration Tests (requires running infrastructure)

```bash
just test
```

Integration tests spin up the relay and exercise the full stack — WebSocket
connections, NIP-42 auth, event ingestion, search indexing, and workflow
execution. `just test` starts Docker services automatically if they're not
already running.

### End-to-End Tests

End-to-end tests live in `crates/buzz-test-client/tests/`:

- `e2e_relay.rs` — WebSocket relay tests
- `e2e_mcp.rs` — MCP tool tests
- `e2e_nostr_interop.rs` — Nostr protocol interoperability tests
- `e2e_media.rs` — media upload/download tests
- `e2e_media_extended.rs` — extended media tests (GIF, image processing)

Run them with (requires running infrastructure):

```bash
cargo test -p buzz-test-client -- --ignored
```

See `TESTING.md` for the full multi-agent E2E testing guide.

### CI Gate

Before opening a PR, run the full CI gate locally:

```bash
just ci
# Runs: check + unit tests + desktop build + Tauri check + mobile tests
```

This is the same check that runs in CI. PRs that fail `just ci` will not be
merged.

---

## Code Style

### Formatting

We use `rustfmt` with default settings. Format your code before committing:

```bash
cargo fmt --all
```

To check without modifying:

```bash
cargo fmt --all -- --check
```

### Linting

We use `clippy` with warnings-as-errors:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Fix all clippy warnings before submitting a PR. If you believe a warning is
a false positive, add a targeted `#[allow(...)]` with a comment explaining
why.

### No Unsafe Code

All crates enforce `#![deny(unsafe_code)]`. Do not add unsafe blocks. If you
believe unsafe is genuinely necessary, open an issue first to discuss the
approach.

### Error Handling

- Use `thiserror` for library error types.
- Use `anyhow` for binary / application-level error propagation.
- Do not use `unwrap()` or `expect()` in production code paths. Use `?` or
  explicit error handling. `unwrap()` is acceptable in tests.

### Logging and Tracing

Use the `tracing` crate for all instrumentation. Prefer structured fields
over string interpolation:

```rust
// Good
tracing::info!(channel_id = %id, event_kind = kind, "Event ingested");

// Avoid
tracing::info!("Event ingested: channel={id} kind={kind}");
```

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(mcp): add get_feed_actions tool
fix(auth): reject expired NIP-42 challenges
docs(agents): document workflow MCP tools
refactor(db): extract channel queries into channel.rs
test(workflow): add approval gate integration test
```

The type prefix (`feat`, `fix`, `docs`, `refactor`, `test`, `chore`) is
required. The scope (in parentheses) is optional but encouraged.

---

## Making a Pull Request

### Before You Start

- Check open issues and PRs to avoid duplicate work.
- For significant changes, open an issue first to discuss the approach.
- For small fixes (typos, doc improvements, obvious bugs), go ahead and open
  a PR directly.

### What a Good PR Looks Like

1. **Focused** — one logical change per PR. If you're fixing a bug and
   refactoring a module, split them into two PRs.

2. **Tested** — new behavior has tests. Bug fixes include a regression test.
   If a test is impractical, explain why in the PR description.

3. **Documented** — public APIs, new event kinds, new MCP tools, and new
   config variables are documented. Update `README.md`, `AGENTS.md`, or
   `VISION.md` as appropriate.

4. **CI passing** — `just ci` passes locally before you push.

5. **Clear description** — the PR description explains:
   - What problem this solves (or what feature it adds)
   - How it was implemented (key decisions, trade-offs)
   - How to test it manually (if applicable)
   - Any follow-up work deferred to a future PR

### PR Checklist

```
- [ ] `just ci` passes (fmt + clippy + unit tests + mobile)
- [ ] Integration tests pass (`just test`)
- [ ] New public APIs / tools / endpoints are documented
- [ ] No new `unwrap()` in production code paths
- [ ] No new `unsafe` blocks
```

### Review Process

- A maintainer will review your PR within a few business days.
- Address review comments by pushing new commits (don't force-push during
  review; it makes it hard to see what changed).
- Once approved, a maintainer will squash-merge your PR.

---

## Architecture Overview

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full system design and
[AGENTS.md](AGENTS.md#repo-structure) for the complete crate map. The key
design principles:

**The relay is the single source of truth.** All state flows through the
event store. Crates communicate through the database and Redis pub/sub — not
through direct function calls across crate boundaries (with the exception
of `buzz-core` types, which are shared everywhere).

**Event kinds are the only switch.** Every action in the system — a message,
a reaction, a workflow step, a canvas update — is a Nostr event with a kind
integer. Adding a new feature means defining a new kind. No breaking changes
to existing clients.

---

## Ecosystem

Buzz is developed across multiple repositories. This repo (`block/buzz`)
is the open-source home for all application code — the relay, desktop app,
mobile app, CLI, and agent harness. Internal repositories handle
enterprise-signed builds and infrastructure deployment.

See [AGENTS.md § Ecosystem](AGENTS.md#ecosystem) for the full repo table and
dependency diagram.

**External contributors:** Fork `block/buzz`, open a PR, and CI runs
automatically. No special access is required.

**Block team members:** See the internal
[sprout-releases CONTRIBUTING.md](https://github.com/squareup/sprout-releases/blob/main/CONTRIBUTING.md)
for team access setup, onboarding, and the full repo inventory. See
[RELEASING.md](RELEASING.md) for the release process.

---

## How to Add a New Event Kind

1. **Define the kind constant** in `buzz-core/src/kind.rs`:

   ```rust
   /// My new event kind — description of what it represents.
   pub const KIND_MY_FEATURE: u32 = 4XXXX;
   ```

   Pick a kind number in the appropriate sub-range defined in `kind.rs`.
   Check the `ALL_KINDS` array for collisions. Each sub-range is documented
   with comments in the file.

2. **Define the payload type** in the appropriate module in `buzz-core/src/`
   (e.g., alongside `event.rs`) if the content field is structured JSON:

   ```rust
   #[derive(Debug, Serialize, Deserialize)]
   pub struct MyFeaturePayload {
       pub field_one: String,
       pub field_two: Option<u64>,
   }
   ```

3. **Register the kind's required scope** in
   `crates/buzz-relay/src/handlers/ingest.rs` inside
   `required_scope_for_kind()`. This controls which auth scope a caller
   needs to submit the event:

   ```rust
   KIND_MY_FEATURE => Ok(Scope::MessagesWrite),
   ```

4. **Handle post-storage side effects** by adding a match arm in
   `crates/buzz-relay/src/handlers/side_effects.rs` inside
   `handle_side_effects()`:

   ```rust
   KIND_MY_FEATURE => handle_my_feature(event, state).await?,
   ```

   `handle_side_effects()` runs after the event is stored — use it for
   notifications, cache invalidation, or derived data. If the new kind
   also needs an HTTP bridge surface (for example, a protocol helper that
   cannot practically use WebSocket), add a handler in
   `crates/buzz-relay/src/api/` and register it in
   `crates/buzz-relay/src/router.rs`.

5. **Persist to the database** — if the event needs to be queryable, add a
   handler in `buzz-db/src/` (e.g., `buzz-db/src/my_feature.rs`) with
   the appropriate `INSERT` and `SELECT` queries.

6. **Index for search** (if applicable) — Postgres FTS indexes persisted
   events automatically via the `events.search_tsv` generated column. To
   exclude a privacy-sensitive kind from search, add it to the `CASE WHEN
   kind IN (...)` exclusion in the `search_tsv` definition (see the initial
   schema migration) rather than wiring a separate indexer.

7. **Audit** — the audit log captures all events automatically; no changes
   needed unless you need custom audit metadata.

8. **Write tests** — add a unit test for payload serialization in
   `buzz-core` and an integration test in `buzz-test-client` that sends
   the new event kind and verifies the expected behavior.

9. **Document** — `kind.rs` is the authoritative registry of all kind numbers.
   Update `README.md` if it's a user-facing feature.

---

## How to Add a New API Endpoint

Prefer a signed Nostr event and the existing WebSocket/`POST /events` ingest
path over adding endpoint-specific JSON APIs. The relay intentionally exposes
only a narrow HTTP surface: NIP-11/NIP-05 metadata, `/events`, `/query`,
`/count`, `/hooks/{id}`, Blossom media, git smart HTTP, git policy hooks, and
health probes.

If an HTTP endpoint is still necessary:

1. **Define the handler** in the appropriate module under
   `crates/buzz-relay/src/api/`. Resolve the request tenant before any auth or
   data lookup, use NIP-98 when the endpoint accepts user credentials, and keep
   community scoping explicit.

2. **Register the route** in `crates/buzz-relay/src/router.rs` using the
   narrowest path possible. Do not add new `/api/*` compatibility routes unless
   the product decision explicitly calls for one.

3. **Add database queries** in `buzz-db/src/` only when the endpoint cannot be
   expressed through the existing event query paths.

4. **Handle errors** using the `api_error()`, `internal_error()`, and
   `not_found()` helpers in `buzz-relay/src/api/mod.rs`. Return
   `(StatusCode, Json<Value>)` tuples.

5. **Write tests** with the `buzz-test-client` harness in
   `crates/buzz-test-client/tests/`, covering auth, community scoping, and the
   relevant success path.

6. **Document** any public endpoint in `ARCHITECTURE.md` and user-facing docs.

---

## License and CLA

Buzz is licensed under the **Apache License, Version 2.0**. See
[LICENSE](LICENSE) for the full text.

By submitting a pull request, you agree that your contribution is licensed
under the Apache 2.0 license and that you have the right to submit it.

If your employer has rights to intellectual property you create, you may need
their sign-off. When in doubt, check with your legal team.

---

*Thank you for contributing to Buzz. Every bug report, documentation fix,
and code contribution makes the project better for everyone. 🐝*
