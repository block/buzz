import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import {
  clearSendFailure,
  clearSendFailures,
  recordSendFailure,
  sendFailureChannelIds,
} from "./sendFailures.ts";

beforeEach(() => {
  clearSendFailures();
});

test("recording marks the channel and clearing resolves it", () => {
  recordSendFailure("channel-a");
  recordSendFailure("channel-b");

  assert.deepEqual([...sendFailureChannelIds()].sort(), [
    "channel-a",
    "channel-b",
  ]);

  clearSendFailure("channel-a");
  assert.deepEqual([...sendFailureChannelIds()], ["channel-b"]);
});

test("record and clear are idempotent and keep the snapshot stable", () => {
  recordSendFailure("channel-a");
  const snapshot = sendFailureChannelIds();

  recordSendFailure("channel-a");
  assert.equal(sendFailureChannelIds(), snapshot);

  clearSendFailure("channel-never-failed");
  assert.equal(sendFailureChannelIds(), snapshot);
});
