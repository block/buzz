import assert from "node:assert/strict";
import test from "node:test";

import { keepOpenableTrayActivities } from "./trayActivities.ts";

const activity = (activityId, channelId) => ({
  activityId,
  agentName: "Agent",
  channelId,
  channelName: "channel",
  elapsed: "1m",
});

test("removes tray activities whose channel is absent", () => {
  const openable = activity("openable", "channel-present");
  const stale = activity("stale", "channel-missing");

  assert.deepEqual(
    keepOpenableTrayActivities([openable, stale], new Set(["channel-present"])),
    [openable],
  );
});

test("does not publish activities before channels are available", () => {
  assert.deepEqual(
    keepOpenableTrayActivities([activity("pending", "channel")], new Set()),
    [],
  );
});
