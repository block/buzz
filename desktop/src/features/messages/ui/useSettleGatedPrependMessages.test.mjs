import assert from "node:assert/strict";
import test from "node:test";

import { selectSettleGatedMessages } from "./useSettleGatedPrependMessages.ts";

const rows = (...ids) => ids.map((id) => ({ id }));

test("holds a pure older-history prepend", () => {
  const decision = selectSettleGatedMessages({
    admitted: rows("a", "b", "c"),
    next: rows("older-1", "older-2", "a", "b", "c"),
  });
  assert.equal(decision.kind, "hold");
  assert.deepEqual(
    decision.held.map(({ id }) => id),
    ["a", "b", "c"],
  );
});

test("held snapshot uses refreshed row objects for the admitted ids", () => {
  const editedA = { id: "a", edited: true };
  const decision = selectSettleGatedMessages({
    admitted: rows("a", "b"),
    next: [{ id: "older-1" }, editedA, { id: "b" }],
  });
  assert.equal(decision.kind, "hold");
  assert.equal(decision.held[0], editedA);
});

test("passes appends through immediately", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("a", "b", "c"),
    }).kind,
    "pass",
  );
});

test("passes a simultaneous prepend+append (own send) through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("older-1", "a", "b", "sent"),
    }).kind,
    "pass",
  );
});

test("passes deletions inside the admitted window through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b", "c"),
      next: rows("older-1", "a", "c"),
    }).kind,
    "pass",
  );
});

test("passes authoritative replacements through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("x", "y"),
    }).kind,
    "pass",
  );
});

test("passes identical snapshots through", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: rows("a", "b"),
      next: rows("a", "b"),
    }).kind,
    "pass",
  );
});

test("passes when the previous snapshot was empty (initial load)", () => {
  assert.equal(
    selectSettleGatedMessages({
      admitted: [],
      next: rows("a", "b"),
    }).kind,
    "pass",
  );
});

// Hook-level: the hold is released by the scroller's observed motion, never by
// an assumed one. Frames and the clock are both driven by hand.
async function mountHold({ motionBeforeHoldAt = null } = {}) {
  const { JSDOM } = await import("jsdom");
  const React = await import("react");
  const { act } = React;
  const { createRoot } = await import("react-dom/client");
  const { useSettleGatedPrependMessages } = await import(
    "./useSettleGatedPrependMessages.ts"
  );
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div><div id='scroller'></div></body></html>",
  );
  const frames = [];
  let now = 0;
  const saved = {
    document: globalThis.document,
    window: globalThis.window,
    IS_REACT_ACT_ENVIRONMENT: globalThis.IS_REACT_ACT_ENVIRONMENT,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    cancelAnimationFrame: globalThis.cancelAnimationFrame,
    performanceNow: performance.now,
  };
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
    IS_REACT_ACT_ENVIRONMENT: true,
    requestAnimationFrame(callback) {
      frames.push(callback);
      return frames.length;
    },
    cancelAnimationFrame() {},
  });
  performance.now = () => now;

  const scroller = dom.window.document.getElementById("scroller");
  scroller.scrollTop = 400;
  if (motionBeforeHoldAt !== null) {
    now = motionBeforeHoldAt;
  }
  let output;
  function Harness({ messages }) {
    output = useSettleGatedPrependMessages({
      channelId: "c",
      messages,
      meta: null,
      scrollElementRef: { current: scroller },
    });
    return null;
  }
  const root = createRoot(dom.window.document.getElementById("root"));
  await act(async () =>
    root.render(React.createElement(Harness, { messages: rows("a", "b") })),
  );
  if (motionBeforeHoldAt !== null) {
    scroller.dispatchEvent(new dom.window.Event("scroll"));
  }
  now = motionBeforeHoldAt === null ? 1_000 : motionBeforeHoldAt + 50;
  await act(async () =>
    root.render(
      React.createElement(Harness, { messages: rows("older", "a", "b") }),
    ),
  );
  const tick = async (advanceMs) => {
    now += advanceMs;
    const pending = frames.splice(0);
    await act(async () => {
      for (const frame of pending) frame();
    });
  };
  const cleanup = async () => {
    await act(async () => root.unmount());
    dom.window.close();
    performance.now = saved.performanceNow;
    for (const key of [
      "document",
      "window",
      "IS_REACT_ACT_ENVIRONMENT",
      "requestAnimationFrame",
      "cancelAnimationFrame",
    ]) {
      if (saved[key] === undefined) delete globalThis[key];
      else globalThis[key] = saved[key];
    }
  };
  return { output: () => output, tick, cleanup };
}

test("a scroller at rest admits after the stable-frame count with no assumed quiet window", async () => {
  const hold = await mountHold();
  assert.equal(hold.output().isHoldingPrepend, true);
  // Three frames, clock barely moving: nothing has reported motion, so the
  // only thing the gate may wait for is frame-over-frame stability.
  await hold.tick(1);
  await hold.tick(1);
  assert.equal(hold.output().isHoldingPrepend, true);
  await hold.tick(1);
  assert.equal(hold.output().isHoldingPrepend, false);
  assert.deepEqual(
    hold.output().messages.map(({ id }) => id),
    ["older", "a", "b"],
  );
  await hold.cleanup();
});

test("motion observed before the hold opened still charges the quiet window", async () => {
  const hold = await mountHold({ motionBeforeHoldAt: 10_000 });
  // Scroll event at t=10000, hold opened at t=10050. Stable frames alone must
  // not admit until 100ms have passed since that event.
  await hold.tick(10);
  await hold.tick(10);
  await hold.tick(10);
  await hold.tick(10);
  assert.equal(hold.output().isHoldingPrepend, true);
  await hold.tick(20);
  assert.equal(hold.output().isHoldingPrepend, false);
  await hold.cleanup();
});
