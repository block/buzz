# Conversation thinking level

An owner can change the thinking level of an existing conversation from the
agent activity pane. A running response finishes at its current level; the next
response in that same ACP session uses the selected level. Conversation history
and saved defaults are preserved. If several conversations are visible, the
owner chooses one before choosing a level.

This is the live-control follow-up deferred in #4557. It does not change saved
agent settings, model discovery, or portable exports covered by #5016.

## Authority and lifecycle

The harness captures native `thought_level` select options after session setup
and identifies the exact channel and ACP session in the observer envelope. The
snapshot advertises `liveEffortSwitching` only when it fits the worker cache
(128 snapshots, at most 256 KiB each); older harnesses and sessions without
native options have no live picker. The frontend retains at most 128 small
session-config snapshots per agent independently of transcript eviction. A new
session for the same conversation supersedes the previous selection.

`switch_effort` uses the existing encrypted, signed owner observer-control route
and its five-minute freshness check. Requests include `channelId`, `sessionId`,
`requestId`, `sessionToken` and `effort`. The per-session token comes from the
harness snapshot and changes on recreation, preventing a stale request from
editing a successor when an adapter reuses session IDs. The harness discovers the native configuration ID from
the session snapshot, validates supported values, and sends
`session/set_config_option` only while the owning worker is idle. It never
cancels a prompt, recreates the session, or changes `startup_effort` on success.
A pending edit fences that worker before its next claim. Other workers remain
available. A possible busy sibling must return before target resolution because
some adapters use process-local session IDs; ambiguous targets are rejected.

The queue allows 32 pending requests, one per exact session. Up to 256 receipts
are retained for ten minutes, covering the full accepted replay window without
evicting receipts early. Edits expire after five minutes; a native RPC has a
five-second deadline, with at most one RPC per main-loop iteration. A transport
failure retires the affected worker so a poisoned stream cannot serve a new
turn; normal pool maintenance replaces it. Its session is then lost and the UI
reports the change as unconfirmed, never applied.

`queued` means accepted for later application. `applied` requires a returned
native `currentValue` matching the request. Rejection, unsupported or stale
sessions, capacity, expiration, and missing confirmation remain distinct
outcomes. The UI correlates all five request fields and subscribes before
publishing. It retains the reported value while queued and explains missing
acknowledgment or confirmation rather than claiming success from relay delivery.

## Verification

`cargo test -p buzz-acp --lib live_effort` exercises the real queue and ACP stdio
boundary. Desktop unit tests cover snapshot retention and receipt correlation;
`conversation-effort.spec.ts` covers the owner activity workflow through the
mock relay bridge. Existing activity-control tests guard adjacent Stop and
model controls.

The ignored `real_codex_preserves_conversation_across_queued_effort_change` test
performs two actual provider turns. Set `BUZZ_TEST_CODEX_ADAPTER` to an installed
adapter, `CODEX_HOME` to an already-authenticated profile, and optionally
`BUZZ_TEST_CODEX_MODEL` to an advertised model ID supporting Low and High. Run:

```sh
cargo test -p buzz-acp --lib \
  real_codex_preserves_conversation_across_queued_effort_change -- --ignored --nocapture
```

It verifies that a busy edit queues without cancellation, confirms High via the
adapter, and recalls a random word from the first turn in the same session while
the saved startup level remains Low. Provider turn records independently expose
the actual model and effort used by each turn; browser fixtures alone do not
establish provider behavior.
