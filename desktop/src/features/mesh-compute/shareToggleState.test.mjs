import assert from "node:assert/strict";
import test from "node:test";

import { deriveMeshShareToggle } from "./shareToggleState.ts";

const status = (overrides = {}) => ({
  state: "off",
  mode: null,
  health: { status: "ok", reason: null },
  apiBaseUrl: null,
  consoleUrl: null,
  modelId: null,
  modelName: null,
  ...overrides,
});

test("serve-mode running/starting reads as sharing", () => {
  for (const state of ["running", "starting"]) {
    const model = deriveMeshShareToggle(status({ state, mode: "serve" }));
    assert.equal(model.isSharing, true, `serve+${state} should be sharing`);
    assert.equal(model.isConsuming, false);
    assert.equal(model.slotOccupied, true);
  }
});

test("client-mode running/starting is consuming, NOT sharing (regression)", () => {
  // The core bug: consuming a peer's compute starts a client node in the same
  // slot, which reports state:"running". The Share toggle must stay off.
  for (const state of ["running", "starting"]) {
    const model = deriveMeshShareToggle(status({ state, mode: "client" }));
    assert.equal(
      model.isSharing,
      false,
      `client+${state} must NOT light the Share toggle`,
    );
    assert.equal(model.isConsuming, true);
    assert.equal(model.slotOccupied, true);
  }
});

test("a FAILED serve node still occupies the slot and stays turn-off-able", () => {
  // Expert review: a failed runtime still holds the single slot, so a fresh
  // start would throw "already running". It must read as sharing (so the
  // switch stays on and can be turned OFF to clear/retry) and occupy the slot.
  const model = deriveMeshShareToggle(
    status({ state: "failed", mode: "serve" }),
  );
  assert.equal(
    model.isSharing,
    true,
    "failed serve node is still turn-off-able",
  );
  assert.equal(model.slotOccupied, true);
});

test("a FAILED client node occupies the slot but is not sharing", () => {
  const model = deriveMeshShareToggle(
    status({ state: "failed", mode: "client" }),
  );
  assert.equal(model.isSharing, false);
  assert.equal(model.isConsuming, true);
  assert.equal(model.slotOccupied, true);
});

test("off / stopping never occupy the slot or read as sharing/consuming", () => {
  for (const state of ["off", "stopping"]) {
    for (const mode of [null, "serve", "client"]) {
      const model = deriveMeshShareToggle(status({ state, mode }));
      assert.equal(model.isSharing, false, `${mode}+${state} not sharing`);
      assert.equal(model.isConsuming, false, `${mode}+${state} not consuming`);
      assert.equal(model.slotOccupied, false, `${mode}+${state} slot free`);
    }
  }
});

test("null status (not yet fetched) is neither sharing nor consuming", () => {
  const model = deriveMeshShareToggle(null);
  assert.equal(model.isSharing, false);
  assert.equal(model.isConsuming, false);
  assert.equal(model.slotOccupied, false);
});

test("running with a missing mode occupies the slot but is not sharing", () => {
  // Defensive: a status that somehow lacks mode must not default to "on", but
  // it DOES hold the slot (a fresh start would fail), so it stays occupied.
  const model = deriveMeshShareToggle(status({ state: "running", mode: null }));
  assert.equal(model.isSharing, false);
  assert.equal(model.isConsuming, false);
  assert.equal(model.slotOccupied, true);
});
