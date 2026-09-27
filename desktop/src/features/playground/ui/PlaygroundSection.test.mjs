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
  sid: "demo-1",
  stack: "hula-app",
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
  globalThis.localStorage?.clear();
});

after(() => dom.window.close());

async function renderSection() {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { SidebarProvider } = await import("@/shared/ui/sidebar.tsx");
  const { TooltipProvider } = await import("@/shared/ui/tooltip.tsx");
  const { PlaygroundSection } = await import("./PlaygroundSection.tsx");
  render(
    createElement(
      TooltipProvider,
      null,
      createElement(SidebarProvider, null, createElement(PlaygroundSection)),
    ),
  );
  return screen;
}

test("Playgrounds section is hidden when empty and visible with a session", async () => {
  const screen = await renderSection();
  assert.equal(screen.queryByTestId("playgrounds-section"), null);

  const { act } = await import("@testing-library/react");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  await act(() => {
    configurePlaygroundScope("pub", "wss://relay.example.com");
    addPlaygroundSession(card);
  });
  assert.ok(screen.getByTestId("playgrounds-section"));
  assert.equal(
    screen.getByTestId("playgrounds-section-label").textContent,
    "Playgrounds",
  );
  assert.ok(screen.getByTestId("open-playground-demo-1"));
});

test("Dispose of the last session removes the Playgrounds section", async () => {
  const { act } = await import("@testing-library/react");
  const { addPlaygroundSession, configurePlaygroundScope, disposePlayground } =
    await import("../lib/sessions.ts");
  configurePlaygroundScope("pub", "wss://relay.example.com");
  addPlaygroundSession(card);
  const screen = await renderSection();
  assert.ok(screen.getByTestId("playgrounds-section"));
  await act(() => {
    disposePlayground("demo-1");
  });
  assert.equal(screen.queryByTestId("playgrounds-section"), null);
});

test("Playgrounds section Open prefers RHS side-panel (not canvas window)", async () => {
  const { readFileSync } = await import("node:fs");
  const { fileURLToPath } = await import("node:url");
  const { dirname, join } = await import("node:path");
  const dir = dirname(fileURLToPath(import.meta.url));
  const source = readFileSync(join(dir, "PlaygroundSection.tsx"), "utf8");
  assert.match(source, /showPlaygroundSession\(session\.sid,\s*\{\s*preferSidePanel:\s*true\s*\}\)/);
});
