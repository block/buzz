import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import { installLocalStorage } from "../lib/testStorage.mjs";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    ResizeObserver: class {
      observe() {}
      disconnect() {}
    },
    self: dom.window,
    window: dom.window,
  });
  installLocalStorage(dom.window.localStorage);
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetConversationPlaygroundPins } = await import(
    "../lib/conversationPins.ts"
  );
  resetConversationPlaygroundPins();
  const { resetLinkSidePanelStore } = await import(
    "@/features/link-panel/lib/linkSidePanelStore.ts"
  );
  resetLinkSidePanelStore();
});

after(() => dom.window.close());

const sampleCard = (overrides = {}) => ({
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "4455",
  sid: "demo-1",
  stack: "hula-app",
  ...overrides,
});

async function renderMenu(props = { channelId: "chan-1" }) {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { ConversationPlaygroundPinsMenu } = await import(
    "./ConversationPlaygroundPinsMenu.tsx"
  );
  render(
    createElement(
      TooltipProvider,
      null,
      createElement(ConversationPlaygroundPinsMenu, props),
    ),
  );
  return screen;
}

test("pin icon hides the count badge when the scoped list is empty", async () => {
  const screen = await renderMenu({ channelId: "chan-1" });
  assert.ok(screen.getByTestId("conversation-playground-pins-menu"));
  assert.equal(screen.queryByTestId("conversation-playground-pins-badge"), null);
});

test("pin icon shows a count badge for scoped pins", async () => {
  const { pinPlaygroundToConversation } = await import(
    "../lib/conversationPins.ts"
  );
  pinPlaygroundToConversation(
    "channel:chan-1",
    sampleCard({ name: "Alpha", sid: "alpha", url: "https://a.example.com" }),
  );
  pinPlaygroundToConversation(
    "channel:chan-1",
    sampleCard({ name: "Beta", sid: "beta", url: "https://b.example.com" }),
  );

  const screen = await renderMenu({ channelId: "chan-1" });
  const badge = screen.getByTestId("conversation-playground-pins-badge");
  assert.equal(badge.textContent, "2");
  assert.equal(
    screen
      .getByTestId("conversation-playground-pins-menu")
      .getAttribute("aria-label"),
    "Playground pins (2)",
  );
});

test("thread scope badge ignores channel pins", async () => {
  const { pinPlaygroundToConversation } = await import(
    "../lib/conversationPins.ts"
  );
  pinPlaygroundToConversation(
    "channel:chan-1",
    sampleCard({
      name: "Channel only",
      sid: "chan-pin",
      url: "https://c.example.com",
    }),
  );
  pinPlaygroundToConversation(
    "thread:thread-9",
    sampleCard({
      name: "Thread pin",
      sid: "thread-pin",
      url: "https://t.example.com",
    }),
  );

  const screen = await renderMenu({
    channelId: "chan-1",
    threadId: "thread-9",
  });
  assert.equal(
    screen.getByTestId("conversation-playground-pins-badge").textContent,
    "1",
  );
});
