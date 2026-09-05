/**
 * Tests for `selectPermissionRequest` (primary) and `hasPermissionRequestCard`
 * (legacy delegate).
 *
 * `selectPermissionRequest` is the single source of truth for prose suppression
 * in `MessageRow` — it incorporates channelId + isAgent eligibility AND calls
 * `computePermissionRequest` to produce the trusted payload the card block
 * renders. Non-null iff a card will render.
 *
 * This closes every blank-row case:
 *   - falsy channelId → no card → no prose suppression
 *   - !message.isAgent → no card → no prose suppression
 *   - forged signer (signerPubkey ≠ agentPubkey) → computePermissionRequest null
 *   - born-resolved-no-provenance → computePermissionRequest null
 *   - correlation-mismatch resolved body → computePermissionRequest null
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";

const mod = await import("./permissionRequestAuthPubkey.js").catch(
  () => import("./permissionRequestAuthPubkey.ts"),
);
const {
  getPermissionRequestAgentPubkey,
  hasPermissionRequestCard,
  selectPermissionRequest,
} = mod;

// ── Fixtures ──────────────────────────────────────────────────────────────────

const AGENT_PUBKEY =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
const ATTACKER_PUBKEY =
  "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

// A valid 64-char hex event ID used as the sentinel's own ID.
const MESSAGE_ID =
  "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const OTHER_ID =
  "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";

const CHANNEL_ID = "chan-test-001";

const PENDING_BODY = JSON.stringify({
  v: 1,
  state: "pending",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9_999_999_999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
});

// A valid resolved body that correlates to MESSAGE_ID + PENDING_BODY nonce.
const RESOLVED_BODY = JSON.stringify({
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId: MESSAGE_ID,
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 9_999_999_999,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  outcome: "applied",
  chosenOptionId: "opt-allow",
});

const PROSE_BODY = "Hello from the agent";

function makePendingMessage(overrides = {}) {
  return {
    kind: 9,
    isAgent: true,
    signerPubkey: AGENT_PUBKEY,
    body: PENDING_BODY,
    id: MESSAGE_ID,
    editSignerPubkey: undefined,
    preEditBody: undefined,
    ...overrides,
  };
}

function isKnownAgent(pubkey) {
  return pubkey === AGENT_PUBKEY;
}

// ── getPermissionRequestAgentPubkey ───────────────────────────────────────────

describe("getPermissionRequestAgentPubkey", () => {
  it("test_returns_signer_pubkey_for_known_agent_on_kind9", () => {
    const msg = makePendingMessage();
    assert.equal(
      getPermissionRequestAgentPubkey(msg, isKnownAgent),
      AGENT_PUBKEY,
    );
  });

  it("test_returns_undefined_for_unknown_signer", () => {
    const msg = makePendingMessage({ signerPubkey: ATTACKER_PUBKEY });
    assert.equal(getPermissionRequestAgentPubkey(msg, isKnownAgent), undefined);
  });

  it("test_returns_undefined_for_non_kind9", () => {
    const msg = makePendingMessage({ kind: 1 });
    assert.equal(getPermissionRequestAgentPubkey(msg, isKnownAgent), undefined);
  });
});

// ── selectPermissionRequest ───────────────────────────────────────────────────
//
// These are the authoritative tests for the prose-suppression gate.
// selectPermissionRequest folds in channelId + isAgent eligibility so the
// result can be used directly in MessageRow without any secondary check.

describe("selectPermissionRequest", () => {
  it("test_returns_selection_for_trusted_pending_sentinel", () => {
    const msg = makePendingMessage();
    const sel = selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID);
    assert.ok(sel !== null, "trusted pending sentinel must return a selection");
    assert.equal(sel.agentPubkey, AGENT_PUBKEY);
    assert.equal(sel.request.state, "pending");
  });

  it("test_returns_null_for_null_channel_id", () => {
    // channelId=null means no card → no prose suppression.
    const msg = makePendingMessage();
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, null),
      null,
      "null channelId must return null — no card will render",
    );
  });

  it("test_returns_null_for_undefined_channel_id", () => {
    const msg = makePendingMessage();
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, undefined),
      null,
      "undefined channelId must return null",
    );
  });

  it("test_returns_null_for_empty_string_channel_id", () => {
    const msg = makePendingMessage();
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, ""),
      null,
      "empty-string channelId must return null (falsy)",
    );
  });

  it("test_returns_null_when_isAgent_false", () => {
    // !isAgent → no card → no prose suppression.
    const msg = makePendingMessage({ isAgent: false });
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID),
      null,
      "non-agent message must return null",
    );
  });

  it("test_returns_null_for_forged_signer_prose_not_suppressed", () => {
    // F3: forged signer — valid sentinel JSON but wrong signer.
    // computePermissionRequest rejects on the D1 signer gate.
    // Prose must NOT be suppressed — fallback to markdown.
    const msg = makePendingMessage({ signerPubkey: ATTACKER_PUBKEY });
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID),
      null,
      "forged signer must return null — prose must not be suppressed",
    );
  });

  it("test_returns_null_for_prose_body_even_with_known_agent", () => {
    const msg = makePendingMessage({ body: PROSE_BODY });
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID),
      null,
      "non-sentinel body must not suppress prose",
    );
  });

  it("test_returns_null_for_born_resolved_no_provenance", () => {
    // Born-resolved-no-provenance: body is already "resolved" but has no edit
    // provenance. computePermissionRequest rejects — no edit signer present.
    const msg = makePendingMessage({
      body: RESOLVED_BODY,
      editSignerPubkey: undefined,
      id: MESSAGE_ID,
      preEditBody: undefined,
    });
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID),
      null,
      "born-resolved without edit provenance must return null",
    );
  });

  it("test_returns_selection_for_resolved_with_valid_provenance", () => {
    const msg = makePendingMessage({
      body: RESOLVED_BODY,
      editSignerPubkey: AGENT_PUBKEY,
      id: MESSAGE_ID,
      preEditBody: PENDING_BODY,
    });
    const sel = selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID);
    assert.ok(
      sel !== null,
      "resolved with valid provenance must return selection",
    );
    assert.equal(sel.request.state, "resolved");
  });

  it("test_returns_null_for_correlation_mismatch_resolved", () => {
    const mismatchedBody = JSON.stringify({
      v: 1,
      state: "resolved",
      requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
      originalEventId: OTHER_ID, // ← different from MESSAGE_ID
      sessionId: "sess-abc",
      turnId: "turn-xyz",
      expiresAt: 9_999_999_999,
      optionIds: ["opt-allow", "opt-deny"],
      labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
      outcome: "applied",
      chosenOptionId: "opt-allow",
    });
    const msg = makePendingMessage({
      body: mismatchedBody,
      editSignerPubkey: AGENT_PUBKEY,
      id: MESSAGE_ID,
      preEditBody: PENDING_BODY,
    });
    assert.equal(
      selectPermissionRequest(msg, isKnownAgent, CHANNEL_ID),
      null,
      "correlation-mismatch resolved must return null",
    );
  });
});

// ── hasPermissionRequestCard ──────────────────────────────────────────────────
// Legacy boolean delegate — kept for coverage. Tests use isAgent: true because
// the function now requires it (delegates to selectPermissionRequest path).

describe("hasPermissionRequestCard", () => {
  it("test_returns_true_for_trusted_agent_pending_sentinel", () => {
    const msg = makePendingMessage();
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      true,
      "trusted pending sentinel must return true",
    );
  });

  it("test_returns_false_for_forged_signer_prose_not_suppressed", () => {
    const msg = makePendingMessage({ signerPubkey: ATTACKER_PUBKEY });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "forged signer must NOT suppress prose — fallback to markdown",
    );
  });

  it("test_returns_false_for_prose_body_even_with_known_agent", () => {
    const msg = makePendingMessage({ body: PROSE_BODY });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "non-sentinel body must not suppress prose",
    );
  });

  it("test_returns_false_for_unknown_agent", () => {
    const msg = makePendingMessage({ signerPubkey: ATTACKER_PUBKEY });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "unknown agent must not suppress prose",
    );
  });

  it("test_returns_false_for_non_kind9", () => {
    const msg = makePendingMessage({ kind: 1 });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "wrong kind must not suppress prose",
    );
  });

  it("test_returns_false_for_born_resolved_no_provenance", () => {
    const msg = makePendingMessage({
      body: RESOLVED_BODY,
      editSignerPubkey: undefined,
      id: MESSAGE_ID,
      preEditBody: undefined,
    });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "born-resolved sentinel without edit provenance must NOT suppress prose",
    );
  });

  it("test_returns_true_for_resolved_with_valid_provenance", () => {
    const msg = makePendingMessage({
      body: RESOLVED_BODY,
      editSignerPubkey: AGENT_PUBKEY,
      id: MESSAGE_ID,
      preEditBody: PENDING_BODY,
    });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      true,
      "resolved sentinel with valid edit provenance must suppress prose",
    );
  });

  it("test_returns_false_for_correlation_mismatch_resolved", () => {
    const mismatchedBody = JSON.stringify({
      v: 1,
      state: "resolved",
      requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
      originalEventId: OTHER_ID, // ← different from MESSAGE_ID
      sessionId: "sess-abc",
      turnId: "turn-xyz",
      expiresAt: 9_999_999_999,
      optionIds: ["opt-allow", "opt-deny"],
      labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
      outcome: "applied",
      chosenOptionId: "opt-allow",
    });
    const msg = makePendingMessage({
      body: mismatchedBody,
      editSignerPubkey: AGENT_PUBKEY,
      id: MESSAGE_ID,
      preEditBody: PENDING_BODY,
    });
    assert.equal(
      hasPermissionRequestCard(msg, isKnownAgent),
      false,
      "correlation-mismatch resolved body must NOT suppress prose",
    );
  });
});
