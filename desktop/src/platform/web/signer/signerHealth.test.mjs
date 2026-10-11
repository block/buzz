import assert from "node:assert/strict";
import test from "node:test";
import { ESCALATE_WAIT_MS, UNRESPONSIVE_AFTER, isRevokedAnswer, signerHealth } from "./signerHealth.ts";

test("timeouts: slow, then unresponsive after N in a row; one success resets", () => {
  signerHealth.reset();
  assert.equal(signerHealth.get().state, "ok");
  signerHealth.timeout();
  assert.equal(signerHealth.get().state, "slow");
  signerHealth.success();
  assert.equal(signerHealth.get().state, "ok");
  for (let i = 0; i < UNRESPONSIVE_AFTER; i++) signerHealth.timeout();
  assert.deepEqual([signerHealth.get().state, signerHealth.get().reason], ["unresponsive", "timeout"]);
  assert.equal(signerHealth.blocksBackground(), true);
  signerHealth.success();
  assert.equal(signerHealth.blocksBackground(), false);
});

test("answers: revoked/unauthorized => unresponsive(revoked); a user 'no' counts as alive", () => {
  signerHealth.reset();
  signerHealth.error("User rejected the request");
  assert.equal(signerHealth.get().state, "ok");
  signerHealth.error("unauthorized: session revoked");
  assert.deepEqual([signerHealth.get().state, signerHealth.get().reason], ["unresponsive", "revoked"]);
  for (const m of ["not connected", "No such session", "unknown client", "Session expired"]) assert.equal(isRevokedAnswer(m), true, m);
  for (const m of ["user denied", "rejected by user", "invalid event kind", ""]) assert.equal(isRevokedAnswer(m), false, m);
});

test("escalation, probe failure, reconnecting and no-session", () => {
  signerHealth.reset();
  assert.ok(ESCALATE_WAIT_MS >= 10_000);
  signerHealth.escalate();
  assert.deepEqual([signerHealth.get().state, signerHealth.get().reason], ["unresponsive", "long-wait"]);
  signerHealth.reset();
  signerHealth.reconnecting(true);
  assert.equal(signerHealth.get().state, "reconnecting");
  signerHealth.reconnecting(false);
  assert.equal(signerHealth.get().state, "ok");
  signerHealth.probeFailed();
  signerHealth.reconnecting(true);
  signerHealth.reconnecting(false);
  assert.equal(signerHealth.get().state, "unresponsive");
  signerHealth.noSession();
  assert.equal(signerHealth.get().state, "no-session");
  assert.equal(signerHealth.blocksBackground(), true);
  let n = 0;
  const off = signerHealth.subscribe(() => n++);
  signerHealth.success();
  signerHealth.success(); // no change, no event
  off();
  assert.equal(n, 1);
});
