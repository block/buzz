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
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

test("primary+Shift+M toggles every mounted composer between prototypes", async () => {
  const { act, renderHook } = await import("@testing-library/react");
  const { isMacPlatform } = await import("@/shared/lib/platform");
  const { useComposerAddressPrototype } = await import(
    "./useComposerAddressPrototype.ts"
  );
  const first = renderHook(() => useComposerAddressPrototype());
  const second = renderHook(() => useComposerAddressPrototype());
  const pressShortcut = () =>
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        ctrlKey: !isMacPlatform(),
        key: "m",
        metaKey: isMacPlatform(),
        shiftKey: true,
      }),
    );

  assert.equal(first.result.current, "mention-button");
  assert.equal(second.result.current, "mention-button");
  act(pressShortcut);
  assert.equal(first.result.current, "send-button");
  assert.equal(second.result.current, "send-button");
  act(pressShortcut);
  assert.equal(first.result.current, "mention-button");
});
