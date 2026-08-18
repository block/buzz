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

test("subscribe notifies on capture, clear, and consume", () => {
  const store = createPanelReturnTargetStore();
  let notifications = 0;
  const unsubscribe = store.subscribe(() => {
    notifications += 1;
  });

  store.capture({ kind: "profile", pubkey: "abc" });
  assert.equal(notifications, 1);

  store.consume();
  assert.equal(notifications, 2);

  store.clear();
  assert.equal(notifications, 3);

  unsubscribe();
  store.capture({ kind: "profile", pubkey: "abc" });
  assert.equal(notifications, 3);
});

test("reset drops the target silently", () => {
  const store = createPanelReturnTargetStore();
  let notifications = 0;
  store.subscribe(() => {
    notifications += 1;
  });

  store.capture({ kind: "thread", threadHeadId: "head-1" });
  assert.equal(notifications, 1);

  store.reset();
  assert.equal(notifications, 1);
  assert.equal(store.peek(), null);
});
