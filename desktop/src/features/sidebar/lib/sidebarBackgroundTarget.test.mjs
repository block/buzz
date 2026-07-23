import assert from "node:assert/strict";
import test from "node:test";

import { isInteractiveSidebarTarget } from "./sidebarBackgroundTarget.ts";

test("non-DOM event targets are sidebar background", () => {
  assert.equal(isInteractiveSidebarTarget(null), false);
  assert.equal(isInteractiveSidebarTarget({}), false);
});
