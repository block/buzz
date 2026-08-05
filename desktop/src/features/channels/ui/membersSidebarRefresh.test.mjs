import assert from "node:assert/strict";
import test from "node:test";

import { shouldRefreshMembersOnOpen } from "./membersSidebarRefresh.ts";

test("shouldRefreshMembersOnOpen refreshes exactly when the sidebar opens", () => {
  assert.equal(shouldRefreshMembersOnOpen(false, false), false);
  assert.equal(shouldRefreshMembersOnOpen(true, false), true);
  assert.equal(shouldRefreshMembersOnOpen(true, true), false);
  assert.equal(shouldRefreshMembersOnOpen(false, true), false);
});
