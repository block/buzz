# Task 2 report: trusted ACP output publishing

Implementation commit: `1ab0721481bcc4d3ac19bcb8aab65ddacb0a3a1a`

## RED evidence

- `cargo test -p buzz-acp config::tests::publish_agent_output_is_off_by_default_and_trigger_reply_is_opt_in -- --exact` failed because `CliArgs` had no `publish_agent_output` field and `PublishAgentOutput` did not exist.
- `cargo test -p buzz-acp acp::tests::agent_output_capture_accumulates_messages_and_fails_closed -- --exact` failed because the capture/reset/take API did not exist.
- Targeting and terminal-policy tests likewise failed before their helpers were introduced.

## GREEN evidence

- Round-1 review fix: `cargo test -p buzz-acp --lib` — 673 unit tests passed.
- `cargo fmt --check` passed.
- `cargo clippy -p buzz-acp --all-targets -- -D warnings` passed.

## Changed files

- `crates/buzz-acp/src/config.rs` — public opt-in policy, CLI/env parsing, and fail-closed pilot invariant validation.
- `crates/buzz-acp/src/acp.rs` — bounded per-prompt agent-message capture and invalidation.
- `crates/buzz-acp/src/pool.rs` — trusted batch-last targeting, signing, terminal gating, exact-ID confirmation/retry, and one pending event.
- `crates/buzz-acp/src/lib.rs` — public policy re-export and runtime wiring.
- `crates/buzz-acp/src/relay.rs` — single-attempt durable submission, preventing an unconfirmed transport retry.

## Round-1 review fixes

- Only the exact configured owner may provide a trusted trigger; sibling-agent events are rejected even though inbound owner-only policy permits them.
- Trigger-reply startup now rejects an owner value that cannot parse as a Nostr public key.
- `accepted: false` is terminal: it is neither confirmed nor retried and never occupies the pending slot. Ambiguous responses are confirmed by exact ID before identical-ID retry; the regression test observes the same event ID on both writes.

## Concerns

- Publishing is intentionally local-pilot-only: one pending signed event is retained in memory, so it is not durable across process restart.
- The single-attempt durable submission is unit-tested; full relay behavior remains unexercised against a production relay in this task.
