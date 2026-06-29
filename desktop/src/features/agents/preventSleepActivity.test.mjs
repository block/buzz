import assert from "node:assert/strict";
import test from "node:test";

import { createPreventSleepActivityTracker } from "./preventSleepActivity.ts";

const AGENT =
  "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

function event(seq) {
  return { seq, timestamp: `2024-01-01T00:00:0${seq}Z` };
}

test("prevent sleep activity tracker seeds existing observer events without touching", () => {
  const tracker = createPreventSleepActivityTracker();

  assert.equal(tracker.observe([{ pubkey: AGENT, events: [event(1)] }]), false);
});

test("prevent sleep activity tracker reports a newer observer event", () => {
  const tracker = createPreventSleepActivityTracker();
  tracker.observe([{ pubkey: AGENT, events: [event(1)] }]);

  assert.equal(
    tracker.observe([{ pubkey: AGENT, events: [event(1), event(2)] }]),
    true,
  );
});

test("prevent sleep activity tracker ignores unchanged latest observer event", () => {
  const tracker = createPreventSleepActivityTracker();
  tracker.observe([{ pubkey: AGENT, events: [event(1), event(2)] }]);

  assert.equal(
    tracker.observe([{ pubkey: AGENT, events: [event(1), event(2)] }]),
    false,
  );
});
