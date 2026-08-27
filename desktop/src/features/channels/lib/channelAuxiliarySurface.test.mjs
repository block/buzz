import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveChannelAuxiliarySurface,
  resolveChannelCoverDrawer,
} from "./channelAuxiliarySurface.ts";

const NO_SURFACE = {
  channelManagementOpen: false,
  hasActiveChannel: true,
  hasProfilePanel: false,
  hasSelectedAgent: false,
  hasThreadHead: false,
  shouldShowThreadSkeleton: false,
};

test("no candidate surface resolves to nothing", () => {
  assert.equal(resolveChannelAuxiliarySurface(NO_SURFACE), null);
});

test("every candidate open at once still resolves to exactly one surface", () => {
  assert.equal(
    resolveChannelAuxiliarySurface({
      channelManagementOpen: true,
      hasActiveChannel: true,
      hasProfilePanel: true,
      hasSelectedAgent: true,
      hasThreadHead: true,
      shouldShowThreadSkeleton: true,
    }),
    "channel-management",
  );
});

test("surfaces resolve in priority order as higher ones drop away", () => {
  const all = {
    channelManagementOpen: true,
    hasActiveChannel: true,
    hasProfilePanel: true,
    hasSelectedAgent: true,
    hasThreadHead: true,
    shouldShowThreadSkeleton: true,
  };

  assert.equal(
    resolveChannelAuxiliarySurface({ ...all, channelManagementOpen: false }),
    "thread",
  );
  assert.equal(
    resolveChannelAuxiliarySurface({
      ...all,
      channelManagementOpen: false,
      hasThreadHead: false,
    }),
    "thread-skeleton",
  );
  assert.equal(
    resolveChannelAuxiliarySurface({
      ...all,
      channelManagementOpen: false,
      hasThreadHead: false,
      shouldShowThreadSkeleton: false,
    }),
    "agent-session",
  );
  assert.equal(
    resolveChannelAuxiliarySurface({
      ...all,
      channelManagementOpen: false,
      hasSelectedAgent: false,
      hasThreadHead: false,
      shouldShowThreadSkeleton: false,
    }),
    "profile",
  );
});

test("channel-scoped surfaces need an active channel", () => {
  const withoutChannel = { ...NO_SURFACE, hasActiveChannel: false };

  assert.equal(
    resolveChannelAuxiliarySurface({
      ...withoutChannel,
      channelManagementOpen: true,
    }),
    null,
  );
  assert.equal(
    resolveChannelAuxiliarySurface({
      ...withoutChannel,
      hasSelectedAgent: true,
    }),
    null,
  );
  // The profile panel is identity-scoped, so it survives without a channel.
  assert.equal(
    resolveChannelAuxiliarySurface({
      ...withoutChannel,
      hasProfilePanel: true,
    }),
    "profile",
  );
});

test("agent activity always covers at wide viewports, whatever the thread preference", () => {
  for (const threadViewMode of ["focus", "split"]) {
    assert.equal(
      resolveChannelCoverDrawer({
        surface: "agent-session",
        threadViewMode,
        useSplitAuxiliaryPane: true,
      }),
      "agent-session",
    );
  }
});

test("threads cover only in focus mode", () => {
  for (const surface of ["thread", "thread-skeleton"]) {
    assert.equal(
      resolveChannelCoverDrawer({
        surface,
        threadViewMode: "focus",
        useSplitAuxiliaryPane: true,
      }),
      "thread",
    );
    assert.equal(
      resolveChannelCoverDrawer({
        surface,
        threadViewMode: "split",
        useSplitAuxiliaryPane: true,
      }),
      null,
    );
  }
});

test("narrow and single-panel viewports never cover", () => {
  for (const surface of ["agent-session", "thread", "thread-skeleton"]) {
    assert.equal(
      resolveChannelCoverDrawer({
        surface,
        threadViewMode: "focus",
        useSplitAuxiliaryPane: false,
      }),
      null,
    );
  }
});

test("split-only surfaces never cover", () => {
  for (const surface of ["channel-management", "profile", null]) {
    assert.equal(
      resolveChannelCoverDrawer({
        surface,
        threadViewMode: "focus",
        useSplitAuxiliaryPane: true,
      }),
      null,
    );
  }
});

test("only one drawer can cover, because only one surface resolves", () => {
  // Thread and agent activity both requested: the surface resolution picks the
  // thread, so the agent drawer cannot also be covering.
  const surface = resolveChannelAuxiliarySurface({
    ...NO_SURFACE,
    hasSelectedAgent: true,
    hasThreadHead: true,
  });
  const drawer = resolveChannelCoverDrawer({
    surface,
    threadViewMode: "focus",
    useSplitAuxiliaryPane: true,
  });

  assert.equal(surface, "thread");
  assert.equal(drawer, "thread");
});
