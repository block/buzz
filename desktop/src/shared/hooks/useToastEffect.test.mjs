import assert from "node:assert/strict";
import { test } from "node:test";

import { feedbackToastOptions } from "./useToastEffect.ts";

test("an error toast is given an explicit close control", () => {
  assert.equal(feedbackToastOptions("error").closeButton, true);
});

test("an error toast is not on a timer", () => {
  const { duration } = feedbackToastOptions("error");
  // Sonner treats a non-finite duration as "never auto-close"; anything finite
  // would put a multi-line error back into a race with the reader.
  assert.equal(Number.isFinite(duration), false);
});

test("a success toast keeps Sonner's defaults", () => {
  assert.equal(feedbackToastOptions("success"), undefined);
});
