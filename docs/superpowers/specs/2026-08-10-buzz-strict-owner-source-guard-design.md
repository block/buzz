# Buzz Strict-Owner Source Guard Design

## Objective

Separate message visibility from work-trigger authority in `buzz-acp`:

- an event signed by the registered owner may start or steer work when it also matches the active subscription rule;
- an event signed by this agent is ignored;
- an event signed by a same-owner managed sibling does not start or steer work under `respond-to=owner-only`;
- sibling events remain stored, queryable, and renderable as conversation context.

This change is source-and-tests only. It does not deploy a binary, restart Buzz, mutate production configuration, alter Gyre or prloop, or send Buzz traffic.

## Verified Current Path

The relay event loop in `crates/buzz-acp/src/lib.rs` performs these decisions in order:

1. drop self-authored events when `ignore_self` is enabled;
2. recognize owner control commands;
3. call `author_allowed`;
4. call `filter::match_event`;
5. enqueue an accepted event with `EventQueue::push`;
6. allow the accepted event to start a turn or signal an in-flight turn.

`author_allowed` currently implements `RespondTo::OwnerOnly` as owner **or** a same-owner sibling verified through NIP-OA. The executable regression witness `test_owner_only_admits_owner_and_sibling_to_steer` passed before this change with one genuine test executed.

Conversation context takes a separate read path in `crates/buzz-acp/src/pool.rs`: it queries relay history and parses returned events without consulting `author_allowed`. Therefore trigger rejection does not require hiding or deleting sibling messages.

## Identity Basis

- **Owner:** exact event-author pubkey equality with the owner resolved from a cryptographically verified `BUZZ_AUTH_TAG`, falling back to the explicitly configured `BUZZ_ACP_AGENT_OWNER` pubkey.
- **Self:** exact event-author pubkey equality with the public key derived from this harness's signing key.
- **Sibling:** a different pubkey whose kind-0 profile contains a cryptographically verified NIP-OA attestation for the same owner.

The strict-owner decision does not need a successful sibling lookup. It accepts only the exact owner pubkey and rejects every other pubkey fail-closed. NIP-OA sibling discovery remains available to the unchanged `allowlist` and DM-hardening behavior of other response modes.

## Chosen Guard

Correct the existing `RespondTo::OwnerOnly` branch to match its documented contract: exact owner only.

- In public channels, `OwnerOnly` returns true only for the registered owner.
- In DMs, `OwnerOnly` returns true only for the registered owner.
- `Allowlist`, `Anyone`, and `Nobody` retain their current behavior.
- Setup mode inherits the corrected result because it calls the same `author_allowed` function.
- The existing self-authored early drop remains unchanged.
- Subscription matching, queueing, reactions, dispatch, steering, and history queries remain unchanged.

No new configuration mode or environment variable is introduced. The default `respond-to=owner-only` becomes consistent with the existing CLI and README documentation.

## Test Design

All regression tests use real Buzz decision functions and signed Nostr events or real relay-response parsing. They do not use the live relay or synthetic live traffic.

| Requirement | Test boundary |
|---|---|
| A. Owner kind-9 starts work | Exact owner passes `author_allowed`; the signed kind-9 event matches an all-channel kind-9 subscription rule. |
| B. Sibling kind-9 does not start work | Cached verified sibling fails `author_allowed` under `OwnerOnly`, before matching, queueing, or steering. |
| C. Sibling remains readable context | A relay query response containing the sibling pubkey and content is parsed into `ConversationContext` with both fields intact. |
| D. Self is ignored | The existing self-ignore predicate is extracted into a small pure helper used by the event loop and tested for enabled/disabled behavior; strict owner also rejects the distinct self key. |
| E. Lifecycle kinds cannot recurse | Signed sibling events of kinds 5, 7, and 20002 are rejected at the author boundary even against a wildcard subscription that would otherwise match them. |
| F. One owner event cannot produce an unbounded sibling chain | A bounded sequence containing one owner kind-9 event followed by many sibling kind-9 events yields exactly one trigger-eligible event. |

The TDD red run must execute the new tests with a nonzero count and fail because the current `OwnerOnly` branch admits siblings. After the minimal source edit, the same focused tests must pass. The broader gate is the complete `buzz-acp` test suite with explicit test counts.

## Risks and Compatibility

- Deployments that relied on undocumented sibling wakeups while using `owner-only` will stop receiving those automatic triggers. They may still read sibling output from channel history.
- Explicit `allowlist` and `anyone` deployments remain behaviorally unchanged; this avoids silently narrowing unrelated users or channels.
- An absent or invalid owner identity remains fail-closed, so owner messages cannot wake the agent until identity is configured correctly.
- The source guard prevents recursive agent-authored work. Kind filtering remains an independent defense against unrelated event kinds and is not changed here.

## Rollback and Live Boundary

Rollback is the inverse source diff restoring `OwnerOnly` to `is_owner_or_sibling` and removing the new regression tests/helper. The work is isolated on `fix/buzz-strict-owner-source-guard`; the user's existing main-checkout modification is untouched.

After source tests pass, the handoff may propose a bounded live canary. It must not execute that canary, deploy the binary, restart agents, mutate production configuration, or send a Buzz message without explicit authorization.
