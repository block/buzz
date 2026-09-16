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
  sid: "demo-stage",
};

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    Blob: dom.window.Blob,
    File: dom.window.File,
    document: dom.window.document,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
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
  if (typeof dom.window.URL.createObjectURL !== "function") {
    dom.window.URL.createObjectURL = () => "blob:playground-test";
    dom.window.URL.revokeObjectURL = () => undefined;
  }
  globalThis.URL = dom.window.URL;
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.requestAnimationFrame = (callback) => {
    callback(0);
    return 0;
  };
  dom.window.cancelAnimationFrame = () => undefined;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetPlaygroundState } = await import("../lib/sessions.ts");
  resetPlaygroundState();
  delete globalThis.isTauri;
  delete window.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

async function renderStage(mode = "mobile") {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { PlaygroundStage } = await import("./PlaygroundStage.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession(card);
  render(createElement(PlaygroundStage, { mode, session }));
  return screen;
}

test("mobile stage paints a hardware bezel around a smaller inner screen", async () => {
  const screen = await renderStage("mobile");
  const frame = screen.getByTestId("playground-device-frame");
  const inner = screen.getByTestId("playground-device-screen");
  const host = screen.getByTestId("playground-webview-host");
  assert.equal(frame.getAttribute("data-bezel"), "hardware");
  assert.equal(frame.getAttribute("data-family"), "iphone");
  assert.equal(frame.getAttribute("data-chrome"), "iphone-island");
  assert.ok(inner.contains(host));
  assert.ok(frame.contains(inner));
  assert.equal(inner.style.width, "393px");
  assert.equal(inner.style.height, "852px");
  assert.ok(Number.parseFloat(frame.style.width) > 393);
  assert.ok(Number.parseFloat(frame.style.height) > 852);
  assert.ok(Number.parseFloat(frame.style.borderRadius) >= 48);
  assert.equal(host.getAttribute("data-viewport-width"), "393");
  assert.equal(host.getAttribute("data-viewport-height"), "852");
  assert.ok(screen.getByTestId("playground-device-island"));
  assert.ok(screen.getByTestId("playground-device-home-indicator"));
});

test("native bounds follow the inner screen host, not the outer bezel", async () => {
  const screen = await renderStage("mobile");
  const host = screen.getByTestId("playground-webview-host");
  const frame = screen.getByTestId("playground-device-frame");
  host.getBoundingClientRect = () => ({
    x: 64,
    y: 96,
    width: 393,
    height: 852,
    top: 96,
    left: 64,
    right: 457,
    bottom: 948,
    toJSON() {},
  });
  frame.getBoundingClientRect = () => ({
    x: 20,
    y: 20,
    width: 425,
    height: 920,
    top: 20,
    left: 20,
    right: 445,
    bottom: 940,
    toJSON() {},
  });
  const { readPlaygroundStageBounds } = await import("../lib/deviceBezel.ts");
  const bounds = readPlaygroundStageBounds(host, {
    width: 393,
    height: 852,
  });
  assert.deepEqual(bounds, { x: 64, y: 96, width: 393, height: 852 });
  assert.notEqual(bounds.width, frame.getBoundingClientRect().width);
  assert.notEqual(bounds.x, frame.getBoundingClientRect().x);
});

test("iPhone family shows island or home-button chrome; Pixel does not", async () => {
  const screen = await renderStage("mobile");
  const { fireEvent } = await import("@testing-library/react");
  assert.ok(screen.getByTestId("playground-device-island"));

  await fireEvent.change(screen.getByTestId("playground-device-select"), {
    target: { value: "iphone-se" },
  });
  assert.equal(screen.queryByTestId("playground-device-island"), null);
  assert.ok(screen.getByTestId("playground-device-home-button"));
  assert.equal(
    screen.getByTestId("playground-device-frame").getAttribute("data-chrome"),
    "iphone-home-button",
  );

  await fireEvent.change(screen.getByTestId("playground-device-select"), {
    target: { value: "pixel-8" },
  });
  assert.equal(screen.queryByTestId("playground-device-island"), null);
  assert.ok(screen.getByTestId("playground-device-punch"));
  assert.equal(
    screen.getByTestId("playground-device-frame").getAttribute("data-family"),
    "pixel",
  );

  await fireEvent.change(screen.getByTestId("playground-device-select"), {
    target: { value: "ipad-mini" },
  });
  assert.equal(screen.queryByTestId("playground-device-island"), null);
  assert.ok(screen.getByTestId("playground-device-camera"));
  assert.equal(
    screen.getByTestId("playground-device-frame").getAttribute("data-family"),
    "ipad",
  );
});

test("landscape moves island chrome onto the long edge", async () => {
  const screen = await renderStage("mobile");
  const { fireEvent } = await import("@testing-library/react");
  await fireEvent.click(screen.getByTestId("playground-orientation"));
  const frame = screen.getByTestId("playground-device-frame");
  const inner = screen.getByTestId("playground-device-screen");
  const island = screen.getByTestId("playground-device-island");
  assert.equal(frame.getAttribute("data-orientation"), "landscape");
  assert.equal(inner.style.width, "852px");
  assert.equal(inner.style.height, "393px");
  assert.ok(Number.parseFloat(frame.style.width) > 852);
  assert.equal(island.style.left !== "", true);
  assert.match(island.style.transform, /translateY/);
});

test("desktop and responsive stages stay unbezeled rectangles", async () => {
  const desktop = await renderStage("desktop");
  assert.ok(desktop.getByTestId("playground-desktop-stage"));
  assert.equal(desktop.queryByTestId("playground-device-frame"), null);
  assert.equal(desktop.queryByTestId("playground-device-island"), null);

  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetPlaygroundState } = await import("../lib/sessions.ts");
  resetPlaygroundState();

  const responsive = await renderStage("responsive");
  assert.ok(responsive.getByTestId("playground-responsive-stage"));
  assert.ok(responsive.getByTestId("playground-responsive-page"));
  assert.equal(responsive.queryByTestId("playground-device-frame"), null);
  assert.ok(responsive.getByTestId("playground-stage-resize"));
});

test("clamp native bounds below playground chrome", async () => {
  const { readPlaygroundStageBounds } = await import("../lib/deviceBezel.ts");
  const host = {
    getBoundingClientRect: () => ({
      x: 8,
      y: 48,
      width: 640,
      height: 400,
      bottom: 448,
    }),
  };
  const chrome = {
    getBoundingClientRect: () => ({ bottom: 80 }),
  };
  assert.deepEqual(readPlaygroundStageBounds(host, undefined, chrome), {
    x: 8,
    y: 80,
    width: 640,
    height: 368,
  });
});

test("unmounting the stage hides the window-scoped webview", async () => {
  const invokes = [];
  const internals = {
    invoke(cmd, args) {
      invokes.push({ cmd, args });
      return Promise.resolve({
        sid: args?.sid,
        canGoBack: false,
        canGoForward: false,
        currentUrl: "",
      });
    },
  };
  globalThis.isTauri = true;
  window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  window.__TAURI_INTERNALS__ = internals;
  const { createElement } = await import("react");
  const { render, cleanup } = await import("@testing-library/react");
  const { PlaygroundStage } = await import("./PlaygroundStage.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession(card);
  render(createElement(PlaygroundStage, { mode: "desktop", session }));
  cleanup();
  await Promise.resolve();
  const hide = invokes.find((row) => row.cmd === "playground_webview_hide");
  assert.ok(hide);
  assert.equal(hide.args.sid, "demo-stage");
  assert.equal(hide.args.windowLabel, "main");
  delete globalThis.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
});
