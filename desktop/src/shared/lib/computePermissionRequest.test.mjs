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

// The kind-9 sentinel event ID. A resolved edit must name this in
// `originalEventId` (F5 correlation).
const MESSAGE_ID =
  "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead";

const PENDING_PAYLOAD = {
  v: 1,
  state: "pending",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9999999999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
};

const RESOLVED_PAYLOAD = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId: MESSAGE_ID,
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9999999999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
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
    MESSAGE_ID, // originalEventId names this card ✓
    raw(PENDING_PAYLOAD), // nonce/session/turn correlate ✓
  );
  assert.deepEqual(result, RESOLVED_PAYLOAD);
});

test("test_resolved_edit_with_mismatched_originalEventId_returns_null", () => {
  // F5: same-signer agent edit, but originalEventId names a DIFFERENT card.
  assert.equal(
    computePermissionRequest(
      raw(RESOLVED_PAYLOAD),
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      "0000000000000000000000000000000000000000000000000000000000000000",
      raw(PENDING_PAYLOAD),
    ),
    null,
  );
});

test("test_resolved_edit_with_mismatched_nonce_returns_null", () => {
  // F5: originalEventId matches, but the resolved nonce does not correlate
  // to the pending body the edit overlaid — a cross-applied resolution.
  const pendingOther = { ...PENDING_PAYLOAD, requestNonce: "different-nonce" };
  assert.equal(
    computePermissionRequest(
      raw(RESOLVED_PAYLOAD),
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      MESSAGE_ID,
      raw(pendingOther),
    ),
    null,
  );
});

test("test_resolved_edit_without_pending_body_returns_null", () => {
  // F5: an arrived edit with no retained pending body cannot be correlated.
  assert.equal(
    computePermissionRequest(
      raw(RESOLVED_PAYLOAD),
      true,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      AGENT_PUBKEY,
      MESSAGE_ID,
      undefined,
    ),
    null,
  );
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

test("test_born_resolved_body_without_edit_provenance_returns_null", () => {
  // A kind-9 whose content is *born* `resolved` (no kind-40003 edit overlaid)
  // carries no edit provenance: editSignerPubkey is undefined. Such a payload
  // must NOT render as a completed card — it would pass the D1 signer gate
  // alone with zero evidence of an edit, matching original event, or matching
  // nonce/session/turn. Mutation proof: relaxing the resolved-state guard to
  // run only when editSignerPubkey is non-null turns this red.
  const result = computePermissionRequest(
    raw(RESOLVED_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
    undefined, // no edit arrived → no provenance
  );
  assert.equal(result, null);
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

test("test_replay_with_edit_provenance_returns_resolved_payload", () => {
  // Archive/replay of a resolved card: `formatTimelineMessages` overlays the
  // kind-40003 edit onto the pending kind-9 and supplies editSignerPubkey,
  // messageId, and preEditContent together (formatTimelineMessages.ts:526-528).
  // With full provenance the resolved payload renders in non-actionable state.
  const result = computePermissionRequest(
    raw(RESOLVED_PAYLOAD),
    true,
    AGENT_PUBKEY,
    AGENT_PUBKEY,
    AGENT_PUBKEY, // edit signer supplied on replay ✓
    MESSAGE_ID, // originalEventId names this card ✓
    raw(PENDING_PAYLOAD), // pre-edit pending body correlates ✓
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
