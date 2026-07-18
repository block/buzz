import assert from "node:assert/strict";
import test from "node:test";

import {
  isWelcomeKickoffSettingUp,
  isWelcomeKickoffStageExiting,
  resolveWelcomeKickoffStagePhase,
} from "./useWelcomeKickoffStage.ts";

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

// The timeout exists to stop a failed kickoff from claiming a team is coming
// forever. These pin the *consequences* of timing out, not just the state name
// — the timed-out phase previously rendered identically to `active`.
test("a timed-out stage stops claiming the team is being set up", () => {
  assert.equal(isWelcomeKickoffSettingUp("active"), true);
  assert.equal(isWelcomeKickoffSettingUp("timed-out"), false);
});

test("a timed-out stage leaves instead of standing there", () => {
  assert.equal(isWelcomeKickoffStageExiting("timed-out"), true);
  assert.equal(isWelcomeKickoffStageExiting("exiting"), true);
  assert.equal(isWelcomeKickoffStageExiting("active"), false);
});

test("the banner never claims setup once the stage has resolved", () => {
  for (const phase of ["hidden", "exiting", "timed-out", "done"]) {
    assert.equal(
      isWelcomeKickoffSettingUp(phase),
      false,
      `${phase} must not claim setup is in progress`,
    );
  }
});

// Regression: `done` must be distinct from `hidden`. If a finished stage fell
// back to `hidden`, the still-empty timeline would re-enter `active` and the
// characters would loop forever.
test("done is terminal and never replays on a still-empty timeline", () => {
  assert.equal(resolveWelcomeKickoffStagePhase("done", base), "done");
  assert.equal(
    resolveWelcomeKickoffStagePhase("done", { ...base, isWelcome: false }),
    "done",
  );
  assert.equal(
    resolveWelcomeKickoffStagePhase("done", { ...base, timedOut: true }),
    "done",
  );
});
