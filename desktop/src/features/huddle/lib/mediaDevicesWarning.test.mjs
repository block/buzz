import assert from "node:assert/strict";
import test from "node:test";

import { availableMediaDevices } from "./mediaDevices.ts";

// Deliberately its own file. The once-per-process latch lives in module scope,
// and the node test runner gives each FILE its own process — so this is the
// only place that observes the latch in its initial state. Folding these cases
// into `mediaDevices.test.mjs` would make them depend on declaration order
// there, because those tests consume the latch on their first null result.

/** Run `body` with `navigator` and `console.warn` swapped out. */
function captureWarnings(navigatorValue, body) {
  const navigatorDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "navigator",
  );
  const originalWarn = console.warn;
  const warnings = [];

  Object.defineProperty(globalThis, "navigator", {
    value: navigatorValue,
    configurable: true,
    writable: true,
  });
  console.warn = (...args) => warnings.push(args.join(" "));

  try {
    body();
  } finally {
    console.warn = originalWarn;
    if (navigatorDescriptor) {
      Object.defineProperty(globalThis, "navigator", navigatorDescriptor);
    } else {
      delete globalThis.navigator;
    }
  }
  return warnings;
}

test("warns exactly once no matter how many callers hit the missing API", () => {
  const warnings = captureWarnings({}, () => {
    // Three call sites, and StrictMode runs the two mount effects twice.
    for (let i = 0; i < 5; i += 1) {
      assert.equal(availableMediaDevices(), null);
    }
  });

  assert.equal(warnings.length, 1, "expected a single diagnostic line");
  assert.match(warnings[0], /^\[mediaDevices\]/);
  assert.match(warnings[0], /non-secure context/);
});

test("stays quiet on later calls once the API is present again", () => {
  const media = { enumerateDevices: () => Promise.resolve([]) };
  const warnings = captureWarnings({ mediaDevices: media }, () => {
    assert.equal(availableMediaDevices(), media);
  });

  assert.deepEqual(warnings, []);
});
