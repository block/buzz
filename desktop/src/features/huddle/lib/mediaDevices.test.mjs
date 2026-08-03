import assert from "node:assert/strict";
import test from "node:test";

import { availableMediaDevices } from "./mediaDevices.ts";

/**
 * Swap `globalThis.navigator` for the duration of `run`, and swallow the
 * missing-API diagnostic so it does not pollute test output. The
 * once-per-process behaviour of that warning is covered separately in
 * `mediaDevicesWarning.test.mjs`, which needs its own file to see the latch
 * unset.
 */
function withNavigator(value, run) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const originalWarn = console.warn;
  Object.defineProperty(globalThis, "navigator", {
    value,
    configurable: true,
    writable: true,
  });
  console.warn = () => {};
  try {
    return run();
  } finally {
    console.warn = originalWarn;
    if (descriptor) {
      Object.defineProperty(globalThis, "navigator", descriptor);
    } else {
      delete globalThis.navigator;
    }
  }
}

test("returns the MediaDevices object when the API is present", () => {
  const media = { enumerateDevices: () => Promise.resolve([]) };
  withNavigator({ mediaDevices: media }, () => {
    assert.equal(availableMediaDevices(), media);
  });
});

test("returns null when navigator.mediaDevices is undefined", () => {
  // The non-secure-context WKWebView case that crashed the app (#3118).
  withNavigator({}, () => {
    assert.equal(availableMediaDevices(), null);
  });
});

test("returns null when mediaDevices exists but exposes no enumerateDevices", () => {
  withNavigator({ mediaDevices: {} }, () => {
    assert.equal(availableMediaDevices(), null);
  });
});

test("returns null when there is no navigator at all", () => {
  withNavigator(undefined, () => {
    assert.equal(availableMediaDevices(), null);
  });
});
