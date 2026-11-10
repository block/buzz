import assert from "node:assert/strict";
import test from "node:test";

import {
  STALL_CHECK_INTERVAL_MS,
  STALL_IDLE_TIMEOUT_MS,
} from "./relayClientTimings.ts";
import {
  computeResumeSince,
  creditInboundLiveness,
  LIVENESS_MAX_GAP_MS,
  RESUME_LAST_SEEN_SKEW_SECS,
  RESUME_LIVE_UNTIL_SKEW_SECS,
  RESUME_SILENT_DROP_WINDOW_SECS,
  stampLiveUntilOnClose,
} from "./relayResumeCursor.ts";

test("resume skews match the shared reconnect contract", () => {
  assert.equal(RESUME_LAST_SEEN_SKEW_SECS, 5);
  // Relay created_at drift window (900s, pinned to ingest.rs in the replay
  // test) plus a 60s client clock margin.
  assert.equal(RESUME_LIVE_UNTIL_SKEW_SECS, 960);
  // Matches the relay-side silent-drop repair window (one hour).
  assert.equal(RESUME_SILENT_DROP_WINDOW_SECS, 3_600);
});

// since = max(lastSeen - 5, liveUntil - 960); the original filter's own `since`
// is applied by the replay path and covered in relayReconnectReplay.test.mjs.
for (const [name, input, expected] of [
  ["no cursor replays the original filter", {}, undefined],
  ["lastSeen only", { lastSeenCreatedAt: 1_000 }, 995],
  ["liveUntil only", { liveUntil: 1_000 }, 40],
  ["newer lastSeen wins", { lastSeenCreatedAt: 1_000, liveUntil: 900 }, 995],
  ["newer liveUntil wins", { lastSeenCreatedAt: 900, liveUntil: 2_000 }, 1_040],
  [
    "a future-dated lastSeen is still the lower bound",
    { lastSeenCreatedAt: 2_000, liveUntil: 1_000 },
    1_995,
  ],
  ["never negative", { lastSeenCreatedAt: 3, liveUntil: 10 }, 0],
  [
    "the liveUntil term never resumes later than the silent-drop window",
    { liveUntil: 10_000, nowSecs: 10_000 },
    10_000 - RESUME_SILENT_DROP_WINDOW_SECS,
  ],
  [
    "the window caps liveUntil only, never lastSeen",
    { lastSeenCreatedAt: 10_000, liveUntil: 10_000, nowSecs: 10_000 },
    9_995,
  ],
  [
    "an old liveUntil is below the window and is used as is",
    { liveUntil: 5_000, nowSecs: 10_000 },
    4_040,
  ],
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

test("liveness credit survives only gaps the stall watchdog would have allowed", () => {
  assert.equal(
    LIVENESS_MAX_GAP_MS,
    STALL_IDLE_TIMEOUT_MS + STALL_CHECK_INTERVAL_MS,
  );
  assert.equal(creditInboundLiveness(0, 5_000), 5_000, "first frame credits");
  assert.equal(
    creditInboundLiveness(5_000, 5_000 + LIVENESS_MAX_GAP_MS),
    5_000 + LIVENESS_MAX_GAP_MS,
    "a gap at the limit still credits",
  );
  assert.equal(
    creditInboundLiveness(5_000, 5_001 + LIVENESS_MAX_GAP_MS),
    5_000,
    "a frame processed after a longer gap may be queued through a sleep",
  );
  // Credit stays at the pre-gap frame: later frames never re-arm it.
  let credit = 5_000;
  for (const nowMs of [3_600_000, 3_601_000, 3_602_000]) {
    credit = creditInboundLiveness(credit, nowMs);
  }
  assert.equal(credit, 5_000);
});

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

  stampLiveUntilOnClose(subscriptions, 7, 1_234_999, 1_300_000);

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
  stampLiveUntilOnClose(
    new Map([["a", subscription]]),
    3,
    5_000_000,
    5_000_000,
  );
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

test("stampLiveUntilOnClose keeps the previous stamp when no inbound frame is known", () => {
  const stamped = live({ eoseGeneration: 3, liveUntil: 700 });
  const fresh = live({ eoseGeneration: 3 });
  stampLiveUntilOnClose(
    new Map([
      ["stamped", stamped],
      ["fresh", fresh],
    ]),
    3,
    0,
    9_000_000,
  );
  assert.equal(stamped.liveUntil, 700, "never guessed from the close clock");
  assert.equal(fresh.liveUntil, undefined);
  assert.equal(stamped.eoseGeneration, undefined);
  assert.equal(fresh.eoseGeneration, undefined);
});
