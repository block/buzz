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
    "consume immediately after request must return true",
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

// ── subscribeOpenEditAgent — live subscriber clears pending ───────────────────

test("subscribeOpenEditAgent_afterLiveHandle_consumeReturnsFalse", () => {
  // Core Fix 2 invariant: after a live subscriber handles the event,
  // consumePendingOpenEditAgent must return false (pending cleared).
  const pubkey = "66778899aabbccdd";
  let handlerCalled = false;

  const unsubscribe = subscribeOpenEditAgent(pubkey, () => {
    handlerCalled = true;
  });

  requestOpenEditAgent(pubkey); // fires synchronously via dispatchEvent

  unsubscribe();

  assert.equal(handlerCalled, true, "handler must have been called");
  assert.equal(
    consumePendingOpenEditAgent(pubkey),
    false,
    "pending must be cleared by live subscriber — not by consume",
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
