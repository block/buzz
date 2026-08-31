# Buzz Admin Community Archive Commands Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add reversible, single-operator `buzz-admin communities archive` and `buzz-admin communities unarchive` commands with exact-host/current-owner safety guards and live connection propagation.

**Architecture:** Add a thin `communities` command module to `buzz-admin`. It will reuse the existing owner-aware, idempotent `buzz-db` lifecycle methods, construct the same deployment-host guard used by the relay operator endpoint, and publish the existing community disconnect command to Redis after archival. Operator identity and reason are required command inputs and included in machine-readable output; no second-operator approval record is created.

**Tech Stack:** Rust, clap, buzz-db, buzz-core tenant normalization, buzz-pubsub/Redis, serde_json, existing Buzz unit and database integration tests.

---

### Task 1: Specify the CLI surface with failing tests

**Files:**
- Modify: `crates/buzz-admin/src/main.rs`
- Test: `crates/buzz-admin/src/main.rs`

**Step 1: Write failing parser tests**

Add tests that parse:

```text
buzz-admin communities archive --host example.communities.buzz.xyz --owner-pubkey <64-hex> --operator-id codex --reason requested-deletion
buzz-admin communities unarchive --host example.communities.buzz.xyz --owner-pubkey <64-hex> --operator-id codex --reason rollback
```

Assert both subcommands capture the exact host, owner pubkey, operator identity, and reason. Assert omitting `--owner-pubkey`, `--operator-id`, or `--reason` fails parsing.

**Step 2: Run the test and verify RED**

Run: `. ./bin/activate-hermit && cargo test -p buzz-admin communities_command`

Expected: compilation/parser failure because the `communities` command surface does not exist.

### Task 2: Add lifecycle validation and execution

**Files:**
- Create: `crates/buzz-admin/src/communities.rs`
- Modify: `crates/buzz-admin/src/main.rs`

**Step 1: Implement the minimum command surface**

Define a clap `CommunitiesCommand` enum with `Archive` and `Unarchive` variants. Both require `--host`, `--owner-pubkey`, `--operator-id`, and `--reason`.

**Step 2: Add validation tests before helpers**

Test normalization of uppercase/default-port/trailing-dot authorities and rejection of schemes, paths, whitespace, malformed authorities, and invalid owner pubkeys. Run those tests and verify they fail before adding helpers.

**Step 3: Implement validation helpers**

Normalize only a bare authority using the shared tenant normalization rules and parse the owner as a Nostr public key, returning canonical lowercase hex.

**Step 4: Implement archive**

Connect to the existing `DATABASE_URL`. Resolve the protected deployment host from `RELAY_URL`. Call `Db::archive_community_owned_by`; fail closed when no exact host/current-owner row is updated. After commit, connect to `REDIS_URL` and publish `ConnControl::DisconnectCommunity` under the returned community tenant. Emit JSON containing `community_id`, `host`, `archived_at`, `status`, `operator_id`, `reason`, and propagation subscriber count. If Redis publication fails, report that archival committed but propagation is pending and return nonzero so a safe idempotent retry is visible.

**Step 5: Implement unarchive**

Call `Db::unarchive_community_owned_by` with the same exact host/current-owner guard. Emit JSON containing `community_id`, `host`, `archived_at: null`, `status: active`, `operator_id`, and `reason`. No disconnect is published.

**Step 6: Run focused tests and verify GREEN**

Run: `. ./bin/activate-hermit && cargo test -p buzz-admin`

Expected: all `buzz-admin` tests pass.

### Task 3: Verify behavior and publish the PR

**Files:**
- Modify only files required by Tasks 1–2 plus this plan.

**Step 1: Run formatting and lint/build gates**

Run:

```bash
. ./bin/activate-hermit
cargo fmt --all -- --check
cargo clippy -p buzz-admin --all-targets -- -D warnings
cargo test -p buzz-admin
cargo build -p buzz-admin
```

Expected: every command exits zero with no warnings or failed tests.

**Step 2: Verify help output**

Run:

```bash
target/debug/buzz-admin communities archive --help
target/debug/buzz-admin communities unarchive --help
```

Expected: required exact-host, current-owner, operator identity, and reason arguments are documented; no approval argument exists.

**Step 3: Commit and publish**

Commit the plan, tests, and implementation on `codex/community-archive-admin`, push it from the local machine after reviewing the transferred patch, and open a draft PR describing reversibility, safety guards, propagation semantics, and test evidence.
