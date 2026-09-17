import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

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
    "../../playground/lib/conversationPins.ts"
  );
  resetConversationPlaygroundPins();
  const { resetLinkSidePanelStore } = await import(
    "@/features/link-panel/lib/linkSidePanelStore.ts"
  );
  resetLinkSidePanelStore();
});

after(() => dom.window.close());

test("popoutOverlayTitlebarShowsPins only for thread/split with ids", async () => {
  const { popoutOverlayTitlebarShowsPins } = await import(
    "./PopoutOverlayTitlebar.tsx"
  );
  assert.equal(popoutOverlayTitlebarShowsPins(null), false);
  assert.equal(
    popoutOverlayTitlebarShowsPins({
      kind: "playground",
      channelId: "c",
      threadId: "t",
    }),
    false,
  );
  assert.equal(
    popoutOverlayTitlebarShowsPins({
      kind: "thread",
      channelId: "c",
    }),
    false,
  );
  assert.equal(
    popoutOverlayTitlebarShowsPins({
      kind: "thread",
      channelId: "c",
      threadId: "t",
    }),
    true,
  );
  assert.equal(
    popoutOverlayTitlebarShowsPins({
      kind: "split",
      channelId: "c",
      threadId: "t",
    }),
    true,
  );
});

async function renderTitlebar(payload) {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { TooltipProvider } = await import("@/shared/ui/tooltip");
  const { PopoutOverlayTitlebar } = await import("./PopoutOverlayTitlebar.tsx");
  render(
    createElement(
      TooltipProvider,
      null,
      createElement(PopoutOverlayTitlebar, { payload }),
    ),
  );
  return screen;
}

test("thread pop-out overlay hosts playground pins without a close button", async () => {
  const { pinPlaygroundToConversation } = await import(
    "../../playground/lib/conversationPins.ts"
  );
  pinPlaygroundToConversation("thread:thread-1", {
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    pin: "4455",
    sid: "demo-1",
    stack: "hula-app",
  });

  const screen = await renderTitlebar({
    kind: "thread",
    channelId: "chan-1",
    threadId: "thread-1",
  });

  assert.ok(screen.getByTestId("popout-titlebar-gap"));
  assert.ok(screen.getByTestId("popout-overlay-titlebar"));
  assert.ok(screen.getByTestId("conversation-playground-pins-menu"));
  assert.ok(screen.getByTestId("conversation-playground-pins-badge"));
  assert.equal(screen.queryByLabelText(/close/i), null);
});

test("playground-only pop-out keeps an empty drag gap", async () => {
  const screen = await renderTitlebar({
    kind: "playground",
    channelId: "chan-1",
    playground: {
      hula: "playground",
      v: 1,
      name: "Demo",
      url: "https://app.example.com",
      sid: "demo-1",
    },
  });

  assert.ok(screen.getByTestId("popout-titlebar-gap"));
  assert.equal(screen.queryByTestId("popout-overlay-titlebar"), null);
  assert.equal(screen.queryByTestId("conversation-playground-pins-menu"), null);
});
