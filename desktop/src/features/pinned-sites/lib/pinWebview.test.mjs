import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import {
  MIN_PIN_WEBVIEW_EDGE,
  pinHideCloseIsWindowScoped,
  pinWebviewBoundsAreUsable,
  pinWebviewLabelForWindow,
} from "./pinWebview.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
  });
});

afterEach(() => {
  delete globalThis.isTauri;
  delete dom.window.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

test("pinWebviewBoundsAreUsable rejects a 1×1 first layout", () => {
  assert.equal(MIN_PIN_WEBVIEW_EDGE, 32);
  assert.equal(
    pinWebviewBoundsAreUsable({ x: 0, y: 0, width: 1, height: 1 }),
    false,
  );
  assert.equal(
    pinWebviewBoundsAreUsable({ x: 0, y: 0, width: 16, height: 400 }),
    false,
  );
  assert.equal(
    pinWebviewBoundsAreUsable({ x: 240, y: 36, width: 800, height: 600 }),
    true,
  );
});

function installTauriInvoke(handler) {
  const internals = { invoke: handler };
  globalThis.isTauri = true;
  dom.window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  dom.window.__TAURI_INTERNALS__ = internals;
  return internals;
}

test("showPinWebview re-hides when hide-all wins while show is in flight", async () => {
  const calls = [];
  let resolveShow;
  const showGate = new Promise((resolve) => {
    resolveShow = resolve;
  });
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "pin_webview_show") {
      return showGate.then(() => ({
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.startUrl,
      }));
    }
    return Promise.resolve(undefined);
  });

  const {
    getPinHideEpoch,
    hideAllPinWebviews,
    showPinWebview,
  } = await import("./pinWebview.ts");

  const epochBefore = getPinHideEpoch();
  const pending = showPinWebview({
    pinId: "pin-race",
    startUrl: "https://example.com",
    bounds: { x: 0, y: 40, width: 800, height: 600 },
  });

  await hideAllPinWebviews();
  assert.equal(getPinHideEpoch(), epochBefore + 1);
  assert.ok(calls.some((row) => row.cmd === "pin_webview_hide_all"));

  resolveShow();
  await pending;

  const hides = calls.filter((row) => row.cmd === "pin_webview_hide");
  assert.equal(hides.length, 1);
  assert.equal(hides[0].args.pinId, "pin-race");
  assert.equal(getPinHideEpoch(), epochBefore + 1);
});

test("stale show re-hide does not invalidate a remounted pin show", async () => {
  const calls = [];
  let resolveOld;
  let resolveNew;
  const oldGate = new Promise((resolve) => {
    resolveOld = resolve;
  });
  const newGate = new Promise((resolve) => {
    resolveNew = resolve;
  });
  let showCount = 0;
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "pin_webview_show") {
      showCount += 1;
      const gate = showCount === 1 ? oldGate : newGate;
      return gate.then(() => ({
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.startUrl,
      }));
    }
    return Promise.resolve(undefined);
  });

  const { getPinShowGeneration, hidePinWebview, showPinWebview } = await import(
    "./pinWebview.ts"
  );

  const first = showPinWebview({
    pinId: "pin-remount",
    startUrl: "https://a.example",
    bounds: { x: 0, y: 40, width: 800, height: 600 },
  });
  await hidePinWebview("pin-remount");
  const second = showPinWebview({
    pinId: "pin-remount",
    startUrl: "https://b.example",
    bounds: { x: 0, y: 40, width: 800, height: 600 },
  });

  // Newer show finishes first (typical when reuse_existing is fast), then the
  // stale show resolves — must NOT re-hide the remounted paint.
  resolveNew();
  await second;
  const hidesAfterNew = calls.filter((row) => row.cmd === "pin_webview_hide")
    .length;

  resolveOld();
  await first;

  const hides = calls.filter((row) => row.cmd === "pin_webview_hide");
  // Only the explicit remount hide — stale show must skip epoch re-hide.
  assert.equal(hides.length, hidesAfterNew);
  assert.equal(hides.length, 1);
  assert.equal(calls.filter((row) => row.cmd === "pin_webview_show").length, 2);
  assert.equal(getPinShowGeneration("pin-remount"), 2);
});

test("first-open remount: late cancelled show does not blank the new show", async () => {
  const calls = [];
  let resolveFirst;
  let resolveSecond;
  const firstGate = new Promise((resolve) => {
    resolveFirst = resolve;
  });
  const secondGate = new Promise((resolve) => {
    resolveSecond = resolve;
  });
  let showCount = 0;
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "pin_webview_show") {
      showCount += 1;
      const gate = showCount === 1 ? firstGate : secondGate;
      return gate.then(() => ({
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.startUrl,
      }));
    }
    return Promise.resolve(undefined);
  });

  const {
    closePinWebview,
    getPinHideEpoch,
    getPinShowGeneration,
    showPinWebview,
  } = await import("./pinWebview.ts");

  const pinId = "hula-link-side-panel";
  const first = showPinWebview({
    pinId,
    startUrl: "https://wayfinder.example",
    bounds: { x: 0, y: 40, width: 800, height: 600 },
  });
  // Strict Mode / url-effect cleanup: destroy + bump (same as LinkSidePanelSurface).
  const epochBeforeClose = getPinHideEpoch();
  await closePinWebview(pinId);
  assert.equal(getPinHideEpoch(), epochBeforeClose + 1);

  const second = showPinWebview({
    pinId,
    startUrl: "https://wayfinder.example",
    bounds: { x: 0, y: 40, width: 800, height: 600 },
  });

  resolveSecond();
  await second;
  resolveFirst();
  await first;

  const hides = calls.filter((row) => row.cmd === "pin_webview_hide");
  assert.equal(
    hides.length,
    0,
    `stale show must not re-hide remounted first open; hides=${JSON.stringify(hides)} calls=${JSON.stringify(calls)}`,
  );
  assert.equal(getPinShowGeneration(pinId), 2);
  assert.equal(
    calls.filter((row) => row.cmd === "pin_webview_show").length,
    2,
  );
});

test("window-scoped labels stay pin-id on main and suffix elsewhere", () => {
  assert.equal(pinWebviewLabelForWindow("hula-link-side-panel"), "pin-hula-link-side-panel");
  assert.equal(
    pinWebviewLabelForWindow("hula-link-side-panel", "main"),
    "pin-hula-link-side-panel",
  );
  assert.equal(
    pinWebviewLabelForWindow("playground-pin-demo-1", "popout-thread-abc"),
    "pin-playground-pin-demo-1--popout-thread-abc",
  );
  assert.equal(
    pinHideCloseIsWindowScoped(
      { pinId: "hula-link-side-panel", windowLabel: "main" },
      "main",
    ),
    true,
  );
  assert.equal(
    pinHideCloseIsWindowScoped(
      { pinId: "hula-link-side-panel", windowLabel: "main" },
      "popout-thread-abc",
    ),
    false,
  );
});

test("show/hide/close pin invokes pass current windowLabel", async () => {
  const calls = [];
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "pin_webview_show") {
      return Promise.resolve({
        canGoBack: false,
        canGoForward: false,
        currentUrl: args.startUrl,
      });
    }
    return Promise.resolve(undefined);
  });

  const { closePinWebview, hideAllPinWebviews, hidePinWebview, showPinWebview } =
    await import("./pinWebview.ts");

  await showPinWebview({
    pinId: "hula-link-side-panel",
    startUrl: "https://example.com",
    bounds: { x: 10, y: 40, width: 800, height: 600 },
  });
  await hidePinWebview("hula-link-side-panel");
  await hideAllPinWebviews();
  await closePinWebview("hula-link-side-panel");

  const show = calls.find((row) => row.cmd === "pin_webview_show");
  const hide = calls.find((row) => row.cmd === "pin_webview_hide");
  const hideAll = calls.find((row) => row.cmd === "pin_webview_hide_all");
  const close = calls.find((row) => row.cmd === "pin_webview_close");
  assert.equal(show?.args.windowLabel, "main");
  assert.equal(hide?.args.windowLabel, "main");
  assert.equal(hideAll?.args.windowLabel, "main");
  assert.equal(close?.args.windowLabel, "main");
});

test("inspect and screenshot invoke pin-scoped commands with window label", async () => {
  const calls = [];
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "pin_webview_inspect") {
      return { webviewId: "pin-demo" };
    }
    if (cmd === "pin_webview_screenshot") {
      return { bytes: [1, 2, 3], mime: "image/png", filename: "pin-demo.png" };
    }
    return undefined;
  });
  const {
    inspectPinWebview,
    screenshotPinWebview,
  } = await import("./pinWebview.ts");
  await inspectPinWebview("demo");
  await screenshotPinWebview("demo");
  assert.equal(calls[0].cmd, "pin_webview_inspect");
  assert.equal(calls[0].args.pinId, "demo");
  assert.ok("windowLabel" in calls[0].args);
  assert.equal(calls[1].cmd, "pin_webview_screenshot");
});
