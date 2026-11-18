import assert from "node:assert/strict";
import test from "node:test";

import { installDOMShim } from "../observedUnreadTestHarness.mjs";

installDOMShim();

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useOwnSendArm } from "./useOwnSendArm.ts";

// Mounts the hook over a timeline that records what it was asked to do. The
// pane's channel is read live, as ChannelPane reads activeChannelIdRef, so a
// test can switch it under an arm the composer already holds.
async function mountPane(channelId) {
  const calls = [];
  const timelineRef = {
    current: {
      cancelScrollToBottomOnNextUpdate: () => calls.push("disarm"),
      scrollToBottomOnNextUpdate: (target) => calls.push(`arm ${target}`),
    },
  };
  const pane = { arm: null, calls, channelId };
  const getTimelineChannelId = () => pane.channelId;
  function Harness() {
    pane.arm = useOwnSendArm(timelineRef, getTimelineChannelId);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(React.createElement(Harness)));
  pane.unmount = () => act(async () => root.unmount());
  return pane;
}

test("a send to the channel the timeline shows arms it", async () => {
  const pane = await mountPane("general");

  pane.arm("general");
  pane.arm(null);

  assert.deepEqual(pane.calls, ["arm general"]);
  await pane.unmount();
});

// A send whose upload spans a channel switch settles with the channel it was
// written in. Its arm must not pin or hold the channel shown now, and must not
// take the generation from the send that is pending there.
test("a send that settles after the pane switched channels is not armed", async () => {
  const pane = await mountPane("design");
  pane.channelId = "general";
  const disarmPending = pane.arm("general");

  const disarmLate = pane.arm("design");
  disarmLate();
  assert.deepEqual(pane.calls, ["arm general"]);

  disarmPending();
  assert.deepEqual(pane.calls, ["arm general", "disarm"]);
  await pane.unmount();
});

test("a failed send disarms the timeline", async () => {
  const pane = await mountPane("general");

  const disarm = pane.arm("general");
  disarm();

  assert.deepEqual(pane.calls, ["arm general", "disarm"]);
  await pane.unmount();
});

test("a later send's arm outlives an earlier send's failure", async () => {
  const pane = await mountPane("general");

  const disarmEarlier = pane.arm("general");
  const disarmLater = pane.arm("general");
  disarmEarlier();
  assert.deepEqual(pane.calls, ["arm general", "arm general"]);

  disarmLater();
  assert.deepEqual(pane.calls, ["arm general", "arm general", "disarm"]);
  await pane.unmount();
});
