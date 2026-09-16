import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

import {
  MIN_PIN_WEBVIEW_EDGE,
  pinWebviewBoundsAreUsable,
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

  const { hidePinWebview, showPinWebview } = await import("./pinWebview.ts");

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

  resolveOld();
  await first;
  resolveNew();
  await second;

  const hides = calls.filter((row) => row.cmd === "pin_webview_hide");
  assert.equal(hides.length, 2);
  assert.equal(calls.filter((row) => row.cmd === "pin_webview_show").length, 2);
});
