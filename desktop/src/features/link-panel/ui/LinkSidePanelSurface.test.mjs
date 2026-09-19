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
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

async function renderSurface(viewportMode = "desktop") {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { LinkSidePanelSurface } = await import("./LinkSidePanelSurface.tsx");
  render(
    createElement(LinkSidePanelSurface, {
      url: "https://example.com/",
      viewportMode,
    }),
  );
  return screen;
}

test("desktop surface fills host without fixed frame chrome", async () => {
  const screen = await renderSurface("desktop");
  const surface = screen.getByTestId("link-side-panel-surface");
  assert.equal(surface.getAttribute("data-viewport-mode"), "desktop");
  assert.match(surface.className, /h-full/);
  assert.match(surface.className, /flex-1/);
  assert.ok(screen.getByTestId("link-side-panel-webview-host"));
});

test("responsive surface exposes W×H inputs and side drag handles", async () => {
  const screen = await renderSurface("responsive");
  assert.equal(
    screen.getByTestId("link-side-panel-surface").getAttribute("data-viewport-mode"),
    "responsive",
  );
  assert.ok(screen.getByTestId("link-side-panel-responsive-width"));
  assert.ok(screen.getByTestId("link-side-panel-responsive-height"));
  assert.ok(screen.getByTestId("link-side-panel-resize"));
  assert.ok(screen.getByTestId("link-side-panel-resize-y"));
  assert.ok(screen.getByTestId("link-side-panel-resize-xy"));
});

test("mobile surface paints device museum + hardware bezel", async () => {
  const screen = await renderSurface("mobile");
  assert.equal(
    screen.getByTestId("link-side-panel-surface").getAttribute("data-viewport-mode"),
    "mobile",
  );
  assert.ok(screen.getByTestId("link-side-panel-device-select"));
  assert.ok(screen.getByTestId("link-side-panel-device-iphone-16"));
  assert.ok(screen.getByTestId("playground-device-frame"));
  assert.ok(screen.getByTestId("link-side-panel-orientation"));
  const backdrop = screen.getByTestId("link-side-panel-mobile-backdrop");
  assert.ok(backdrop.className.includes("bg-white"));
});

test("mobile surface exposes orientation and scale controls on white backdrop", async () => {
  const screen = await renderSurface("mobile");
  const { fireEvent } = await import("@testing-library/react");
  assert.ok(screen.getByTestId("link-side-panel-orientation"));
  assert.equal(screen.getByTestId("link-side-panel-device-scale-value").textContent, "100%");
  await fireEvent.click(screen.getByTestId("link-side-panel-device-scale-up"));
  assert.equal(screen.getByTestId("link-side-panel-device-scale-value").textContent, "125%");
  assert.equal(
    screen.getByTestId("playground-device-frame").getAttribute("data-scale"),
    "125",
  );
  assert.ok(
    screen.getByTestId("link-side-panel-mobile-backdrop").className.includes("bg-white"),
  );
});
