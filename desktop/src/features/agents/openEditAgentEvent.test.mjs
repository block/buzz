import assert from "node:assert/strict";
import test from "node:test";

// Provide a minimal window shim for the module's DOM event calls.
// Node 25 has CustomEvent but not window.addEventListener / dispatchEvent.
const _eventTarget = new EventTarget();
globalThis.window = {
  addEventListener: _eventTarget.addEventListener.bind(_eventTarget),
  removeEventListener: _eventTarget.removeEventListener.bind(_eventTarget),
  dispatchEvent: _eventTarget.dispatchEvent.bind(_eventTarget),
};

import {
  consumePendingOpenEditAgent,
  requestOpenEditAgent,
  subscribeOpenEditAgent,
} from "./openEditAgentEvent.ts";

// ── consumePendingOpenEditAgent ───────────────────────────────────────────────

test("consumePendingOpenEditAgent_noPriorRequest_returnsFalse", () => {
  // No request has been made for this pubkey — consume must return false.
  assert.equal(
    consumePendingOpenEditAgent("aabbccddeeff0011"),
    false,
    "consume with no pending request must return false",
  );
});

test("consumePendingOpenEditAgent_afterRequest_returnsTrue", () => {
  const pubkey = "ddeeff00112233aa";
  requestOpenEditAgent(pubkey);
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    true,
    "consume immediately after request (no focus) must return true",
  );
});

test("consumePendingOpenEditAgent_afterRequest_clearsState", () => {
  const pubkey = "112233aabbccddee";
  requestOpenEditAgent(pubkey);
  consumePendingOpenEditAgent(pubkey);
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    false,
    "second consume must return false — state cleared by first",
  );
});

test("consumePendingOpenEditAgent_wrongPubkey_returnsFalse", () => {
  const pubkey = "aabbcc001122ddef";
  requestOpenEditAgent(pubkey);
  const result = consumePendingOpenEditAgent("ffffffffffffffff");
  consumePendingOpenEditAgent(pubkey); // clean up
  assert.equal(
    result,
    false,
    "consume with non-matching pubkey must return false",
  );
});

// ── focus-target round-trip ───────────────────────────────────────────────────

test("consumePendingOpenEditAgent_withEnvKeyFocus_returnsFocusTarget", () => {
  const pubkey = "aa11bb22cc33dd44";
  requestOpenEditAgent(pubkey, { type: "env_key", key: "ANTHROPIC_API_KEY" });
  const result = consumePendingOpenEditAgent(pubkey);
  assert.deepEqual(
    result,
    { type: "env_key", key: "ANTHROPIC_API_KEY" },
    "consume after env_key request must return the focus target",
  );
});

test("consumePendingOpenEditAgent_withNormalizedFieldFocus_returnsFocusTarget", () => {
  const pubkey = "bb22cc33dd44ee55";
  requestOpenEditAgent(pubkey, { type: "normalized_field", field: "provider" });
  const result = consumePendingOpenEditAgent(pubkey);
  assert.deepEqual(
    result,
    { type: "normalized_field", field: "provider" },
    "consume after normalized_field request must return the focus target",
  );
});

test("consumePendingOpenEditAgent_focusTarget_clearedAfterConsume", () => {
  const pubkey = "cc33dd44ee55ff66";
  requestOpenEditAgent(pubkey, { type: "env_key", key: "OPENAI_API_KEY" });
  consumePendingOpenEditAgent(pubkey);
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    false,
    "focus target must be cleared along with pubkey on consume",
  );
});

// ── pending-before-mount (consume) + focus ────────────────────────────────────

test("consumePendingOpenEditAgent_pendingBeforeMount_withFocus_returnsTarget", () => {
  // Simulates the panel mounting AFTER requestOpenEditAgent was dispatched
  // (i.e., no live subscriber was registered when the event fired).
  const pubkey = "dd44ee55ff66aa77";
  requestOpenEditAgent(pubkey, { type: "env_key", key: "DATABRICKS_HOST" });
  // Panel mounts now — no subscriber was present, so it calls consume.
  const result = consumePendingOpenEditAgent(pubkey);
  assert.deepEqual(
    result,
    { type: "env_key", key: "DATABRICKS_HOST" },
    "pending-before-mount consume must return the queued focus target",
  );
});

// ── subscribeOpenEditAgent — live subscriber clears pending ───────────────────

test("subscribeOpenEditAgent_afterLiveHandle_consumeReturnsFalse", () => {
  // Core Fix 2 invariant: after a live subscriber handles the event,
  // consumePendingOpenEditAgent must return false (pending cleared).
  const pubkey = "66778899aabbccdd";
  let handlerCalled = false;
  let receivedFocus = /** @type {unknown} */ ("not-called");

  const unsubscribe = subscribeOpenEditAgent(pubkey, (focus) => {
    handlerCalled = true;
    receivedFocus = focus;
  });

  requestOpenEditAgent(pubkey); // fires synchronously via dispatchEvent

  unsubscribe();

  assert.equal(handlerCalled, true, "handler must have been called");
  assert.equal(
    receivedFocus,
    undefined,
    "focus must be undefined when not passed",
  );
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    false,
    "pending must be cleared by live subscriber — not by consume",
  );
});

test("subscribeOpenEditAgent_withFocusTarget_handlerReceivesFocus", () => {
  const pubkey = "7788990011aabbcc";
  let receivedFocus = /** @type {unknown} */ ("not-called");

  const unsubscribe = subscribeOpenEditAgent(pubkey, (focus) => {
    receivedFocus = focus;
  });

  requestOpenEditAgent(pubkey, { type: "env_key", key: "OPENAI_API_KEY" });

  unsubscribe();

  assert.deepEqual(
    receivedFocus,
    { type: "env_key", key: "OPENAI_API_KEY" },
    "live subscriber must receive the focus target from the event",
  );
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    false,
    "pending must be cleared by live subscriber",
  );
});

test("subscribeOpenEditAgent_differentPubkey_doesNotHandle", () => {
  const subscribedPubkey = "aabbccdd11223344";
  const requestedPubkey = "ffffffffffffffff00000000";
  let handlerCalled = false;

  const unsubscribe = subscribeOpenEditAgent(subscribedPubkey, () => {
    handlerCalled = true;
  });

  requestOpenEditAgent(requestedPubkey);
  consumePendingOpenEditAgent(requestedPubkey); // clean up
  unsubscribe();

  assert.equal(
    handlerCalled,
    false,
    "handler must not fire for a different pubkey",
  );
});
