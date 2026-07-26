import assert from "node:assert/strict";
import test from "node:test";

import { isValidGroupHandle } from "./groupValidation.ts";

test("group handle validation mirrors the relay pattern", () => {
  for (const handle of ["ab", "ios-team", "risk_models", "a1"]) {
    assert.equal(isValidGroupHandle(handle), true, handle);
  }
  for (const handle of ["a", "IOS", "-team", "team space", "a".repeat(33)]) {
    assert.equal(isValidGroupHandle(handle), false, handle);
  }
});
