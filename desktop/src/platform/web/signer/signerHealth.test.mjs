import assert from "node:assert/strict";
import test from "node:test";
import { BANNER_SILENCE_MS, ESCALATE_WAIT_MS, UNRESPONSIVE_AFTER, isRevokedAnswer, signerHealth } from "./signerHealth.ts";

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
  signerHealth.escalate(); // signer answered moments ago: a long wait alone isn't "unresponsive"
  assert.equal(signerHealth.get().state, "ok");
  const realNow = Date.now;
  Date.now = () => realNow() + BANNER_SILENCE_MS + 1;
  try {
    signerHealth.escalate();
  } finally {
    Date.now = realNow;
  }
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

test("background timeouts never escalate; hidden page pauses counting and resume starts clean", () => {
  signerHealth.reset();
  for (let i = 0; i < 10; i++) signerHealth.timeout({ background: true });
  assert.equal(signerHealth.get().state, "slow");
  signerHealth.reset();
  signerHealth.setVisible(false);
  for (let i = 0; i < 10; i++) signerHealth.timeout();
  assert.equal(signerHealth.get().state, "ok");
  signerHealth.timeout();
  signerHealth.setVisible(true);
  assert.equal(signerHealth.get().state, "ok");
  signerHealth.error("unauthorized: session revoked");
  signerHealth.setVisible(false);
  signerHealth.setVisible(true);
  assert.equal(signerHealth.get().reason, "revoked");
  signerHealth.reset();
});
