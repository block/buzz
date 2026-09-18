import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import { installLocalStorage } from "../lib/testStorage.mjs";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "4455",
  sid: "demo-chrome-layout",
};

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
  const { resetPlaygroundState } = await import("../lib/sessions.ts");
  resetPlaygroundState();
});

after(() => dom.window.close());

test("Open as Split chrome: URL row then modes left + tools right", async () => {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { PlaygroundChrome } = await import("./PlaygroundChrome.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession(card);
  render(
    createElement(PlaygroundChrome, {
      conversation: null,
      docked: false,
      fullscreen: false,
      mode: "desktop",
      onModeChange() {},
      onToggleDock() {},
      onToggleFullscreen() {},
      session,
    }),
  );

  const chrome = screen.getByTestId("playground-chrome");
  const address = screen.getByTestId("playground-address");
  const modeRow = screen.getByTestId("playground-mode-row");
  const tools = screen.getByTestId("playground-tool-icons");

  assert.equal(address.parentElement, chrome);
  assert.equal(modeRow.parentElement, chrome);
  assert.ok(address.compareDocumentPosition(modeRow) & 4);
  assert.ok(modeRow.contains(screen.getByTestId("playground-mode-desktop")));
  assert.ok(modeRow.contains(tools));
  assert.match(tools.className, /ml-auto/);
  assert.ok(tools.contains(screen.getByTestId("playground-back")));
  assert.ok(screen.getByTestId("playground-url-prefix"));
});
