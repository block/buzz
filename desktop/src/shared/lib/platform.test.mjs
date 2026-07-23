import assert from "node:assert/strict";
import test from "node:test";

import { isWindowsPlatform } from "./platform.ts";

function withNavigatorPlatform(platform, callback) {
  const originalNavigator = Object.getOwnPropertyDescriptor(
    globalThis,
    "navigator",
  );
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { platform, userAgent: "" },
  });

  try {
    callback();
  } finally {
    if (originalNavigator) {
      Object.defineProperty(globalThis, "navigator", originalNavigator);
    } else {
      delete globalThis.navigator;
    }
  }
}

test("Windows platform detection accepts Win32 without matching Darwin", () => {
  withNavigatorPlatform("Win32", () => {
    assert.equal(isWindowsPlatform(), true);
  });
  withNavigatorPlatform("Darwin", () => {
    assert.equal(isWindowsPlatform(), false);
  });
});
