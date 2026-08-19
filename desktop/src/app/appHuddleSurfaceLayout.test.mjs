import assert from "node:assert/strict";
import test from "node:test";

import { appHuddleSurfaceClassName } from "./appHuddleSurfaceLayout.ts";

test("normal app surface extends the sidebar substrate behind inset panels", () => {
  const className = appHuddleSurfaceClassName(false);
  assert.match(className, /\bbg-sidebar\b/);
  assert.doesNotMatch(className, /\bbg-background\b/);
});

test("dedicated Huddle room keeps its content background", () => {
  const className = appHuddleSurfaceClassName(true);
  assert.match(className, /\bbg-background\b/);
  assert.doesNotMatch(className, /\bbg-sidebar\b/);
});
