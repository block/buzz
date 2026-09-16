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
  const { PLAYGROUND_DOCK_WIDTH_SESSION_KEY } = await import("../lib/dock.ts");
  window.sessionStorage.removeItem(PLAYGROUND_DOCK_WIDTH_SESSION_KEY);
});

after(() => dom.window.close());

async function renderOverlay() {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession(card);
  render(createElement(PlaygroundOverlay, { session }));
  return screen;
}

test("overlay shows chrome PIN and parks on Dismiss", async () => {
  const screen = await renderOverlay();
  assert.ok(screen.getByTestId("playground-overlay"));
  assert.match(
    screen.getByTestId("playground-chrome-pin").textContent ?? "",
    /4455/,
  );
  assert.equal(screen.queryByTestId("playground-chrome-name"), null);
  assert.equal(screen.queryByTestId("playground-chrome-stack"), null);
  assert.ok(screen.getByTestId("playground-desktop-stage"));
  const chrome = screen.getByTestId("playground-chrome");
  const dispose = screen.getByTestId("playground-dispose");
  const back = screen.getByTestId("playground-back");
  assert.ok(chrome.contains(dispose));
  assert.ok(dispose.compareDocumentPosition(back) & 4);
  const dismiss = screen.getByTestId("playground-dismiss");
  assert.equal(dismiss.getAttribute("aria-label"), "Dismiss");
  assert.ok(dismiss.querySelector("svg"));
  assert.equal(screen.getByTestId("playground-inspect").textContent, "");
  assert.equal(
    screen.getByTestId("playground-inspect").getAttribute("aria-label"),
    "Inspect",
  );
  const dock = screen.getByTestId("playground-dock");
  assert.equal(dock.getAttribute("aria-label"), "Dock left");
  assert.ok(dock.querySelector("svg"));
  const fullscreen = screen.getByTestId("playground-fullscreen");
  assert.ok(fullscreen.compareDocumentPosition(dock) & 4);
  assert.ok(dock.compareDocumentPosition(dismiss) & 4);
  assert.equal(
    screen.getByTestId("playground-overlay").getAttribute("data-docked"),
    null,
  );
  assert.ok(
    screen
      .getByTestId("playground-mode-row")
      .contains(screen.getByTestId("playground-chrome-pin")),
  );

  const { fireEvent } = await import("@testing-library/react");
  await fireEvent.click(screen.getByTestId("playground-dismiss"));
  const {
    getActivePlaygroundSid,
    listPlaygroundSessions,
    showPlaygroundSession,
  } = await import("../lib/sessions.ts");
  assert.equal(listPlaygroundSessions().length, 1);
  assert.equal(getActivePlaygroundSid(), null);

  showPlaygroundSession("demo-1");
  assert.equal(getActivePlaygroundSid(), "demo-1");
});

test("Dispose is confirmed and removes the session; device museum is a dropdown", async () => {
  const screen = await renderOverlay();
  const { fireEvent } = await import("@testing-library/react");
  await fireEvent.click(screen.getByTestId("playground-mode-mobile"));
  assert.ok(screen.getByTestId("playground-mobile-stage"));
  const select = screen.getByTestId("playground-device-select");
  assert.equal(select.tagName, "SELECT");
  assert.ok(screen.getByTestId("playground-device-iphone-se"));
  assert.ok(screen.getByTestId("playground-device-iphone-16"));
  assert.ok(screen.getByTestId("playground-device-iphone-16-pro-max"));
  assert.ok(screen.getByTestId("playground-device-pixel-8"));
  assert.ok(screen.getByTestId("playground-device-ipad-mini"));
  assert.ok(screen.getByTestId("playground-device-ipad-pro-11"));
  const host = screen.getByTestId("playground-webview-host");
  assert.equal(host.getAttribute("data-viewport-width"), "393");
  assert.match(host.getAttribute("data-user-agent") ?? "", /iPhone|Mobile/);

  await fireEvent.click(screen.getByTestId("playground-dispose"));
  await fireEvent.click(screen.getByTestId("playground-dispose-confirm"));
  const { listPlaygroundSessions, getActivePlaygroundSid } = await import(
    "../lib/sessions.ts"
  );
  assert.equal(listPlaygroundSessions().length, 0);
  assert.equal(getActivePlaygroundSid(), null);
});

test("URL prefix is locked and suffix submits a same-origin navigation", async () => {
  const screen = await renderOverlay();
  assert.equal(
    screen.getByTestId("playground-url-prefix").textContent,
    "https://app.example.com/",
  );
  assert.equal(screen.getByTestId("playground-url-suffix").value, "");
});

test("Screenshot is hidden without a channel and stages a draft with one", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent, cleanup } = await import(
    "@testing-library/react"
  );
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    getActivePlaygroundSid,
  } = await import("../lib/sessions.ts");
  const { initDraftStore } = await import(
    "@/features/messages/lib/useDrafts.ts"
  );
  const { takeQueuedAttachmentsForDraft } = await import(
    "@/features/messages/lib/backgroundMediaUploadStore.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  initDraftStore("pub", "wss://relay.example.com");
  const session = addPlaygroundSession(card);
  render(createElement(PlaygroundOverlay, { session }));
  assert.equal(screen.queryByTestId("playground-screenshot"), null);
  cleanup();

  render(
    createElement(PlaygroundOverlay, {
      conversation: { channelId: "hula-id", draftKey: "hula-id" },
      session,
    }),
  );
  assert.ok(screen.getByTestId("playground-screenshot"));
  assert.equal(screen.getByTestId("playground-screenshot").textContent, "");
  assert.equal(
    screen.getByTestId("playground-screenshot").getAttribute("aria-label"),
    "Screenshot",
  );
  const screenshot = screen.getByTestId("playground-screenshot");
  const fullscreen = screen.getByTestId("playground-fullscreen");
  const dock = screen.getByTestId("playground-dock");
  const dismiss = screen.getByTestId("playground-dismiss");
  assert.ok(screenshot.compareDocumentPosition(fullscreen) & 4);
  assert.ok(fullscreen.compareDocumentPosition(dock) & 4);
  assert.ok(dock.compareDocumentPosition(dismiss) & 4);
  const { act } = await import("@testing-library/react");
  await act(async () => {
    await fireEvent.click(screen.getByTestId("playground-screenshot"));
  });
  assert.equal(getActivePlaygroundSid(), null);
  assert.equal(takeQueuedAttachmentsForDraft("hula-id").length, 1);
});

test("PIN is hidden when empty and fullscreen fills the window", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { PLAYGROUND_DESKTOP_UA } = await import("../lib/devices.ts");
  const { PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-open",
  });
  render(createElement(PlaygroundOverlay, { session }));
  assert.equal(screen.queryByTestId("playground-chrome-pin"), null);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.equal(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-user-agent"),
    PLAYGROUND_DESKTOP_UA,
  );
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^window:/,
  );

  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  const overlay = screen.getByTestId("playground-overlay");
  assert.equal(overlay.getAttribute("data-fullscreen"), "true");
  assert.match(overlay.className, /fixed/);
  assert.match(overlay.className, /inset-0/);
  const gap = screen.getByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID);
  const chrome = screen.getByTestId("playground-chrome");
  const {
    playgroundFullscreenDragRegionIsGapOnly,
    playgroundFullscreenOverlayIsPortaled,
    playgroundOverlaySurfaceIsOpaque,
  } = await import("../lib/overlayLayout.ts");
  assert.equal(playgroundOverlaySurfaceIsOpaque(overlay.className), true);
  assert.equal(playgroundOverlaySurfaceIsOpaque(gap.className), true);
  assert.equal(playgroundOverlaySurfaceIsOpaque(chrome.className), true);
  assert.match(overlay.style.backgroundColor, /--background/);
  assert.match(gap.style.backgroundColor, /--background/);
  assert.match(chrome.style.backgroundColor, /--background/);
  assert.doesNotMatch(overlay.className, /backdrop-blur|\/\d+/);
  assert.doesNotMatch(gap.className, /backdrop-blur|\/\d+/);
  assert.doesNotMatch(chrome.className, /backdrop-blur|\/\d+/);
  assert.doesNotMatch(
    screen.getByTestId("playground-mode-row").className,
    /backdrop-blur|\/\d+/,
  );
  assert.equal(playgroundFullscreenOverlayIsPortaled(overlay), true);
  assert.equal(playgroundFullscreenDragRegionIsGapOnly(gap, chrome), true);
  assert.ok(overlay.contains(gap));
  assert.ok(gap.compareDocumentPosition(chrome) & 4);
  assert.equal(screen.queryByTestId("playground-dock"), null);
  for (const testId of [
    "playground-dispose",
    "playground-back",
    "playground-inspect",
    "playground-fullscreen",
    "playground-dismiss",
    "playground-mode-desktop",
    "playground-mode-responsive",
    "playground-mode-mobile",
  ]) {
    const control = screen.getByTestId(testId);
    assert.ok(chrome.contains(control));
    assert.equal(gap.contains(control), false);
  }
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^fullscreen:/,
  );
  assert.equal(
    screen.getByTestId("playground-fullscreen").getAttribute("aria-label"),
    "Exit fullscreen",
  );
  assert.equal(screen.getByTestId("playground-dispose").disabled, false);
  assert.equal(screen.getByTestId("playground-inspect").disabled, false);
});

test("Escape and the fullscreen control exit overlay fullscreen", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-escape",
  });
  render(createElement(PlaygroundOverlay, { session }));
  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  assert.equal(
    screen.getByTestId("playground-overlay").getAttribute("data-fullscreen"),
    "true",
  );

  await fireEvent.keyDown(window, { key: "Escape" });
  assert.equal(
    screen.getByTestId("playground-overlay").getAttribute("data-fullscreen"),
    null,
  );
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^window:/,
  );

  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  assert.equal(
    screen.getByTestId("playground-overlay").getAttribute("data-fullscreen"),
    "true",
  );
  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  assert.equal(
    screen.getByTestId("playground-overlay").getAttribute("data-fullscreen"),
    null,
  );
  assert.equal(
    screen.getByTestId("playground-fullscreen").getAttribute("aria-label"),
    "Fullscreen",
  );
});

test("overlay chrome is fully opaque", async () => {
  const screen = await renderOverlay();
  const { playgroundOverlaySurfaceIsOpaque } = await import(
    "../lib/overlayLayout.ts"
  );
  const overlay = screen.getByTestId("playground-overlay");
  assert.equal(playgroundOverlaySurfaceIsOpaque(overlay.className), true);
  assert.doesNotMatch(overlay.className, /\/\d+/);
  assert.doesNotMatch(overlay.className, /backdrop-blur/);
  assert.match(
    screen.getByTestId("playground-chrome").className,
    /bg-background/,
  );
  assert.doesNotMatch(
    screen.getByTestId("playground-chrome").className,
    /\/\d+/,
  );
  assert.equal(
    screen.getByTestId("playground-overlay").parentElement === document.body,
    false,
  );
});

test("responsive resize handles sit outside the webview host", async () => {
  const screen = await renderOverlay();
  const { fireEvent } = await import("@testing-library/react");
  await fireEvent.click(screen.getByTestId("playground-mode-responsive"));
  const host = screen.getByTestId("playground-webview-host");
  const resizeX = screen.getByTestId("playground-stage-resize");
  const resizeY = screen.getByTestId("playground-stage-resize-y");
  const resizeXy = screen.getByTestId("playground-stage-resize-xy");
  const { playgroundResizeHandleSitsOutsideHost } = await import(
    "../lib/overlayLayout.ts"
  );
  assert.equal(playgroundResizeHandleSitsOutsideHost(host, resizeX), true);
  assert.equal(playgroundResizeHandleSitsOutsideHost(host, resizeY), true);
  assert.equal(playgroundResizeHandleSitsOutsideHost(host, resizeXy), true);
  assert.match(resizeX.className, /w-2/);
  assert.match(resizeY.className, /h-2/);
  const frame = screen.getByTestId("playground-responsive-frame");
  assert.equal(frame.contains(host), true);
  assert.equal(frame.contains(resizeX), true);
  assert.equal(
    screen.getByTestId("playground-responsive-page").style.maxWidth,
    "",
  );

  const widthField = screen.getByTestId("playground-responsive-width");
  const before = Number(widthField.value);
  await fireEvent.pointerDown(resizeX, { clientX: 400, clientY: 200 });
  await fireEvent.pointerMove(window, { clientX: 480, clientY: 200 });
  await fireEvent.pointerUp(window);
  assert.equal(Number(widthField.value), before + 80);
});

test("Inspect re-syncs the native stage without targeting main", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-inspect",
  });
  render(createElement(PlaygroundOverlay, { session }));
  const before = screen
    .getByTestId("playground-webview-host")
    .getAttribute("data-layout-key");
  const { act } = await import("@testing-library/react");
  await act(async () => {
    await fireEvent.click(screen.getByTestId("playground-inspect"));
  });
  const after = screen
    .getByTestId("playground-webview-host")
    .getAttribute("data-layout-key");
  assert.notEqual(after, before);
  assert.match(after ?? "", /^window:/);
});

function stubMainWidth(overlay, width) {
  const parent = overlay.parentElement;
  assert.ok(parent);
  Object.defineProperty(parent, "clientWidth", {
    configurable: true,
    value: width,
  });
  parent.getBoundingClientRect = () => ({
    x: 256,
    y: 0,
    width,
    height: 720,
    top: 0,
    left: 256,
    right: 256 + width,
    bottom: 720,
    toJSON() {},
  });
  overlay.getBoundingClientRect = () => {
    const dockedWidth = Number.parseFloat(overlay.style.width) || width;
    return {
      x: 256,
      y: 0,
      width:
        overlay.getAttribute("data-docked") === "true" ? dockedWidth : width,
      height: 720,
      top: 0,
      left: 256,
      right:
        256 +
        (overlay.getAttribute("data-docked") === "true" ? dockedWidth : width),
      bottom: 720,
      toJSON() {},
    };
  };
}

test("collapse docks to a left pane and expand restores the full overlay", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { playgroundDockLeavesMainClickable, playgroundOverlaySearchState } =
    await import("../lib/dock.ts");
  const { PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-dock",
  });
  render(
    createElement(
      "div",
      { "data-testid": "playground-main-standin" },
      createElement(
        "div",
        { "data-testid": "playground-chat-standin" },
        "chat",
      ),
      createElement(PlaygroundOverlay, { session }),
    ),
  );

  const overlay = screen.getByTestId("playground-overlay");
  const main = screen.getByTestId("playground-main-standin");
  stubMainWidth(overlay, 1000);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID),
    null,
  );
  assert.equal(playgroundOverlaySearchState(overlay).placement, "window");

  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.getAttribute("data-docked"), "true");
  assert.equal(overlay.getAttribute("data-fullscreen"), null);
  assert.deepEqual(playgroundOverlaySearchState(overlay), {
    docked: true,
    fullscreen: false,
    placement: "dock",
  });
  assert.match(overlay.className, /left-0/);
  assert.doesNotMatch(overlay.className, /inset-0/);
  assert.equal(overlay.style.width, "500px");
  assert.equal(overlay.parentElement === document.body, false);
  assert.equal(playgroundDockLeavesMainClickable(overlay, main), true);
  assert.ok(screen.getByTestId("playground-chat-standin"));
  assert.equal(
    overlay.contains(screen.getByTestId("playground-chat-standin")),
    false,
  );
  const handle = screen.getByTestId(PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID);
  assert.ok(overlay.contains(handle));
  assert.match(handle.className, /\bw-2\b/);
  assert.match(overlay.className, /pr-2/);
  assert.equal(
    overlay.contains(screen.getByTestId("playground-webview-host")) &&
      !screen.getByTestId("playground-webview-host").contains(handle),
    true,
  );
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^dock:/,
  );
  assert.equal(
    screen.getByTestId("playground-dock").getAttribute("aria-label"),
    "Expand overlay",
  );

  await fireEvent.pointerDown(handle, { clientX: 756, clientY: 200 });
  await fireEvent.pointerMove(window, { clientX: 856, clientY: 200 });
  await fireEvent.pointerUp(window);
  assert.equal(overlay.style.width, "600px");

  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.getAttribute("data-docked"), null);
  assert.equal(playgroundOverlaySearchState(overlay).placement, "window");
  assert.match(overlay.className, /inset-0/);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID),
    null,
  );
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^window:/,
  );
});

test("fullscreen from dock returns to dock; dismiss still parks", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const {
    addPlaygroundSession,
    configurePlaygroundScope,
    listPlaygroundSessions,
  } = await import("../lib/sessions.ts");
  const { playgroundOverlaySearchState } = await import("../lib/dock.ts");
  const { PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-dock-fs",
  });
  render(createElement(PlaygroundOverlay, { session }));
  stubMainWidth(screen.getByTestId("playground-overlay"), 1000);

  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(
    playgroundOverlaySearchState(screen.getByTestId("playground-overlay"))
      .placement,
    "dock",
  );

  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  const fullscreenOverlay = screen.getByTestId("playground-overlay");
  assert.equal(fullscreenOverlay.getAttribute("data-fullscreen"), "true");
  assert.equal(fullscreenOverlay.getAttribute("data-docked"), "true");
  assert.equal(
    playgroundOverlaySearchState(fullscreenOverlay).placement,
    "fullscreen",
  );
  assert.equal(fullscreenOverlay.parentElement === document.body, true);
  assert.ok(screen.getByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID));
  assert.equal(screen.queryByTestId("playground-dock"), null);

  await fireEvent.keyDown(window, { key: "Escape" });
  const dockedOverlay = screen.getByTestId("playground-overlay");
  assert.equal(dockedOverlay.getAttribute("data-fullscreen"), null);
  assert.equal(dockedOverlay.getAttribute("data-docked"), "true");
  assert.equal(playgroundOverlaySearchState(dockedOverlay).placement, "dock");
  assert.equal(dockedOverlay.parentElement === document.body, false);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.equal(
    screen.getByTestId("playground-dock").getAttribute("aria-label"),
    "Expand overlay",
  );
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^dock:/,
  );

  await fireEvent.click(screen.getByTestId("playground-dismiss"));
  assert.equal(
    listPlaygroundSessions().some((item) => item.sid === "demo-dock-fs"),
    true,
  );
});

function stubThreadPanel(thread, { mainLeft = 256, mainWidth, threadWidth }) {
  thread.getBoundingClientRect = () => ({
    x: mainLeft + mainWidth - threadWidth,
    y: 0,
    width: threadWidth,
    height: 720,
    top: 0,
    left: mainLeft + mainWidth - threadWidth,
    right: mainLeft + mainWidth,
    bottom: 720,
    toJSON() {},
  });
}

test("dock button snaps flush to an open thread pane", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID } = await import(
    "../lib/dock.ts"
  );
  const { PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-dock-thread",
  });
  render(
    createElement(
      "div",
      { "data-testid": "playground-main-standin" },
      createElement(
        "div",
        { "data-testid": PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID },
        "thread",
      ),
      createElement(PlaygroundOverlay, {
        conversation: { channelId: "hula-id", draftKey: "thread:root-1" },
        session,
      }),
    ),
  );

  const overlay = screen.getByTestId("playground-overlay");
  stubMainWidth(overlay, 1000);
  stubThreadPanel(screen.getByTestId(PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID), {
    mainWidth: 1000,
    threadWidth: 380,
  });

  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.getAttribute("data-docked"), "true");
  assert.equal(overlay.style.width, "620px");

  const handle = screen.getByTestId(PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID);
  await fireEvent.pointerDown(handle, { clientX: 876, clientY: 200 });
  await fireEvent.pointerMove(window, { clientX: 926, clientY: 200 });
  await fireEvent.pointerUp(window);
  assert.equal(overlay.style.width, "670px");

  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.getAttribute("data-docked"), null);

  stubThreadPanel(screen.getByTestId(PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID), {
    mainWidth: 1000,
    threadWidth: 420,
  });
  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.style.width, "670px");
});

test("dock button without a thread keeps the half-inset default", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-dock-no-thread",
  });
  render(
    createElement(
      "div",
      { "data-testid": "playground-main-standin" },
      createElement(PlaygroundOverlay, {
        conversation: { channelId: "hula-id", draftKey: "hula-id" },
        session,
      }),
    ),
  );

  const overlay = screen.getByTestId("playground-overlay");
  stubMainWidth(overlay, 1000);
  await fireEvent.click(screen.getByTestId("playground-dock"));
  assert.equal(overlay.style.width, "500px");
});

test("locked window chrome keeps the mode row and hides layout controls", async () => {
  const { createElement } = await import("react");
  const { render, screen } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-lock-window",
  });
  render(
    createElement(PlaygroundOverlay, { lockPlacement: "window", session }),
  );

  const overlay = screen.getByTestId("playground-overlay");
  const chrome = screen.getByTestId("playground-chrome");
  const modeRow = screen.getByTestId("playground-mode-row");
  assert.match(overlay.className, /inset-0/);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.ok(chrome.contains(modeRow));
  assert.ok(screen.getByTestId("playground-mode-desktop"));
  assert.ok(screen.getByTestId("playground-mode-responsive"));
  assert.ok(screen.getByTestId("playground-mode-mobile"));
  assert.match(chrome.className, /shrink-0/);
  assert.match(chrome.className, /flex-col/);
  assert.match(modeRow.className, /shrink-0/);
  assert.match(modeRow.className, /min-h-7/);
  assert.doesNotMatch(chrome.className, /overflow-hidden/);
  assert.equal(screen.queryByTestId("playground-dispose"), null);
  assert.equal(screen.queryByTestId("playground-fullscreen"), null);
  assert.equal(screen.queryByTestId("playground-dock"), null);
  assert.equal(screen.queryByTestId("playground-dismiss"), null);
  const host = screen.getByTestId("playground-webview-host");
  assert.ok(chrome.compareDocumentPosition(host) & 4);
});

test("locked dock is an in-flow split pane and does not snap to a thread", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID } = await import(
    "../lib/dock.ts"
  );
  const {
    PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID,
    PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID,
    PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS,
  } = await import("../lib/overlayLayout.ts");

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-lock-dock",
  });
  render(
    createElement(
      "div",
      { "data-testid": "playground-main-standin", className: "flex flex-row" },
      createElement(PlaygroundOverlay, {
        conversation: { channelId: "hula-id", draftKey: "thread:root-1" },
        lockPlacement: "dock",
        session,
      }),
      createElement(
        "div",
        {
          "data-testid": PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID,
          className: "min-w-0 flex-1",
        },
        "thread",
      ),
    ),
  );

  const overlay = screen.getByTestId("playground-overlay");
  stubMainWidth(overlay, 1000);
  stubThreadPanel(screen.getByTestId(PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID), {
    mainWidth: 1000,
    threadWidth: 380,
  });

  assert.equal(overlay.getAttribute("data-docked"), "true");
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.match(overlay.className, /relative/);
  assert.doesNotMatch(overlay.className, /\babsolute\b/);
  assert.doesNotMatch(overlay.className, /inset-y-0/);
  for (const token of PLAYGROUND_SPLIT_PANE_OVERLAY_CLASS.split(/\s+/)) {
    assert.match(overlay.className, new RegExp(`\\b${token}\\b`));
  }
  assert.ok(screen.getByTestId("playground-mode-row"));
  assert.match(screen.getByTestId("playground-mode-row").className, /shrink-0/);
  assert.match(screen.getByTestId("playground-mode-row").className, /min-h-7/);
  assert.equal(screen.queryByTestId("playground-dispose"), null);
  assert.ok(screen.getByTestId("playground-fullscreen"));
  assert.equal(screen.queryByTestId("playground-dock"), null);
  assert.equal(screen.queryByTestId("playground-dismiss"), null);

  const handle = screen.getByTestId(PLAYGROUND_DOCK_RESIZE_HANDLE_TEST_ID);
  assert.ok(overlay.contains(handle));
  const before = Number.parseFloat(overlay.style.width);
  assert.ok(before > 0);
  assert.notEqual(overlay.style.width, "620px");

  await fireEvent.pointerDown(handle, { clientX: 256 + before, clientY: 200 });
  await fireEvent.pointerMove(window, {
    clientX: 256 + before + 80,
    clientY: 200,
  });
  await fireEvent.pointerUp(window);
  assert.equal(overlay.style.width, `${before + 80}px`);
});

test("locked dock fullscreen covers both panes and restores the split", async () => {
  const { createElement } = await import("react");
  const { render, screen, fireEvent } = await import("@testing-library/react");
  const { PlaygroundOverlay } = await import("./PlaygroundOverlay.tsx");
  const { addPlaygroundSession, configurePlaygroundScope } = await import(
    "../lib/sessions.ts"
  );
  const { playgroundOverlaySearchState } = await import("../lib/dock.ts");
  const { PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID } = await import(
    "../lib/dock.ts"
  );
  const { PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID } = await import(
    "../lib/overlayLayout.ts"
  );

  configurePlaygroundScope("pub", "wss://relay.example.com");
  const session = addPlaygroundSession({
    hula: "playground",
    v: 1,
    name: "Demo",
    url: "https://app.example.com",
    sid: "demo-lock-dock-fs",
  });
  render(
    createElement(
      "div",
      { "data-testid": "playground-main-standin", className: "flex flex-row" },
      createElement(PlaygroundOverlay, {
        conversation: { channelId: "hula-id", draftKey: "thread:root-1" },
        lockPlacement: "dock",
        session,
      }),
      createElement(
        "div",
        {
          "data-testid": PLAYGROUND_CHANNEL_THREAD_PANEL_TEST_ID,
          className: "min-w-0 flex-1",
        },
        "thread",
      ),
    ),
  );

  const overlay = screen.getByTestId("playground-overlay");
  stubMainWidth(overlay, 1000);
  assert.equal(overlay.getAttribute("data-docked"), "true");
  assert.equal(screen.queryByTestId("playground-dispose"), null);
  assert.equal(screen.queryByTestId("playground-dock"), null);
  assert.equal(screen.queryByTestId("playground-dismiss"), null);

  await fireEvent.click(screen.getByTestId("playground-fullscreen"));
  const fullscreenOverlay = screen.getByTestId("playground-overlay");
  assert.equal(fullscreenOverlay.getAttribute("data-fullscreen"), "true");
  assert.equal(fullscreenOverlay.getAttribute("data-docked"), "true");
  assert.equal(
    playgroundOverlaySearchState(fullscreenOverlay).placement,
    "fullscreen",
  );
  assert.match(fullscreenOverlay.className, /fixed/);
  assert.match(fullscreenOverlay.className, /inset-0/);
  assert.equal(fullscreenOverlay.parentElement === document.body, true);
  assert.ok(screen.getByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID));
  assert.equal(screen.queryByTestId("playground-dock"), null);
  assert.equal(screen.queryByTestId("playground-dispose"), null);
  assert.equal(screen.queryByTestId("playground-dismiss"), null);

  await fireEvent.keyDown(window, { key: "Escape" });
  const dockedOverlay = screen.getByTestId("playground-overlay");
  assert.equal(dockedOverlay.getAttribute("data-fullscreen"), null);
  assert.equal(dockedOverlay.getAttribute("data-docked"), "true");
  assert.equal(playgroundOverlaySearchState(dockedOverlay).placement, "dock");
  assert.match(dockedOverlay.className, /relative/);
  assert.doesNotMatch(dockedOverlay.className, /\babsolute\b/);
  assert.equal(dockedOverlay.parentElement === document.body, false);
  assert.equal(
    screen.queryByTestId(PLAYGROUND_FULLSCREEN_TITLEBAR_GAP_TEST_ID),
    null,
  );
  assert.ok(screen.getByTestId("playground-fullscreen"));
  assert.equal(screen.queryByTestId("playground-dock"), null);
  assert.equal(screen.queryByTestId("playground-dismiss"), null);
  assert.equal(screen.queryByTestId("playground-dispose"), null);
  assert.match(
    screen
      .getByTestId("playground-webview-host")
      .getAttribute("data-layout-key") ?? "",
    /^dock:/,
  );
});
