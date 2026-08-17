import assert from "node:assert/strict";
import test from "node:test";

import { refreshChannelsWhenIdle } from "./refreshChannelsWhenIdle.ts";

function spy() {
  const fn = () => {
    fn.calls++;
  };
  fn.calls = 0;
  return fn;
}

test("invalidates when no fetch is in flight", () => {
  const invalidate = spy();
  const reArm = spy();
  refreshChannelsWhenIdle({ isFetching: () => 0, invalidate, reArm });
  assert.equal(invalidate.calls, 1);
  assert.equal(reArm.calls, 0);
});

test("re-arms instead of invalidating while a fetch is in flight", () => {
  const invalidate = spy();
  const reArm = spy();
  refreshChannelsWhenIdle({ isFetching: () => 1, invalidate, reArm });
  assert.equal(
    invalidate.calls,
    0,
    "must not invalidate mid-flight (would drop the dirty signal)",
  );
  assert.equal(reArm.calls, 1);
});

test("re-arm then idle eventually invalidates exactly once", () => {
  const invalidate = spy();
  const reArm = spy();
  // First pass: fetch in flight -> re-arm.
  let fetching = 1;
  const deps = {
    isFetching: () => fetching,
    invalidate,
    // Simulate the debounce re-firing the same action.
    reArm: () => {
      reArm.calls++;
      refreshChannelsWhenIdle(deps);
    },
  };
  // Once re-armed, the fetch has finished.
  reArm.calls = 0;
  const reArmOnce = deps.reArm;
  deps.reArm = () => {
    fetching = 0;
    reArmOnce();
  };

  refreshChannelsWhenIdle(deps);
  assert.equal(reArm.calls, 1, "re-armed once while fetching");
  assert.equal(invalidate.calls, 1, "invalidated once after going idle");
});
