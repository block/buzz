import assert from "node:assert/strict";
import test from "node:test";

import { isSuppressedAsActiveChannel } from "./activeChannelSuppression.ts";

test("suppresses the active channel while the app is focused", () => {
  assert.equal(isSuppressedAsActiveChannel("ch1", "ch1", true, false), true);
});

test("does not suppress the active channel while the app is unfocused", () => {
  // Regression: a DM channel left selected in a minimized/backgrounded
  // window must still notify — the user isn't reading it.
  assert.equal(isSuppressedAsActiveChannel("ch1", "ch1", false, false), false);
});

test("does not suppress other channels regardless of focus", () => {
  assert.equal(isSuppressedAsActiveChannel("ch2", "ch1", true, false), false);
  assert.equal(isSuppressedAsActiveChannel("ch2", "ch1", false, false), false);
});

test("does not suppress when no channel is active", () => {
  assert.equal(isSuppressedAsActiveChannel("ch1", null, true, false), false);
});

test("notifyForActiveChannel opt-in disables suppression entirely", () => {
  assert.equal(isSuppressedAsActiveChannel("ch1", "ch1", true, true), false);
  assert.equal(isSuppressedAsActiveChannel("ch1", "ch1", false, true), false);
});
