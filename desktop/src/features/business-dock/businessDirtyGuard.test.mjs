import assert from "node:assert/strict";
import test from "node:test";

import {
  getBusinessLeaveDecision,
  resolveBusinessLeaveConfirmation,
} from "./businessDirtyGuard.ts";

test("clean pages leave immediately while dirty pages request confirmation", () => {
  assert.equal(getBusinessLeaveDecision(false), "leave");
  assert.equal(getBusinessLeaveDecision(true), "confirm");
});

test("cancel preserves the page and confirmation performs navigation", () => {
  let navigated = false;
  assert.equal(
    resolveBusinessLeaveConfirmation(false, () => {
      navigated = true;
    }),
    false,
  );
  assert.equal(navigated, false);
  assert.equal(
    resolveBusinessLeaveConfirmation(true, () => {
      navigated = true;
    }),
    true,
  );
  assert.equal(navigated, true);
});
