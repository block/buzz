import assert from "node:assert/strict";
import test from "node:test";

import {
  computeResumeSince,
  RESUME_LAST_SEEN_SKEW_SECS,
  RESUME_LIVE_UNTIL_SKEW_SECS,
  stampLiveUntilOnClose,
} from "./relayResumeCursor.ts";

test("resume skews match the shared reconnect contract", () => {
  assert.equal(RESUME_LAST_SEEN_SKEW_SECS, 5);
  assert.equal(RESUME_LIVE_UNTIL_SKEW_SECS, 60);
});

// since = max(lastSeen - 5, liveUntil - 60); the original filter's own `since`
// is applied by the replay path and covered in relayReconnectReplay.test.mjs.
for (const [name, input, expected] of [
  ["no cursor replays the original filter", {}, undefined],
  ["lastSeen only", { lastSeenCreatedAt: 1_000 }, 995],
  ["liveUntil only", { liveUntil: 1_000 }, 940],
  ["newer lastSeen wins", { lastSeenCreatedAt: 1_000, liveUntil: 900 }, 995],
  ["newer liveUntil wins", { lastSeenCreatedAt: 900, liveUntil: 1_000 }, 940],
  [
    "a future-dated lastSeen is still the lower bound",
    { lastSeenCreatedAt: 2_000, liveUntil: 1_000 },
    1_995,
  ],
  ["never negative", { lastSeenCreatedAt: 3, liveUntil: 10 }, 0],
  [
    "custom skews apply to each term",
    {
      lastSeenCreatedAt: 1_000,
      liveUntil: 1_000,
      lastSeenSkewSecs: 100,
      liveUntilSkewSecs: 200,
    },
    900,
  ],
]) {
  test(`computeResumeSince: ${name}`, () => {
    assert.equal(computeResumeSince(input), expected);
  });
}

function live(overrides = {}) {
  return { mode: "live", filter: { kinds: [9], limit: 1 }, ...overrides };
}

test("stampLiveUntilOnClose stamps only subscriptions that reached EOSE on the closing generation", () => {
  const eosed = live({ eoseGeneration: 7 });
  const otherGeneration = live({ eoseGeneration: 6, liveUntil: 500 });
  const never = live();
  const history = { mode: "history", eoseGeneration: 7 };
  const subscriptions = new Map([
    ["eosed", eosed],
    ["older", otherGeneration],
    ["never", never],
    ["history", history],
  ]);

  stampLiveUntilOnClose(subscriptions, 7, 0, 1_234_999);

  assert.equal(eosed.liveUntil, 1_234);
  assert.equal(
    otherGeneration.liveUntil,
    500,
    "a subscription that missed EOSE on this connection keeps its old stamp",
  );
  assert.equal(never.liveUntil, undefined);
  assert.equal(history.liveUntil, undefined);
  for (const subscription of [eosed, otherGeneration, never]) {
    assert.equal(subscription.eoseGeneration, undefined);
  }
});

test("stampLiveUntilOnClose never moves liveUntil backwards", () => {
  const subscription = live({ eoseGeneration: 3, liveUntil: 9_999 });
  stampLiveUntilOnClose(new Map([["a", subscription]]), 3, 0, 5_000_000);
  assert.equal(subscription.liveUntil, 9_999);
});

test("stampLiveUntilOnClose caps the stamp at the last inbound frame of a stalled socket", () => {
  const subscription = live({ eoseGeneration: 3 });
  // Stall detected 70s after the last frame: the close time overstates liveness.
  stampLiveUntilOnClose(
    new Map([["a", subscription]]),
    3,
    1_000_000,
    1_070_000,
  );
  assert.equal(subscription.liveUntil, 1_000);
});
