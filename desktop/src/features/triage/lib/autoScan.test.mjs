import assert from "node:assert/strict";
import test from "node:test";

import {
  AUTO_SCAN_DEBOUNCE_MS,
  AUTO_SCAN_MAX_WAIT_MS,
  computeAutoScanDelay,
} from "./autoScan.ts";

test("nothing pending means no scan is scheduled", () => {
  assert.equal(computeAutoScanDelay({ pendingCount: 0, waitedMs: 0 }), null);
});

test("a fresh arrival waits the full debounce window", () => {
  assert.equal(
    computeAutoScanDelay({ pendingCount: 1, waitedMs: 0 }),
    AUTO_SCAN_DEBOUNCE_MS,
  );
});

test("the debounce window does not grow with the number of pending items", () => {
  assert.equal(
    computeAutoScanDelay({ pendingCount: 50, waitedMs: 0 }),
    AUTO_SCAN_DEBOUNCE_MS,
  );
});

test("continuous activity still scans once the max wait is reached", () => {
  // A channel that never goes quiet keeps re-arming the debounce; the shrinking
  // budget is what stops triage from being deferred forever.
  assert.equal(
    computeAutoScanDelay({
      pendingCount: 3,
      waitedMs: AUTO_SCAN_MAX_WAIT_MS,
    }),
    0,
  );
});

test("the remaining budget caps the delay near the ceiling", () => {
  const waitedMs = AUTO_SCAN_MAX_WAIT_MS - 2_000;
  assert.equal(computeAutoScanDelay({ pendingCount: 1, waitedMs }), 2_000);
});

test("delay never goes negative once the ceiling is passed", () => {
  assert.equal(
    computeAutoScanDelay({
      pendingCount: 1,
      waitedMs: AUTO_SCAN_MAX_WAIT_MS * 2,
    }),
    0,
  );
});
