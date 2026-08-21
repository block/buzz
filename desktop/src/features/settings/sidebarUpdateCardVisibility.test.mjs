import assert from "node:assert/strict";
import test from "node:test";

import { shouldShowSidebarUpdateCard } from "./sidebarUpdateCardVisibility.ts";

test("shows the card for states that require user action or feedback", () => {
  assert.equal(shouldShowSidebarUpdateCard({ state: "ready" }), true);
  assert.equal(shouldShowSidebarUpdateCard({ state: "installing" }), true);
  assert.equal(shouldShowSidebarUpdateCard({ state: "manual-required" }), true);
});

test("hides the card for states with nothing actionable to show", () => {
  assert.equal(shouldShowSidebarUpdateCard({ state: "idle" }), false);
  assert.equal(shouldShowSidebarUpdateCard({ state: "checking" }), false);
  assert.equal(shouldShowSidebarUpdateCard({ state: "up-to-date" }), false);
  assert.equal(shouldShowSidebarUpdateCard({ state: "unavailable" }), false);
  assert.equal(shouldShowSidebarUpdateCard({ state: "available" }), false);
  assert.equal(shouldShowSidebarUpdateCard({ state: "downloading" }), false);
});

// Pre-existing behavior, unchanged by this fix: an install failure hides the
// card rather than surfacing an error state in the sidebar. The background
// check will retry and can re-show "available"/"ready" on its own schedule,
// so this is left as-is rather than scoped into the installing-state fix.
test("hides the card on error, matching pre-existing behavior", () => {
  assert.equal(
    shouldShowSidebarUpdateCard({ state: "error", message: "boom" }),
    false,
  );
});
