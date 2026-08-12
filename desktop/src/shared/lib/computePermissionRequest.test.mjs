/**
 * Named test matrix for `computePermissionRequest` and `selectProseOrPermission`.
 *
 * Fixtures use the frozen schema (event b31c716e).
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  computePermissionRequest,
  selectProseOrPermission,
} from "./computePermissionRequest.ts";

// ── Fixtures ──────────────────────────────────────────────────────────────────

const AGENT_PUBKEY =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const ATTACKER_PUBKEY =
  "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
const OWNER_PUBKEY =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

const PENDING_PAYLOAD = {
  v: 1,
  state: "pending",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9999999999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  hasDurableRule: false,
  durableRuleNote: null,
};

const RESOLVED_PAYLOAD = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId:
    "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9999999999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  hasDurableRule: false,
  durableRuleNote: null,
  outcome: "applied",
  chosenOptionId: "opt-allow",
};

// Wire contract: the harness signs bare JSON as the kind:9 event content.
// computePermissionRequest receives the raw event content string — no fence.
function raw(payload) {
  return JSON.stringify(payload);
}

// ── computePermissionRequest ──────────────────────────────────────────────────

test("test_not_interactive_returns_null", () => {
  assert.equal(
    computePermissionRequest(
      raw(PENDING_PAYLOAD),
      false,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
    ),
    null,
  );
});

test("test_missing_agentPubkey_returns_null", () => {
  assert.equal(
    computePermissionRequest(
      raw(PENDING_PAYLOAD),
      true,
      undefined,
      AGENT_PUBKEY,
    ),
    null,
  );
});

test("test_missing_signerPubkey_returns_null", () => {
  assert.equal(
    computePermissionRequest(
      raw(PENDING_PAYLOAD),
      true,
      AGENT_PUBKEY,
      undefined,
    ),
    null,
  );
});

test("test_forged_card_wrong_signer_returns_null", () => {
  // agentPubkey (channel's known agent) ≠ signerPubkey (event signer)
  assert.equal(
    computePermissionRequest(
      raw(PENDING_PAYLOAD),
      true,
      AGENT_PUBKEY,
      ATTACKER_PUBKEY,
    ),
    null,
  );
});

test("test_valid_signer_returns_payload", () => {
  const result = computePermissionRequest(
    raw(PENDING_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
  );
  assert.deepEqual(result, PENDING_PAYLOAD);
});

test("test_signer_check_is_case_insensitive", () => {
  const result = computePermissionRequest(
    raw(PENDING_PAYLOAD),
    true,
    AGENT_PUBKEY.toUpperCase(),
    AGENT_PUBKEY.toLowerCase(),
  );
  assert.deepEqual(result, PENDING_PAYLOAD);
});

test("test_no_sentinel_returns_null", () => {
  assert.equal(
    computePermissionRequest(
      "No sentinel here",
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
    ),
    null,
  );
});

test("test_agent_signed_edit_resolves_card", () => {
  const result = computePermissionRequest(
    raw(RESOLVED_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY, // original event signer
    AGENT_PUBKEY, // edit signer == agent ✓
  );
  assert.deepEqual(result, RESOLVED_PAYLOAD);
});

test("test_owner_signed_edit_does_not_resolve", () => {
  assert.equal(
    computePermissionRequest(
      raw(RESOLVED_PAYLOAD),
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      OWNER_PUBKEY, // edit signer is owner, not agent ✗
    ),
    null,
  );
});

test("test_attacker_signed_edit_does_not_resolve", () => {
  assert.equal(
    computePermissionRequest(
      raw(RESOLVED_PAYLOAD),
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      ATTACKER_PUBKEY, // attacker edit ✗
    ),
    null,
  );
});

test("test_resolved_body_with_no_edit_arrived_parses_body_directly", () => {
  // When editSignerPubkey is undefined, no edit-authenticity check runs.
  // If the original event body happened to contain a resolved sentinel, we
  // return it. This handles the edge case where the edit arrives before we
  // query the original event.
  const result = computePermissionRequest(
    raw(RESOLVED_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
    undefined,
  );
  assert.deepEqual(result, RESOLVED_PAYLOAD);
});

// ── selectProseOrPermission ───────────────────────────────────────────────────

test("test_selectProseOrPermission_returns_markdown_when_no_request", () => {
  const node = "markdown-node";
  assert.equal(selectProseOrPermission(null, node), node);
});

test("test_selectProseOrPermission_returns_null_when_request_present", () => {
  // Pass a typed object directly (not parsed from content)
  assert.equal(selectProseOrPermission(PENDING_PAYLOAD, "markdown-node"), null);
});

// ── Component behavior — pure-function coverage ───────────────────────────────
// These test the underlying pure logic for behaviors that manifest in the
// React component. Component state (double-click guard, countdown UI) is
// not testable without a DOM renderer.

test("test_non_owner_viewer_gets_payload_but_is_owner_false", () => {
  // computePermissionRequest returns the payload for any authenticated viewer;
  // isOwner is determined by the caller (PermissionRequestCardBlock) comparing
  // viewerPubkey to ownerPubkey. Verify the payload is returned so the card
  // renders, then the test documents that a non-owner sees it as read-only.
  const result = computePermissionRequest(
    raw(PENDING_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
  );
  assert.ok(result !== null, "payload returned for authenticated render");
  // isOwner=false would be computed by PermissionRequestCardBlock when
  // viewerPubkey !== ownerPubkey — card renders in read-only mode (no buttons).
});

test("test_replay_archive_resolved_state_returns_resolved_payload", () => {
  // Simulates archive/replay: the message body carries resolved payload
  // (edit already applied), agentPubkey present, editSignerPubkey absent.
  // computePermissionRequest must return the resolved payload — the card
  // renders in non-actionable archived state.
  const result = computePermissionRequest(
    raw(RESOLVED_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
    undefined, // no separate edit event needed in archive — body is resolved
  );
  assert.deepEqual(result, RESOLVED_PAYLOAD);
  assert.equal(result?.state, "resolved");
});

test("test_expiry_field_is_preserved_for_local_disable", () => {
  // computePermissionRequest preserves the expiresAt field so the card's
  // PermissionButtons component can compare it to Date.now() / 1000 and
  // disable buttons locally when the harness deadline has passed.
  const result = computePermissionRequest(
    raw(PENDING_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
  );
  assert.ok(result !== null);
  assert.equal(result.expiresAt, 9999999999);
  // Buttons disable when expiresAt <= Date.now()/1000. Since 9999999999 is
  // far in the future, buttons would be enabled. A past value would disable them.
  assert.ok(
    result.expiresAt > Date.now() / 1000,
    "far-future expiresAt stays enabled",
  );
});

test("test_past_expiresAt_parsed_without_rejection", () => {
  // The parser accepts any finite expiresAt (past or future) — expiry is
  // enforced by the component at render time, not at parse time.
  const expired = { ...PENDING_PAYLOAD, expiresAt: 1 }; // Unix epoch + 1s (past)
  const result = computePermissionRequest(
    raw(expired),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
  );
  assert.ok(
    result !== null,
    "past expiresAt is valid — expiry enforced at render",
  );
  assert.equal(result.expiresAt, 1);
});
