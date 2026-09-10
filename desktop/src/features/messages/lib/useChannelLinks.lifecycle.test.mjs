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
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

async function renderChannelLinks(t) {
  const React = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { ChannelNavigationProvider } = await import(
    "@/shared/context/ChannelNavigationContext.tsx"
  );
  const { useChannelLinks } = await import("./useChannelLinks.ts");
  const channels = ["general", "engineering"].map((name) => ({
    id: name,
    name,
    channelType: "stream",
    archivedAt: null,
  }));
  const { result } = renderHook(useChannelLinks, {
    wrapper: ({ children }) =>
      React.createElement(ChannelNavigationProvider, { channels }, children),
  });
  t.mock.timers.enable({ apis: ["setTimeout"] });
  return {
    result,
    update(value, cursor = value.length) {
      act(() => result.current.updateChannelQuery(value, cursor));
    },
    tick(ms) {
      act(() => t.mock.timers.tick(ms));
    },
  };
}

for (const replacement of ["Edited, not deleted", "", "Welcome to #general "]) {
  test(`invalidating an open channel query is immediate: ${JSON.stringify(replacement)}`, async (t) => {
    const { result, update, tick } = await renderChannelLinks(t);
    update("Welcome to #general");
    tick(120);
    assert.equal(result.current.isChannelOpen, true);

    update(replacement);
    // Do not advance the debounce clock: the very next Enter must be free
    // to submit instead of inserting #general at its obsolete offset.
    assert.equal(result.current.isChannelOpen, false);
    for (const key of ["Enter", "Tab", "ArrowDown"]) {
      const event = { key, preventDefault: () => assert.fail("stole the key") };
      assert.deepEqual(result.current.handleChannelKeyDown(event), {
        handled: false,
      });
    }
    tick(120);
    assert.equal(result.current.isChannelOpen, false);
  });
}

test("an invalid update cancels pending publication; later valid queries still debounce", async (t) => {
  const { result, update, tick } = await renderChannelLinks(t);
  update("#general");
  tick(60);
  update("plain replacement");
  tick(120);
  assert.equal(result.current.isChannelOpen, false);

  update("#eng");
  tick(119);
  assert.equal(result.current.isChannelOpen, false);
  tick(1);
  assert.equal(result.current.isChannelOpen, true);
  assert.deepEqual(
    result.current.channelSuggestions.map((s) => s.name),
    ["engineering"],
  );
  let prevented = false;
  assert.deepEqual(
    result.current.handleChannelKeyDown({
      key: "Enter",
      preventDefault: () => {
        prevented = true;
      },
    }),
    { handled: true, suggestion: result.current.channelSuggestions[0] },
  );
  assert.equal(prevented, true);
});
