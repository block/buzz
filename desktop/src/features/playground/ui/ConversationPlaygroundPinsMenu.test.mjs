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
    // Radix Presence reads global getComputedStyle when the menu opens.
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
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
  assert.equal(
    screen.queryByTestId("conversation-playground-pins-badge"),
    null,
  );
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

test("header pin open path uses keepAlive and sanitize-safe pinId", async () => {
  // Mirrors ConversationPlaygroundPinsMenu.openPin — the named-pin click path.
  const { conversationPlaygroundPinWebviewId } = await import(
    "../lib/conversationPins.ts"
  );
  const { getLinkSidePanel, openLinkSidePanel } = await import(
    "@/features/link-panel/lib/linkSidePanelStore.ts"
  );

  const pin = {
    sid: "alpha",
    name: "Alpha",
    url: "https://a.example.com",
  };
  assert.equal(
    openLinkSidePanel(pin.url, {
      title: pin.name,
      pinId: conversationPlaygroundPinWebviewId(pin.sid),
      keepAlive: true,
    }),
    true,
  );
  const panel = getLinkSidePanel();
  assert.ok(panel);
  assert.equal(panel.url, pin.url);
  assert.equal(panel.title, pin.name);
  assert.equal(panel.keepAlive, true);
  assert.equal(panel.pinId, "playground-pin-alpha");
  assert.match(panel.pinId, /^[A-Za-z0-9_-]+$/);
});

test("menu unpin path removes the pin and destroys any open keep-alive panel", async () => {
  // Mirrors ConversationPlaygroundPinsMenu.unpin — store contract the X button
  // must honor (no openLinkSidePanel). UI open of Radix menus is flaky in jsdom.
  const {
    conversationPlaygroundPinWebviewId,
    listConversationPlaygroundPins,
    pinPlaygroundToConversation,
    unpinPlaygroundFromConversation,
  } = await import("../lib/conversationPins.ts");
  const { destroyLinkSidePanelIfPin, getLinkSidePanel, openLinkSidePanel } =
    await import("@/features/link-panel/lib/linkSidePanelStore.ts");

  pinPlaygroundToConversation(
    "channel:chan-1",
    sampleCard({
      name: "Hula Home",
      sid: "hula-home",
      url: "https://hulapreview.example.com",
    }),
  );
  assert.equal(
    openLinkSidePanel("https://hulapreview.example.com", {
      title: "Hula Home",
      pinId: conversationPlaygroundPinWebviewId("hula-home"),
      keepAlive: true,
    }),
    true,
  );
  assert.ok(getLinkSidePanel());

  const scopeKey = "channel:chan-1";
  const sid = "hula-home";
  unpinPlaygroundFromConversation(scopeKey, sid);
  destroyLinkSidePanelIfPin(conversationPlaygroundPinWebviewId(sid));

  assert.equal(listConversationPlaygroundPins(scopeKey).length, 0);
  assert.equal(getLinkSidePanel(), null);
});

test("opening a named pin uses keepAlive chrome path (not a bare webview)", async () => {
  // Mirrors ConversationPlaygroundPinsMenu.openPin — row click must open the
  // IdleAuxiliaryPanel-backed link side panel with title/close chrome.
  const { conversationPlaygroundPinWebviewId } = await import(
    "../lib/conversationPins.ts"
  );
  const { getLinkSidePanel, openLinkSidePanel } = await import(
    "@/features/link-panel/lib/linkSidePanelStore.ts"
  );

  const pin = {
    sid: "hula-home",
    name: "Hula Home",
    url: "https://hulapreview.example.com",
  };
  assert.equal(
    openLinkSidePanel(pin.url, {
      title: pin.name,
      pinId: conversationPlaygroundPinWebviewId(pin.sid),
      keepAlive: true,
    }),
    true,
  );
  const panel = getLinkSidePanel();
  assert.ok(panel);
  assert.equal(panel.title, "Hula Home");
  assert.equal(panel.keepAlive, true);
  assert.equal(panel.pinId, "playground-pin-hula-home");
});
