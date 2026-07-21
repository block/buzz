import assert from "node:assert/strict";
import test from "node:test";

// ── Fake-timer setup ──────────────────────────────────────────────────────────

// The gate uses window.setTimeout/clearTimeout. We need a controllable fake
// before the module is loaded. Set up globalThis.window with a fake timer
// implementation and then dynamically import the module under test.

let fakeNow = 0;
const pendingTimers = new Map(); // id → { fn, fireAt }
let nextTimerId = 1;

function fakeSetTimeout(fn, ms) {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
  return id;
}

function fakeClearTimeout(id) {
  pendingTimers.delete(id);
}

function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
}

// Install window shim before any imports use it.
globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
};

// Patch Date.now for the gate.
const origDateNow = Date.now;
function setFakeNow(ms) {
  fakeNow = ms;
  Date.now = () => fakeNow;
}

// Import after window is set up.
const {
  activateRateLimit,
  isRateLimited,
  waitForRateLimit,
  resetRateLimitGate,
  parseRateLimitHint,
} = await import("./relayRateLimitGate.ts");

// Helper to reset between tests.
function reset(startMs = 0) {
  pendingTimers.clear();
  nextTimerId = 1;
  setFakeNow(startMs);
  resetRateLimitGate();
}

// ── parseRateLimitHint ────────────────────────────────────────────────────────

test("parseRateLimitHint extracts seconds from CLOSED message", () => {
  assert.equal(
    parseRateLimitHint("rate-limited: quota exceeded; retry in 4s"),
    4,
  );
});

test("parseRateLimitHint extracts seconds from HTTP 429 prefix", () => {
  assert.equal(parseRateLimitHint("relay rate-limited: retry in 30s"), 30);
});

test("parseRateLimitHint returns null when no hint present", () => {
  assert.equal(parseRateLimitHint("rate-limited: quota exceeded"), null);
  assert.equal(parseRateLimitHint(""), null);
  assert.equal(parseRateLimitHint("some other message"), null);
});

// ── isRateLimited / activate ──────────────────────────────────────────────────

test("not rate-limited before any activation", () => {
  reset();
  assert.equal(isRateLimited(), false);
});

test("rate-limited immediately after activation", () => {
  reset(0);
  activateRateLimit(10);
  assert.equal(isRateLimited(), true);
});

test("rate-limit expires when timer fires", () => {
  reset(0);
  activateRateLimit(10);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

test("activation extends expiry when new hint is longer than existing", () => {
  reset(0);
  activateRateLimit(5); // expires at 5000
  activateRateLimit(20); // should extend to 20000
  tickTo(5_001);
  // Gate should still be active because the longer window was applied.
  assert.equal(isRateLimited(), true);
  tickTo(20_001);
  assert.equal(isRateLimited(), false);
});

test("shorter hint does not shrink an existing longer window", () => {
  reset(0);
  activateRateLimit(20); // expires at 20000
  activateRateLimit(5); // shorter — should NOT shrink
  tickTo(5_001);
  assert.equal(isRateLimited(), true);
  tickTo(20_001);
  assert.equal(isRateLimited(), false);
});

test("null hint uses 10s default", () => {
  reset(0);
  activateRateLimit(null);
  tickTo(9_999);
  assert.equal(isRateLimited(), true);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

// ── waitForRateLimit ──────────────────────────────────────────────────────────

test("waitForRateLimit resolves immediately when not rate-limited", async () => {
  reset();
  let resolved = false;
  const p = waitForRateLimit().then(() => {
    resolved = true;
  });
  await p;
  assert.equal(resolved, true);
});

test("waitForRateLimit resolves after timer fires", async () => {
  reset(0);
  activateRateLimit(5);

  let resolved = false;
  const p = waitForRateLimit().then(() => {
    resolved = true;
  });

  // Should not have resolved yet.
  await Promise.resolve();
  assert.equal(resolved, false);

  // Fire the timer.
  tickTo(5_001);

  await p;
  assert.equal(resolved, true);
});

test("multiple waiters all resolve when gate expires", async () => {
  reset(0);
  activateRateLimit(5);

  const results = [];
  const promises = [1, 2, 3].map((id) =>
    waitForRateLimit().then(() => results.push(id)),
  );

  await Promise.resolve();
  assert.equal(results.length, 0);

  tickTo(5_001);
  await Promise.all(promises);
  assert.deepEqual(results, [1, 2, 3]);
});

// ── resetRateLimitGate ────────────────────────────────────────────────────────

test("resetRateLimitGate clears an active gate immediately", () => {
  reset(0);
  activateRateLimit(30);
  assert.equal(isRateLimited(), true);

  resetRateLimitGate();
  assert.equal(isRateLimited(), false);
});

test("resetRateLimitGate clears the timer so it does not fire later", () => {
  reset(0);
  activateRateLimit(10);
  const timerCountBefore = pendingTimers.size;
  assert.equal(timerCountBefore, 1);

  resetRateLimitGate();
  assert.equal(pendingTimers.size, 0);
});

test("waitForRateLimit resolves immediately after resetRateLimitGate", async () => {
  reset(0);
  activateRateLimit(30);
  // Start waiting before reset (intentionally not awaited — tests the in-flight case).
  waitForRateLimit();

  resetRateLimitGate();
  // Now the gate is gone. The in-flight promise won't resolve on its own
  // (the timer was cleared). Confirm the gate read is clean.
  assert.equal(isRateLimited(), false);
  // A new waitForRateLimit after reset should resolve immediately.
  await waitForRateLimit();
});

// ── Cleanup ───────────────────────────────────────────────────────────────────

// Restore Date.now after all tests to avoid polluting subsequent test files.
test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
