import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    window: dom.window,
  });
});

afterEach(async () => {
  const { resetNativeWebviewModalPark } = await import(
    "./nativeWebviewModalPark.ts"
  );
  resetNativeWebviewModalPark();
  delete globalThis.isTauri;
  delete dom.window.isTauri;
  delete globalThis.__TAURI_INTERNALS__;
  delete dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

function installTauriInvoke(handler) {
  const internals = { invoke: handler };
  globalThis.isTauri = true;
  dom.window.isTauri = true;
  globalThis.__TAURI_INTERNALS__ = internals;
  dom.window.__TAURI_INTERNALS__ = internals;
  return internals;
}

test("first acquire parks pins and playground; nested acquire does not re-park", async () => {
  const calls = [];
  installTauriInvoke((cmd, args) => {
    calls.push({ cmd, args });
    return Promise.resolve(undefined);
  });

  const {
    acquireNativeWebviewModalPark,
    getNativeWebviewModalParkDepth,
    isNativeWebviewModalParked,
    releaseNativeWebviewModalPark,
  } = await import("./nativeWebviewModalPark.ts");

  assert.equal(isNativeWebviewModalParked(), false);
  acquireNativeWebviewModalPark();
  assert.equal(getNativeWebviewModalParkDepth(), 1);
  assert.equal(isNativeWebviewModalParked(), true);
  assert.ok(calls.some((row) => row.cmd === "pin_webview_hide_all"));
  assert.ok(calls.some((row) => row.cmd === "playground_webview_hide_all"));

  const afterFirst = calls.length;
  acquireNativeWebviewModalPark();
  assert.equal(getNativeWebviewModalParkDepth(), 2);
  assert.equal(calls.length, afterFirst, "nested acquire must not hide again");

  releaseNativeWebviewModalPark();
  assert.equal(getNativeWebviewModalParkDepth(), 1);
  assert.equal(isNativeWebviewModalParked(), true);
});

test("last release restores pins and playground", async () => {
  installTauriInvoke(() => Promise.resolve(undefined));

  const {
    acquireNativeWebviewModalPark,
    PLAYGROUND_WEBVIEW_RESTORE_EVENT,
    releaseNativeWebviewModalPark,
  } = await import("./nativeWebviewModalPark.ts");
  const { PIN_WEBVIEW_RESTORE_EVENT } = await import(
    "../../features/pinned-sites/lib/pinWebview.ts"
  );

  let pinRestores = 0;
  let playgroundRestores = 0;
  const onPin = () => {
    pinRestores += 1;
  };
  const onPlayground = () => {
    playgroundRestores += 1;
  };
  window.addEventListener(PIN_WEBVIEW_RESTORE_EVENT, onPin);
  window.addEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onPlayground);

  acquireNativeWebviewModalPark();
  acquireNativeWebviewModalPark();
  releaseNativeWebviewModalPark();
  assert.equal(pinRestores, 0, "must not restore while another overlay is open");
  assert.equal(playgroundRestores, 0);

  releaseNativeWebviewModalPark();
  assert.equal(pinRestores, 1);
  assert.equal(playgroundRestores, 1);

  window.removeEventListener(PIN_WEBVIEW_RESTORE_EVENT, onPin);
  window.removeEventListener(PLAYGROUND_WEBVIEW_RESTORE_EVENT, onPlayground);
});

test("notifyNativeWebviewRestore no-ops while parked", async () => {
  installTauriInvoke(() => Promise.resolve(undefined));

  const {
    acquireNativeWebviewModalPark,
    notifyNativeWebviewRestore,
    releaseNativeWebviewModalPark,
  } = await import("./nativeWebviewModalPark.ts");
  const { PIN_WEBVIEW_RESTORE_EVENT } = await import(
    "../../features/pinned-sites/lib/pinWebview.ts"
  );

  let restores = 0;
  const onRestore = () => {
    restores += 1;
  };
  window.addEventListener(PIN_WEBVIEW_RESTORE_EVENT, onRestore);

  acquireNativeWebviewModalPark();
  notifyNativeWebviewRestore();
  assert.equal(restores, 0);

  releaseNativeWebviewModalPark();
  assert.equal(restores, 1);

  notifyNativeWebviewRestore();
  assert.equal(restores, 2);

  window.removeEventListener(PIN_WEBVIEW_RESTORE_EVENT, onRestore);
});
