import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const LONG_URL =
  "https://example.com/very/long/path/that/used/to/push/chrome/icons/off/screen?query=1&more=2";

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
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  globalThis.__TAURI_INTERNALS__ = {
    invoke: async () => ({
      canGoBack: false,
      canGoForward: false,
      currentUrl: LONG_URL,
    }),
  };
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetLinkSidePanelStore } = await import(
    "../lib/linkSidePanelStore.ts"
  );
  resetLinkSidePanelStore();
});

after(() => dom.window.close());

async function renderChrome({
  expanded = false,
  url = LONG_URL,
  viewportMode = "desktop",
} = {}) {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const {
    Outlet,
    RouterProvider,
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
  } = await import("@tanstack/react-router");
  const { LinkSidePanelChrome } = await import("./LinkSidePanelChrome.tsx");

  const rootRoute = createRootRoute({ component: Outlet });
  const channelRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/channels/$channelId",
    component: () =>
      createElement(LinkSidePanelChrome, {
        expanded,
        pinId: "hula-link-side-panel",
        url,
        viewportMode,
      }),
  });
  const router = createRouter({
    defaultPendingMs: 0,
    routeTree: rootRoute.addChildren([channelRoute]),
    history: createMemoryHistory({ initialEntries: ["/channels/chan-1"] }),
  });
  await router.load();
  render(createElement(RouterProvider, { router }));
  return screen;
}

test("chrome keeps URL on row 1 and right-justifies tooling on the mode row", async () => {
  const screen = await renderChrome();

  const chrome = screen.getByTestId("link-side-panel-chrome");
  const url = screen.getByTestId("link-side-panel-url");
  const modeRow = screen.getByTestId("link-side-panel-mode-row");
  const tools = screen.getByTestId("link-side-panel-tool-icons");

  assert.match(chrome.className, /flex-col/);
  assert.match(url.className, /truncate/);
  assert.match(url.className, /min-w-0/);
  assert.equal(url.textContent, LONG_URL);

  // URL is a direct child of chrome (row 1), not nested under the mode/tools row.
  assert.equal(url.parentElement, chrome);
  assert.equal(modeRow.parentElement, chrome);
  assert.ok(url.compareDocumentPosition(modeRow) & 4);

  // Device modes and tool icons share row 2; icons are right-justified.
  assert.ok(modeRow.contains(screen.getByTestId("link-side-panel-mode-desktop")));
  assert.ok(modeRow.contains(screen.getByTestId("link-side-panel-mode-responsive")));
  assert.ok(modeRow.contains(screen.getByTestId("link-side-panel-mode-mobile")));
  assert.ok(modeRow.contains(tools));
  assert.match(tools.className, /ml-auto/);
  assert.match(tools.className, /shrink-0/);

  for (const id of [
    "link-side-panel-back",
    "link-side-panel-forward",
    "link-side-panel-refresh",
    "link-side-panel-copy-url",
    "link-side-panel-inspect",
    "link-side-panel-expand",
  ]) {
    assert.ok(tools.contains(screen.getByTestId(id)), id);
  }

  // Tooling must not live on the URL row.
  assert.equal(url.contains(screen.getByTestId("link-side-panel-back")), false);
  assert.equal(url.contains(screen.getByTestId("link-side-panel-expand")), false);
});

test("expand control stays on the tooling row in both expanded states", async () => {
  const screen = await renderChrome({ expanded: true });
  const tools = screen.getByTestId("link-side-panel-tool-icons");
  const expand = screen.getByTestId("link-side-panel-expand");
  assert.ok(tools.contains(expand));
  assert.equal(expand.getAttribute("aria-label"), "Exit full screen");
});
