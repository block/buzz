# Buzz Strict-Owner Source Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `respond-to=owner-only` admit only the exact registered owner as automatic work while retaining sibling-authored events in readable relay context.

**Architecture:** Keep the existing event pipeline and make the smallest change at its authoritative pre-queue boundary, `author_allowed`. Tests exercise the real author gate, real subscription matcher, signed Nostr events, and real context parser so trigger authority and visibility are independently pinned.

**Tech Stack:** Rust 2024 workspace, Tokio tests, `nostr` signed events, Cargo test/fmt/clippy.

## Global Constraints

- Do not send Buzz channel traffic.
- Do not deploy or restart Buzz.
- Do not mutate production configuration.
- Do not modify Gyre, prloop, agent roles, or Buzz source outside this isolated worktree.
- Preserve current `Allowlist`, `Anyone`, `Nobody`, setup-mode reuse, and history-query semantics.
- Every focused test command must report a nonzero executed-test count.
- Stop after the source fix, tests, rollback description, and unexecuted canary proposal.

---

### Task 1: Pin Trigger Authority and Context Visibility in Failing Tests

**Files:**
- Modify: `crates/buzz-acp/src/lib.rs` in `author_gate_tests`
- Modify: `crates/buzz-acp/src/pool.rs` in `pool::tests`

**Interfaces:**
- Consumes: `author_allowed`, `filter::match_event`, `SubscriptionRule`, `OwnerCache`, and `parse_nostr_thread_response`.
- Produces: seven `strict_owner_*` regression tests covering requirements A–F without live relay access.

- [ ] **Step 1: Add real signed-event helpers to `author_gate_tests`**

```rust
fn signed_event(keys: &nostr::Keys, kind: u32) -> nostr::Event {
    nostr::EventBuilder::new(nostr::Kind::Custom(kind as u16), "trigger")
        .sign_with_keys(keys)
        .expect("test event must sign")
}

fn cache_for_keys(
    owner: &nostr::Keys,
    sibling: &nostr::Keys,
    agent: &nostr::Keys,
) -> OwnerCache {
    let cache = OwnerCache::new(Some(owner.public_key().to_hex()));
    cache.cache_sibling(sibling.public_key().to_hex(), true);
    cache.cache_sibling(agent.public_key().to_hex(), false);
    cache
}

async fn wildcard_matches(event: &nostr::Event, agent: &nostr::Keys) -> bool {
    let rule = SubscriptionRule {
        name: "wildcard".into(),
        ..SubscriptionRule::default()
    };
    filter::match_event(
        event,
        Uuid::new_v4(),
        &[rule],
        &agent.public_key().to_hex(),
    )
    .await
    .is_some()
}

async fn owner_only_allows(event: &nostr::Event, is_dm: bool, cache: &OwnerCache) -> bool {
    author_allowed(
        &RespondTo::OwnerOnly,
        &HashSet::new(),
        &event.pubkey.to_hex(),
        is_dm,
        cache,
        &dummy_rest_client(),
    )
    .await
}
```

- [ ] **Step 2: Replace the legacy owner-plus-sibling expectation with strict-owner work tests**

Replace `test_owner_only_admits_owner_and_sibling_to_steer` and the combined
DM-mode test with these exact tests:

```rust
#[tokio::test]
async fn strict_owner_accepts_owner_kind9_as_work() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);
    let event = signed_event(&owner, 9);

    assert!(wildcard_matches(&event, &agent).await);
    assert!(owner_only_allows(&event, false, &cache).await);
}

#[tokio::test]
async fn strict_owner_rejects_sibling_kind9_as_work() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);
    let event = signed_event(&sibling, 9);

    assert!(wildcard_matches(&event, &agent).await);
    assert!(!owner_only_allows(&event, false, &cache).await);
}

#[tokio::test]
async fn strict_owner_rejects_self_authored_kind9() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);
    let event = signed_event(&agent, 9);

    assert!(!owner_only_allows(&event, false, &cache).await);
}

#[tokio::test]
async fn strict_owner_rejects_sibling_lifecycle_kinds_even_when_filter_wildcard_matches() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);

    for kind in [5, 7, 20002] {
        let event = signed_event(&sibling, kind);
        assert!(wildcard_matches(&event, &agent).await, "kind {kind}");
        assert!(
            !owner_only_allows(&event, false, &cache).await,
            "sibling kind {kind} must not reach work"
        );
    }
}

#[tokio::test]
async fn strict_owner_owner_message_cannot_seed_sibling_reply_chain() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);
    let mut events = vec![signed_event(&owner, 9)];
    events.extend((0..32).map(|_| signed_event(&sibling, 9)));

    let mut eligible_count = 0;
    for event in &events {
        if wildcard_matches(event, &agent).await
            && owner_only_allows(event, false, &cache).await
        {
            eligible_count += 1;
        }
    }

    assert_eq!(eligible_count, 1);
}

#[tokio::test]
async fn strict_owner_dm_accepts_owner_but_rejects_sibling() {
    let owner = nostr::Keys::generate();
    let sibling = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let cache = cache_for_keys(&owner, &sibling, &agent);

    assert!(owner_only_allows(&signed_event(&owner, 9), true, &cache).await);
    assert!(!owner_only_allows(&signed_event(&sibling, 9), true, &cache).await);
}

#[tokio::test]
async fn test_dm_admits_owner_and_sibling_in_non_strict_modes() {
    let cache = cache_with_sibling();
    for mode in [RespondTo::Allowlist, RespondTo::Anyone] {
        for (who, label) in [(OWNER, "owner"), (SIBLING, "sibling")] {
            assert!(
                author_allowed(
                    &mode,
                    &HashSet::new(),
                    who,
                    true,
                    &cache,
                    &dummy_rest_client(),
                )
                .await,
                "in a DM under {mode}, the {label} must still be admitted"
            );
        }
    }
}
```

Retain the existing allowlist and non-`OwnerOnly` DM tests so unrelated response modes remain locked.

- [ ] **Step 3: Add the independent readable-context test**

```rust
#[test]
fn strict_owner_sibling_message_remains_readable_context() {
    let root_id = "aa".repeat(32);
    let sibling = "bb".repeat(32);
    let context = parse_nostr_thread_response(
        serde_json::json!([
            {
                "id": root_id,
                "pubkey": "cc".repeat(32),
                "content": "owner prompt",
                "created_at": 1
            },
            {
                "id": "dd".repeat(32),
                "pubkey": sibling,
                "content": "sibling context",
                "created_at": 2
            }
        ]),
        &"aa".repeat(32),
    )
    .expect("thread context must remain readable");

    match context {
        ConversationContext::Thread { messages, .. } => {
            assert_eq!(messages[1].pubkey, "bb".repeat(32));
            assert_eq!(messages[1].content, "sibling context");
        }
        _ => panic!("expected thread context"),
    }
}
```

- [ ] **Step 4: Run the focused RED test set and retain the receipt**

Run:

```bash
cargo test -p buzz-acp strict_owner -- --nocapture
```

Expected: seven tests execute; owner, self, and context cases pass; sibling kind-9, sibling lifecycle, DM sibling, and reply-chain assertions fail because current `OwnerOnly` calls `is_owner_or_sibling`.

---

### Task 2: Implement the Minimal Exact-Owner Guard

**Files:**
- Modify: `crates/buzz-acp/src/lib.rs:188-258`
- Modify: `crates/buzz-acp/src/lib.rs:2136-2145` comments

**Interfaces:**
- Consumes: `OwnerCache::get`, existing `RespondTo` values, and existing NIP-OA sibling lookup.
- Produces: `is_owner(author, owner_cache) -> bool`; corrected `OwnerOnly` behavior in public channels and DMs.

- [ ] **Step 1: Add the exact-owner predicate**

```rust
fn is_owner(author: &str, owner_cache: &OwnerCache) -> bool {
    owner_cache.get().is_some_and(|owner| author == owner)
}
```

- [ ] **Step 2: Reuse it in sibling discovery and the `OwnerOnly` branches**

```rust
if is_owner(author, owner_cache) {
    return true;
}
```

For DMs:

```rust
return match respond_to {
    RespondTo::Nobody => false,
    RespondTo::OwnerOnly => is_owner(author, owner_cache),
    RespondTo::Allowlist | RespondTo::Anyone => {
        is_owner_or_sibling(author, owner_cache, rest_client).await
    }
};
```

For non-DMs:

```rust
RespondTo::OwnerOnly => is_owner(author, owner_cache),
```

- [ ] **Step 3: Correct comments without changing other response modes**

State that `OwnerOnly` accepts the exact owner, `Allowlist` preserves its current owner/sibling/explicit-list behavior, and the gate still runs before subscription matching and queueing.

- [ ] **Step 4: Run the focused GREEN test set**

Run:

```bash
cargo test -p buzz-acp strict_owner -- --nocapture
```

Expected: seven executed tests, seven passed, zero failed.

- [ ] **Step 5: Run the focused author-gate module**

Run:

```bash
cargo test -p buzz-acp author_gate_tests -- --nocapture
```

Expected: nonzero executed tests, all passed, including unchanged allowlist and DM hardening tests.

---

### Task 3: Broader Verification and Local Commit

**Files:**
- Verify: `crates/buzz-acp/src/lib.rs`
- Verify: `crates/buzz-acp/src/pool.rs`
- Verify: `docs/superpowers/specs/2026-08-10-buzz-strict-owner-source-guard-design.md`
- Verify: `docs/superpowers/plans/2026-08-10-buzz-strict-owner-source-guard.md`

**Interfaces:**
- Consumes: the focused green source state.
- Produces: formatting, lint, full-suite, diff, rollback, and no-deploy receipts.

- [ ] **Step 1: Format and verify formatting**

```bash
cargo fmt --all
cargo fmt --all -- --check
```

- [ ] **Step 2: Run the broader relevant test suite**

```bash
cargo test -p buzz-acp
```

Expected: at least the 607 baseline tests plus the net-new regression tests execute across unit and integration targets; zero failures.

- [ ] **Step 3: Run targeted lint**

```bash
cargo clippy -p buzz-acp --all-targets -- -D warnings
```

Expected: exit zero with no warnings.

- [ ] **Step 4: Re-derive scope and rollback from Git**

```bash
git diff --check
git status --short
git diff --stat HEAD~1
git diff HEAD~1 -- crates/buzz-acp/src/lib.rs crates/buzz-acp/src/pool.rs
```

Confirm no production configuration, Buzz Desktop, Gyre, or prloop files appear. Rollback is the inverse of the exact source/test diff or removal of the local fix commit.

- [ ] **Step 5: Commit the source fix locally**

```bash
git add -- crates/buzz-acp/src/lib.rs crates/buzz-acp/src/pool.rs docs/superpowers/plans/2026-08-10-buzz-strict-owner-source-guard.md
git commit -m "fix(acp): separate owner triggers from sibling context"
```

- [ ] **Step 6: Propose, but do not execute, a bounded live canary**

The proposal must require a build/deploy authorization, retained lifecycle kind exclusions, one intended seat first, passive observation, exactly one owner text message, no synthetic lifecycle event, bounded process/log checks, and immediate rollback on any sibling-triggered turn.
