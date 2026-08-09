import assert from "node:assert/strict";
import test from "node:test";

import {
  availableMediaDevices,
  MICROPHONE_UNAVAILABLE_ERROR,
  requireMediaDevices,
} from "./mediaDevices.ts";

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

test("requireMediaDevices throws the sentinel when the API is missing", () => {
  // The join path cannot degrade, so it throws a value huddleError can map to
  // copy instead of letting a raw TypeError reach the error boundary.
  withNavigator({}, () => {
    assert.throws(
      () => requireMediaDevices(),
      (error) => error.message === MICROPHONE_UNAVAILABLE_ERROR,
    );
  });
});

test("requireMediaDevices returns the object when getUserMedia is present", () => {
  const media = {
    enumerateDevices: () => Promise.resolve([]),
    getUserMedia: () => Promise.resolve({}),
  };
  withNavigator({ mediaDevices: media }, () => {
    assert.equal(requireMediaDevices(), media);
  });
});
