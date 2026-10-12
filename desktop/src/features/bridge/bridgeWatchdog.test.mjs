import test from "node:test";
import assert from "node:assert/strict";

test("watchdog: resubscribes after >45s without a heartbeat while visible, not when hidden or just kicked", async () => {
  const { shouldKickBridge } = await import("./bridgeWatchdog.ts");
  const base = { visible: true, keyFailed: false, now: 1000, lastKick: 0 };
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: 990 }), false);
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: 950 }), true); // relay dropped / Mac slept: 50s silent
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: null }), true);
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: 900, visible: false }), false);
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: 900, lastKick: 990 }), false);
  assert.equal(shouldKickBridge({ ...base, lastHeardTs: 999, keyFailed: true }), true); // stuck key decrypt is retried
});

test("seq guard: a desktop reload (counter restarts under the same key) is accepted when the envelope is newer; replays are not", async () => {
  const { SeqGuard } = await import("./protocol.ts");
  const g = new SeqGuard();
  const env = (seq, ts) => ({ dev: "d", kid: "k", type: "presence.heartbeat", seq, ts });
  assert.equal(g.accept(env(50, 1000)), true);
  assert.equal(g.accept(env(51, 1030)), true);
  assert.equal(g.accept(env(51, 1030)), false); // replay
  assert.equal(g.accept(env(1, 1100)), true); // restarted publisher, newer time
  assert.equal(g.accept(env(1, 1100)), false);
  assert.equal(g.accept(env(40, 1050)), false); // older than the last accepted
});
