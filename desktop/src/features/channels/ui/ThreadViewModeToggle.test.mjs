import assert from "node:assert/strict";
import test from "node:test";

import { shouldRestoreThreadToggleFocus } from "./ThreadViewModeToggle.tsx";

test("restores toggle focus for keyboard activation, not pointer clicks", () => {
  assert.equal(shouldRestoreThreadToggleFocus(0), true);
  assert.equal(shouldRestoreThreadToggleFocus(1), false);
  assert.equal(shouldRestoreThreadToggleFocus(2), false);
});
