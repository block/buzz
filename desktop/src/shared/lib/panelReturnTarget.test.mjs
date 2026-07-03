import assert from "node:assert/strict";
import test from "node:test";

import { createPanelReturnTargetStore } from "./panelReturnTarget.ts";

test("consume returns the captured target exactly once", () => {
  const store = createPanelReturnTargetStore();

  store.capture({ kind: "thread", threadHeadId: "head-1" });

  assert.deepEqual(store.consume(), { kind: "thread", threadHeadId: "head-1" });
  assert.equal(store.consume(), null);
});

test("capture overwrites a previous target", () => {
  const store = createPanelReturnTargetStore();

  store.capture({ kind: "thread", threadHeadId: "head-1" });
  store.capture({ kind: "profile", pubkey: "abc" });

  assert.deepEqual(store.consume(), { kind: "profile", pubkey: "abc" });
});

test("capturing null clears the target", () => {
  const store = createPanelReturnTargetStore();

  store.capture({ kind: "profile", pubkey: "abc" });
  store.capture(null);

  assert.equal(store.consume(), null);
});

test("clear drops the target without consuming", () => {
  const store = createPanelReturnTargetStore();

  store.capture({ kind: "profile", pubkey: "abc" });
  store.clear();

  assert.equal(store.peek(), null);
  assert.equal(store.consume(), null);
});

test("peek reads without consuming", () => {
  const store = createPanelReturnTargetStore();

  store.capture({ kind: "thread", threadHeadId: "head-1" });

  assert.deepEqual(store.peek(), { kind: "thread", threadHeadId: "head-1" });
  assert.deepEqual(store.consume(), { kind: "thread", threadHeadId: "head-1" });
});
