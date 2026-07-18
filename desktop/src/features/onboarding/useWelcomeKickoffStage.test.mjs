import assert from "node:assert/strict";
import test from "node:test";

import { resolveWelcomeKickoffStagePhase } from "./useWelcomeKickoffStage.ts";

const base = {
  isWelcome: true,
  timelineSettled: true,
  hasMessages: false,
  timedOut: false,
};

test("stage stays hidden outside the Welcome channel", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", { ...base, isWelcome: false }),
    "hidden",
  );
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, isWelcome: false }),
    "hidden",
  );
});

test("stage waits for the timeline to settle before entering", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", {
      ...base,
      timelineSettled: false,
    }),
    "hidden",
  );
  assert.equal(resolveWelcomeKickoffStagePhase("hidden", base), "active");
});

test("stage never enters when messages already exist (revisit)", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("hidden", { ...base, hasMessages: true }),
    "hidden",
  );
});

test("first message moves an active stage to exiting", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, hasMessages: true }),
    "exiting",
  );
});

test("first message also dismisses a timed-out stage", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("timed-out", {
      ...base,
      hasMessages: true,
    }),
    "exiting",
  );
});

test("timeout only downgrades an active stage", () => {
  assert.equal(
    resolveWelcomeKickoffStagePhase("active", { ...base, timedOut: true }),
    "timed-out",
  );
  assert.equal(
    resolveWelcomeKickoffStagePhase("exiting", { ...base, timedOut: true }),
    "exiting",
  );
});

test("exiting is terminal until the exit animation completes", () => {
  assert.equal(resolveWelcomeKickoffStagePhase("exiting", base), "exiting");
});
