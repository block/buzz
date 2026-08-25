import assert from "node:assert/strict";
import test from "node:test";

import {
  businessDockReducer,
  createInitialBusinessDockState,
} from "./businessDockStore.ts";

const config = {
  homeUrl: "https://biz.example.com/embed/",
  origin: "https://biz.example.com",
};

test("business dock starts closed", () => {
  const state = createInitialBusinessDockState(config);
  assert.equal(state.open, false);
  assert.equal(state.fullscreen, false);
  assert.equal(state.currentUrl, config.homeUrl);
  assert.equal(state.followConversation, true);
  assert.equal(state.dirty, false);
});

test("business dock tracks resource, dirty, action, data, and follow state", () => {
  let state = createInitialBusinessDockState(config);
  const resource = {
    version: 1,
    type: "sales_order",
    id: "SO-1",
    path: "/embed/sales-orders/SO-1",
  };
  state = businessDockReducer(state, { type: "resource", resource });
  state = businessDockReducer(state, { type: "dirty", dirty: true });
  state = businessDockReducer(state, { type: "data-changed", changed: true });
  state = businessDockReducer(state, {
    type: "action",
    status: "completed",
    action: "approve_order",
    message: "Approved",
  });
  state = businessDockReducer(state, { type: "toggle-follow" });
  assert.equal(state.currentResource?.id, "SO-1");
  assert.equal(state.dirty, true);
  assert.equal(state.dataChanged, true);
  assert.equal(state.lastAction?.status, "completed");
  assert.equal(state.followConversation, false);
});

test("business dock opens, closes, pins, and preserves its URL", () => {
  let state = createInitialBusinessDockState(config);
  state = businessDockReducer(state, { type: "open" });
  state = businessDockReducer(state, {
    type: "navigate",
    url: "https://biz.example.com/orders/42",
  });
  state = businessDockReducer(state, { type: "toggle-pinned" });
  state = businessDockReducer(state, { type: "close" });
  assert.equal(state.open, false);
  assert.equal(state.pinned, true);
  assert.equal(state.currentUrl, "https://biz.example.com/orders/42");
});

test("business dock enters and exits full screen without changing its page", () => {
  const initial = businessDockReducer(createInitialBusinessDockState(config), {
    type: "open",
  });
  const fullscreen = businessDockReducer(initial, {
    type: "toggle-fullscreen",
  });
  assert.equal(fullscreen.fullscreen, true);
  const restored = businessDockReducer(fullscreen, {
    type: "exit-fullscreen",
  });
  assert.equal(restored.fullscreen, false);
  assert.equal(restored.currentUrl, initial.currentUrl);
});
