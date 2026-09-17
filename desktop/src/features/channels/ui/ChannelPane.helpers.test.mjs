import assert from "node:assert/strict";
import test from "node:test";

import {
  getChannelIntroDescription,
  getChannelIntroKind,
  shouldPrioritizeIdleAuxiliary,
  shouldUseFocusIdleDrawer,
} from "./ChannelPane.helpers.ts";

function channel(overrides = {}) {
  return {
    ttlDeadline: null,
    ttlSeconds: null,
    visibility: "open",
    ...overrides,
  };
}

test("focus idle drawers yield to every higher-priority auxiliary surface", () => {
  const idleDrawer = {
    channelManagementOpen: false,
    hasAgentSession: false,
    hasIdleAuxiliaryPanel: true,
    hasIdlePanelCloseHandler: true,
    hasProfilePanel: false,
    hasThreadSurface: false,
    useSplitAuxiliaryPane: true,
  };

  assert.equal(shouldUseFocusIdleDrawer(idleDrawer), true);
  for (const surface of [
    "channelManagementOpen",
    "hasAgentSession",
    "hasProfilePanel",
    "hasThreadSurface",
  ]) {
    assert.equal(
      shouldUseFocusIdleDrawer({ ...idleDrawer, [surface]: true }),
      false,
      `idle drawer must yield when ${surface} is open`,
    );
  }
});

test("an explicit thread override keeps the idle panel in its own focus drawer", () => {
  assert.equal(
    shouldUseFocusIdleDrawer({
      channelManagementOpen: false,
      hasAgentSession: false,
      hasIdleAuxiliaryPanel: true,
      hasIdlePanelCloseHandler: true,
      hasProfilePanel: false,
      hasThreadSurface: true,
      overrideThread: true,
      useSplitAuxiliaryPane: false,
    }),
    true,
  );
});

test("channel intro shares description-over-purpose derivation with the header", () => {
  assert.equal(
    getChannelIntroDescription(
      channel({
        description: "Description paragraphs.\n\nKeep this structure.",
        purpose: "Legacy purpose",
        topic: "",
      }),
    ),
    "Description paragraphs.\n\nKeep this structure.",
  );
});

test("getChannelIntroKind names project homes ahead of regular streams", () => {
  assert.equal(getChannelIntroKind(channel(), true), "project channel");
  assert.equal(getChannelIntroKind(channel(), false), "regular channel");
});

test("getChannelIntroKind keeps private and ephemeral labels for other streams", () => {
  assert.equal(
    getChannelIntroKind(channel({ visibility: "private" })),
    "private channel",
  );
  assert.equal(
    getChannelIntroKind(channel({ ttlSeconds: 3600 })),
    "ephemeral channel",
  );
});

test("an open link slide-out must override a thread even when not expanded", () => {
  // Regression for useChannelLinkSidePanel: it used to pass panel.expanded as
  // idleAuxiliaryOverridesThread. Open / pin / link clicks from inside a
  // thread updated the store, but the thread kept the right-hand slot so the
  // slide-out never appeared. Contract matches Project workspace sheets:
  // any open idle auxiliary that wants the slide-out sets overrideThread.
  assert.equal(shouldPrioritizeIdleAuxiliary(true, true), true);
  assert.equal(
    shouldUseFocusIdleDrawer({
      channelManagementOpen: false,
      hasAgentSession: false,
      hasIdleAuxiliaryPanel: true,
      hasIdlePanelCloseHandler: true,
      hasProfilePanel: false,
      hasThreadSurface: true,
      overrideThread: true,
      useSplitAuxiliaryPane: true,
    }),
    true,
  );
});

test("idle auxiliary priority does not depend on thread layout mode", () => {
  assert.equal(shouldPrioritizeIdleAuxiliary(true, true), true);
  assert.equal(shouldPrioritizeIdleAuxiliary(true, false), false);
  assert.equal(shouldPrioritizeIdleAuxiliary(false, true), false);
});

test("link slide-out expand gates focus drawer; collapsed stays a side panel", () => {
  const base = {
    channelManagementOpen: false,
    hasAgentSession: false,
    hasIdleAuxiliaryPanel: true,
    hasIdlePanelCloseHandler: true,
    hasProfilePanel: false,
    hasThreadSurface: true,
    overrideThread: true,
    useSplitAuxiliaryPane: true,
  };
  // Default / expanded → focus drawer covering the conversation.
  assert.equal(shouldUseFocusIdleDrawer(base), true);
  assert.equal(shouldUseFocusIdleDrawer({ ...base, expanded: true }), true);
  // Collapsed → normal-width side panel (still overrides thread slot via
  // shouldPrioritizeIdleAuxiliary + replaceThreadWithIdleAuxiliary).
  assert.equal(shouldUseFocusIdleDrawer({ ...base, expanded: false }), false);
});
