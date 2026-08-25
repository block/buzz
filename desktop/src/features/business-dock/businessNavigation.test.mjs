import assert from "node:assert/strict";
import test from "node:test";

import {
  canMoveBusinessNavigation,
  createBusinessNavigationState,
  currentBusinessNavigationEntry,
  moveBusinessNavigation,
  pushBusinessNavigation,
  updateCurrentBusinessNavigation,
} from "./businessNavigation.ts";

const resource = (id) => ({
  version: 1,
  type: "sales_order",
  id,
  path: `/embed/sales-orders/${id}`,
});

test("push, back, forward, and boundary state", () => {
  let state = createBusinessNavigationState(resource("SO-1"));
  state = pushBusinessNavigation(state, resource("SO-2"));
  assert.equal(state.index, 1);
  assert.equal(canMoveBusinessNavigation(state, -1), true);
  assert.equal(canMoveBusinessNavigation(state, 1), false);
  state = moveBusinessNavigation(state, -1);
  assert.equal(currentBusinessNavigationEntry(state)?.id, "SO-1");
  state = moveBusinessNavigation(state, 1);
  assert.equal(currentBusinessNavigationEntry(state)?.id, "SO-2");
});

test("new navigation truncates forward entries", () => {
  let state = createBusinessNavigationState(resource("SO-1"));
  state = pushBusinessNavigation(state, resource("SO-2"));
  state = moveBusinessNavigation(state, -1);
  state = pushBusinessNavigation(state, resource("SO-3"));
  assert.deepEqual(
    state.entries.map((entry) => entry.id),
    ["SO-1", "SO-3"],
  );
});

test("same paths dedupe and route changes replace the current entry", () => {
  let state = createBusinessNavigationState(resource("SO-1"));
  state = pushBusinessNavigation(state, {
    ...resource("SO-1"),
    title: "Order one",
  });
  assert.equal(state.entries.length, 1);
  assert.equal(state.entries[0].title, "Order one");
  state = updateCurrentBusinessNavigation(state, resource("SO-9"));
  assert.equal(state.entries.length, 1);
  assert.equal(currentBusinessNavigationEntry(state)?.id, "SO-9");
});
