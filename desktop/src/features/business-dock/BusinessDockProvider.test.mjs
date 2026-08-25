import assert from "node:assert/strict";
import test from "node:test";

import {
  canNavigateBusinessResource,
  keepLatestBusinessNavigation,
  shouldQueuePendingBusinessNavigation,
} from "./businessDockProviderPolicy.ts";
import {
  businessDockReducer,
  createInitialBusinessDockState,
} from "./businessDockStore.ts";

const config = {
  homeUrl: "https://biz.example.com/embed/",
  origin: "https://biz.example.com",
};
const resource = (id) => ({
  version: 1,
  type: "sales_order",
  id,
  path: `/embed/sales-orders/${id}`,
});

test("openBusinessResource opens a closed Dock and starts resource navigation", () => {
  let state = createInitialBusinessDockState(config);
  assert.equal(state.open, false);
  state = businessDockReducer(state, { type: "open" });
  state = businessDockReducer(state, {
    type: "navigate",
    url: "https://biz.example.com/embed/sales-orders/SO-1",
    resource: resource("SO-1"),
    openingResource: true,
  });
  assert.equal(state.open, true);
  assert.equal(state.currentResource?.id, "SO-1");
  assert.equal(state.openingResource, true);
});

test("explicit navigation remains allowed while pinned", () => {
  assert.equal(
    canNavigateBusinessResource({
      followConversation: false,
      pinned: true,
      source: "explicit",
    }),
    true,
  );
  assert.equal(
    canNavigateBusinessResource({
      followConversation: true,
      pinned: true,
      source: "automatic",
    }),
    false,
  );
});

test("pendingNavigation retains only the latest resource until Ready", () => {
  assert.equal(shouldQueuePendingBusinessNavigation(null), true);
  assert.equal(shouldQueuePendingBusinessNavigation(2), false);
  const pending = keepLatestBusinessNavigation(
    resource("SO-1"),
    resource("SO-2"),
  );
  assert.equal(pending.id, "SO-2");
});
