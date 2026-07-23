import assert from "node:assert/strict";
import test from "node:test";

import { isSidebarBackgroundTarget } from "./sidebarBackgroundTarget.ts";

test("non-DOM event targets are not sidebar background", () => {
  assert.equal(isSidebarBackgroundTarget(null), false);
  assert.equal(isSidebarBackgroundTarget({}), false);
});
