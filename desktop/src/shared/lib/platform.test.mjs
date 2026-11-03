import assert from "node:assert/strict";
import test from "node:test";

import { isWindowsPlatform } from "./platform.ts";

function withNavigator(navigator, callback) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: navigator,
  });

  try {
    callback();
  } finally {
    if (original) {
      Object.defineProperty(globalThis, "navigator", original);
    } else {
      delete globalThis.navigator;
    }
  }
}

test("Windows detection accepts the platform strings WebView2 reports", () => {
  for (const platform of ["Win32", "Win64", "Windows"]) {
    withNavigator({ platform, userAgent: "" }, () => {
      assert.equal(isWindowsPlatform(), true, platform);
    });
  }
});

test("Windows detection rejects the other desktop platforms", () => {
  for (const platform of ["MacIntel", "Darwin", "Linux x86_64"]) {
    withNavigator({ platform, userAgent: "" }, () => {
      assert.equal(isWindowsPlatform(), false, platform);
    });
  }
});
