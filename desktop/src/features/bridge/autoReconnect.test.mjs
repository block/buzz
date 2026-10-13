import assert from "node:assert/strict";
import test from "node:test";
import {
  AutoReconnector,
  backoffDelay,
  RECONNECT_CAP_MS,
} from "./autoReconnect.ts";

test("backoff doubles from 1 s with ±25% jitter and caps at 30 s", () => {
  assert.equal(
    backoffDelay(0, () => 0.5),
    1000,
  );
  assert.equal(
    backoffDelay(3, () => 0.5),
    8000,
  );
  assert.equal(
    backoffDelay(0, () => 0),
    750,
  );
  assert.equal(
    backoffDelay(0, () => 1),
    1250,
  );
  for (let n = 5; n < 40; n++)
    assert.ok(backoffDelay(n, () => 1) <= RECONNECT_CAP_MS);
});

test("retries until an attempt succeeds, then stops", async () => {
  let calls = 0;
  const r = new AutoReconnector(async () => ++calls >= 3);
  r.start();
  assert.ok(r.active);
  for (let i = 0; i < 3; i++) {
    r.now();
    await new Promise((res) => setTimeout(res, 5));
  }
  assert.equal(calls, 3);
  assert.equal(r.active, false);
  assert.equal(r.failingSince, 0);
  r.stop();
});
