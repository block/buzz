# Repository Skill Overlay

## Repository shape

Buzz is a Rust workspace with a Tauri/React desktop client and a Flutter mobile
client. The relay is Nostr-first: new durable behavior is normally expressed as
signed events, with a narrow HTTP bridge for generic event submission and
queries.

## Architecture sources

- `AGENTS.md` — contributor rules and current repository map
- `ARCHITECTURE.md` — production topology and boundaries
- `crates/buzz-core` — event verification, kinds, filters, and shared types
- `crates/buzz-pair-relay` — bounded single-process relay precedent

## Implementation constraints

- Rust production paths use `Result` propagation rather than new `unwrap()` or
  `expect()` calls.
- Public Rust APIs have documentation comments.
- New protocol behavior is specified under `specs/` before implementation.
- The local relay is a separate crate; the pairing relay's intentionally narrow
  security boundary is not broadened.
- Local mode binds to loopback by default and requires an explicit address to
  become reachable elsewhere.

## Verification

Activate Hermit before repository tooling:

```bash
. ./bin/activate-hermit
cargo fmt --all --check
cargo test -p buzz-local-relay
cargo clippy -p buzz-local-relay --all-targets -- -D warnings
```

Run `just ci` before proposing a repository-wide change.
