import assert from "node:assert/strict";
import test from "node:test";

import {
  reconcilePresentationWithNativeExists,
  shouldCoalesceHuddleCompanionOpen,
  shouldMountMainHuddleDrawerBar,
  shouldPromoteNoneToDrawer,
  shouldShowHuddleDockChrome,
} from "./huddlePresentation.ts";

test("main must never mount drawer bar (companion-only product)", () => {
  assert.equal(shouldMountMainHuddleDrawerBar("window"), false);
  assert.equal(shouldMountMainHuddleDrawerBar("drawer"), false);
  assert.equal(shouldMountMainHuddleDrawerBar("none"), false);
});

test("native companion existence forces window presentation (heals desync)", () => {
  assert.equal(reconcilePresentationWithNativeExists("drawer", true), "window");
  assert.equal(reconcilePresentationWithNativeExists("none", true), "window");
  assert.equal(reconcilePresentationWithNativeExists("window", true), null);
});

test("missing companion while React says window demotes to none (not drawer)", () => {
  assert.equal(
    reconcilePresentationWithNativeExists("window", false, {
      huddleActive: true,
    }),
    "none",
  );
  assert.equal(
    reconcilePresentationWithNativeExists("window", false, {
      huddleActive: false,
    }),
    "none",
  );
  assert.equal(
    reconcilePresentationWithNativeExists("drawer", false, {
      huddleActive: true,
    }),
    "none",
  );
});

test("in-flight open must not demote window while exists is briefly false", () => {
  assert.equal(
    reconcilePresentationWithNativeExists("window", false, {
      openInFlight: true,
      huddleActive: true,
    }),
    null,
  );
});

test("none stays put when companion does not exist", () => {
  assert.equal(reconcilePresentationWithNativeExists("none", false), null);
});

test("none→drawer promote is hard-disabled (companion-only)", () => {
  assert.equal(
    shouldPromoteNoneToDrawer({
      presentation: "none",
      companionExists: false,
      openInFlight: false,
      startPending: false,
    }),
    false,
  );
  assert.equal(
    shouldPromoteNoneToDrawer({
      presentation: "none",
      companionExists: false,
      openInFlight: false,
      startPending: true,
    }),
    false,
  );
});

test("coalesce companion open only while in-flight for same channel", () => {
  assert.equal(
    shouldCoalesceHuddleCompanionOpen({
      sameChannel: true,
      openInFlight: true,
    }),
    true,
  );
  assert.equal(
    shouldCoalesceHuddleCompanionOpen({
      sameChannel: true,
      openInFlight: false,
    }),
    false,
  );
  assert.equal(
    shouldCoalesceHuddleCompanionOpen({
      sameChannel: false,
      openInFlight: true,
    }),
    false,
  );
});

test("companion room always shows dock chrome; main never", () => {
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: true, presentation: "none" }),
    true,
  );
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: true, presentation: "window" }),
    true,
  );
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: true, presentation: "drawer" }),
    true,
  );
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: false, presentation: "none" }),
    false,
  );
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: false, presentation: "drawer" }),
    false,
  );
  assert.equal(
    shouldShowHuddleDockChrome({ isRoom: false, presentation: "window" }),
    false,
  );
});
