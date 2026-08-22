import assert from "node:assert/strict";
import test from "node:test";

import {
  hasTimelineSettledThisSession,
  markTimelineSettledThisSession,
  resetSettledTimelineChannels,
} from "./settledTimelineChannels.ts";

test("settled channels are remembered for the session", () => {
  resetSettledTimelineChannels();
  assert.equal(hasTimelineSettledThisSession("ch-a"), false);
  markTimelineSettledThisSession("ch-a");
  assert.equal(hasTimelineSettledThisSession("ch-a"), true);
  assert.equal(hasTimelineSettledThisSession("ch-b"), false);
});

test("community reset clears every settled channel", () => {
  resetSettledTimelineChannels();
  markTimelineSettledThisSession("ch-a");
  markTimelineSettledThisSession("ch-b");
  resetSettledTimelineChannels();
  assert.equal(hasTimelineSettledThisSession("ch-a"), false);
  assert.equal(hasTimelineSettledThisSession("ch-b"), false);
});
