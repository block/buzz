import assert from "node:assert/strict";
import test from "node:test";

import {
  BUSINESS_AUTH_HEARTBEAT_INTERVAL_MS,
  startBusinessAuthHeartbeat,
} from "./businessAuthHeartbeat.ts";

test("polls only while the authenticated Dock heartbeat is enabled", () => {
  let callback;
  let cleared;
  const scheduler = {
    setInterval(next, delay) {
      callback = next;
      assert.equal(delay, BUSINESS_AUTH_HEARTBEAT_INTERVAL_MS);
      return 42;
    },
    clearInterval(id) {
      cleared = id;
    },
  };
  let checks = 0;
  const stop = startBusinessAuthHeartbeat(
    true,
    () => {
      checks += 1;
    },
    scheduler,
  );
  callback();
  assert.equal(checks, 1);
  stop();
  assert.equal(cleared, 42);
});

test("does not allocate an interval while disabled", () => {
  let allocated = false;
  const scheduler = {
    setInterval() {
      allocated = true;
      return 1;
    },
    clearInterval() {},
  };
  startBusinessAuthHeartbeat(false, () => undefined, scheduler)();
  assert.equal(allocated, false);
});
