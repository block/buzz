import assert from "node:assert/strict";
import { afterEach, it } from "node:test";

import { JSDOM } from "jsdom";
import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { useTimelineRetention } from "./useTimelineRetention.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;
const originalActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT;
const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
const originalCancelAnimationFrame = globalThis.cancelAnimationFrame;

afterEach(() => {
  if (originalDocument === undefined) delete globalThis.document;
  else globalThis.document = originalDocument;
  if (originalWindow === undefined) delete globalThis.window;
  else globalThis.window = originalWindow;
  if (originalActEnvironment === undefined)
    delete globalThis.IS_REACT_ACT_ENVIRONMENT;
  else globalThis.IS_REACT_ACT_ENVIRONMENT = originalActEnvironment;
  if (originalRequestAnimationFrame === undefined)
    delete globalThis.requestAnimationFrame;
  else globalThis.requestAnimationFrame = originalRequestAnimationFrame;
  if (originalCancelAnimationFrame === undefined)
    delete globalThis.cancelAnimationFrame;
  else globalThis.cancelAnimationFrame = originalCancelAnimationFrame;
});

it("does not keep the full timeline mounted before the viewport is measured", async () => {
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div></body></html>",
  );
  let initialRefresh;
  Object.assign(globalThis, {
    cancelAnimationFrame() {
      initialRefresh = undefined;
    },
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
    requestAnimationFrame(callback) {
      initialRefresh = callback;
      return 1;
    },
    window: dom.window,
  });

  const keys = Array.from({ length: 10_000 }, (_, index) => `message-${index}`);
  const itemHeight = 100;
  const list = {
    findItemIndex(offset) {
      return Math.min(keys.length - 1, Math.floor(offset / itemHeight));
    },
    scrollOffset: 500_000,
    scrollSize: keys.length * itemHeight,
    viewportSize: 1_000,
  };
  let retention;
  function Harness() {
    retention = useTimelineRetention(keys, { current: list }, false);
    return null;
  }

  const root = createRoot(document.getElementById("root"));
  await act(async () => root.render(React.createElement(Harness)));

  assert.equal(retention.retainedIndices.length, 100);
  assert.equal(retention.retainedIndices[0], 9_900);
  assert.equal(retention.retainedIndices.at(-1), 9_999);

  await act(async () => initialRefresh());
  assert.ok(retention.retainedIndices.length > 0);
  assert.ok(retention.retainedIndices.length < 500);
  assert.ok(retention.retainedIndices.includes(5_000));
  assert.ok(retention.retainedIndices.includes(9_999));

  await act(async () => retention.onScrollEnd());
  assert.ok(retention.retainedIndices.length > 0);
  assert.ok(retention.retainedIndices.length < 500);
  assert.ok(retention.retainedIndices.includes(5_000));
  assert.ok(retention.retainedIndices.includes(9_999));

  await act(async () => root.unmount());
  dom.window.close();
});
