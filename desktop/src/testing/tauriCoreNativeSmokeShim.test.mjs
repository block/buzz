import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { JSDOM } from "jsdom";

let activeDom;

afterEach(() => {
  activeDom?.window.close();
  activeDom = undefined;
});

/**
 * A fresh JSDOM per test, not a shared one with properties deleted between
 * runs: all three of `window.__TAURI_INTERNALS__` itself, `.invoke`, and
 * `.metadata` are non-writable/non-configurable on the real Tauri runtime
 * (tauri-2.11.5's `scripts/core.js` and `src/manager/webview.rs`), so once
 * defined that way they can't be deleted or reset — a shared window would
 * leak state across tests.
 */
function makeNativeWindow(invokeImpl) {
  activeDom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost",
  });
  const internals = {};
  Object.defineProperty(internals, "invoke", { value: invokeImpl });
  Object.defineProperty(internals, "metadata", {
    value: { currentWindow: { label: "main" } },
  });
  Object.defineProperty(activeDom.window, "__TAURI_INTERNALS__", {
    value: internals,
    enumerable: true,
  });
  globalThis.window = activeDom.window;
  globalThis.document = activeDom.window.document;
  return activeDom.window;
}

test("a passthrough command reaches the real native invoke, untouched by the mock hook", async () => {
  const nativeCalls = [];
  const window_ = makeNativeWindow(async (command, payload) => {
    nativeCalls.push({ command, payload });
    return "native-result";
  });
  const mockCalls = [];
  window_.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ = async (command, payload) => {
    mockCalls.push({ command, payload });
    return "mock-result";
  };

  const { invoke } = await import("./tauriCoreNativeSmokeShim.ts");
  const result = await invoke("plugin_browser_open", { pluginId: "p" });

  assert.equal(result, "native-result");
  assert.deepEqual(nativeCalls, [
    { command: "plugin_browser_open", payload: { pluginId: "p" } },
  ]);
  assert.equal(mockCalls.length, 0);
});

test("a non-passthrough command reaches the e2e mock hook, never the native invoke", async () => {
  const nativeCalls = [];
  const window_ = makeNativeWindow(async (command, payload) => {
    nativeCalls.push({ command, payload });
    return "native-result";
  });
  const mockCalls = [];
  window_.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ = async (command, payload) => {
    mockCalls.push({ command, payload });
    return "mock-result";
  };

  const { invoke } = await import("./tauriCoreNativeSmokeShim.ts");
  const result = await invoke("get_identity", { some: "arg" });

  assert.equal(result, "mock-result");
  assert.equal(nativeCalls.length, 0);
  assert.deepEqual(mockCalls, [
    { command: "get_identity", payload: { some: "arg" } },
  ]);
});

test("a non-passthrough command falls back to the real invoke when the mock hook isn't installed yet", async () => {
  const nativeCalls = [];
  makeNativeWindow(async (command, payload) => {
    nativeCalls.push({ command, payload });
    return "native-result";
  });
  // No __BUZZ_E2E_INVOKE_MOCK_COMMAND__ installed — simulates a call made
  // before e2eBridge.ts's bootstrap has finished wiring it up.

  const { invoke } = await import("./tauriCoreNativeSmokeShim.ts");
  const result = await invoke("get_identity");

  assert.equal(result, "native-result");
  assert.deepEqual(nativeCalls, [{ command: "get_identity", payload: {} }]);
});
