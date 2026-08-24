import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    KeyboardEvent: dom.window.KeyboardEvent,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

async function renderShortcuts(enabled) {
  const { renderHook } = await import("@testing-library/react");
  const { useMarkAsReadShortcuts } = await import(
    "./useMarkAsReadShortcuts.ts"
  );
  const calls = [];
  const view = renderHook(() =>
    useMarkAsReadShortcuts({
      activeChannelId: "general",
      activeChannelLastMessageAt: "2026-08-24T00:00:00.000Z",
      enabled,
      markAllChannelsRead: () => calls.push("all"),
      markChannelRead: () => calls.push("channel"),
      selectedView: "channel",
    }),
  );
  return { calls, view };
}

function pressEscape({ shiftKey = false } = {}) {
  window.dispatchEvent(
    new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      key: "Escape",
      shiftKey,
    }),
  );
}

test("disabled shortcuts ignore Escape and Shift+Escape", async () => {
  const { calls } = await renderShortcuts(false);

  pressEscape();
  pressEscape({ shiftKey: true });

  assert.deepEqual(calls, []);
});

test("enabled shortcuts preserve channel and all-channel actions", async () => {
  const { calls } = await renderShortcuts(true);

  pressEscape();
  pressEscape({ shiftKey: true });

  assert.deepEqual(calls, ["channel", "all"]);
});
