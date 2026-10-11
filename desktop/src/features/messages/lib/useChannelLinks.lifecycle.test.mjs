import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";
import * as React from "react";

import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { useChannelLinks } from "./useChannelLinks.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});
after(() => dom.window.close());

const channels = [
  { id: "general", name: "general", channelType: "stream", archivedAt: null },
];

function wrapper({ children }) {
  return React.createElement(ChannelNavigationProvider, { channels }, children);
}

function enterEvent() {
  return {
    key: "Enter",
    defaultPrevented: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
  };
}

for (const [name, text, cursor] of [
  ["replacing the loaded edit", "Edited, not deleted", 19],
  ["clearing the loaded edit", "", 0],
  [
    "receiving an update with the cursor before the reference",
    "Welcome to #general",
    0,
  ],
]) {
  test(`${name} releases Enter before the debounce expires`, async (t) => {
    const { act, renderHook } = await import("@testing-library/react");
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const { result, unmount } = renderHook(useChannelLinks, { wrapper });
    act(() => result.current.updateChannelQuery("Welcome to #general", 19));
    act(() => t.mock.timers.tick(120));
    assert.equal(result.current.isChannelOpen, true);

    act(() => result.current.updateChannelQuery(text, cursor));
    assert.equal(result.current.isChannelOpen, false);
    const event = enterEvent();
    assert.deepEqual(result.current.handleChannelKeyDown(event), {
      handled: false,
    });
    assert.equal(event.defaultPrevented, false);
    act(() => t.mock.timers.tick(120));
    assert.equal(result.current.isChannelOpen, false);
    unmount();
  });
}

test("clearing a pending query cannot reopen channel completion", async (t) => {
  const { act, renderHook } = await import("@testing-library/react");
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { result, unmount } = renderHook(useChannelLinks, { wrapper });
  act(() => result.current.updateChannelQuery("#gen", 4));
  act(() => result.current.updateChannelQuery("", 0));
  act(() => t.mock.timers.tick(120));
  assert.equal(result.current.isChannelOpen, false);
  unmount();
});

test("valid channel completion still debounces and consumes Enter", async (t) => {
  const { act, renderHook } = await import("@testing-library/react");
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { result, unmount } = renderHook(useChannelLinks, { wrapper });
  act(() => result.current.updateChannelQuery("See #gen", 8));
  act(() => t.mock.timers.tick(119));
  assert.equal(result.current.isChannelOpen, false);
  act(() => t.mock.timers.tick(1));
  assert.equal(result.current.isChannelOpen, true);
  const event = enterEvent();
  const selection = result.current.handleChannelKeyDown(event);
  assert.equal(selection.handled, true);
  assert.equal(event.defaultPrevented, true);
  let edit;
  act(() => {
    edit = result.current.insertChannel(selection.suggestion, 8);
  });
  assert.deepEqual(edit, {
    replaceFromOffset: 4,
    replaceToOffset: 8,
    insertText: "#general ",
  });
  assert.equal(result.current.isChannelOpen, false);
  unmount();
});
