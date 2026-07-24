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
  rateLimitRemainingMs,
  parseRateLimitHint,
  MAX_HINT_SECONDS,
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

test("zero hint uses 10s default (0s gate would be swallowed)", () => {
  reset(0);
  activateRateLimit(0);
  tickTo(9_999);
  assert.equal(isRateLimited(), true);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

test("negative hint uses 10s default", () => {
  reset(0);
  activateRateLimit(-5);
  tickTo(9_999);
  assert.equal(isRateLimited(), true);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

// ── MAX_HINT_SECONDS cap ──────────────────────────────────────────────────────

test("oversized hint is clamped to MAX_HINT_SECONDS", () => {
  reset(0);
  // A relay sending 1 000 000s must not pin the gate beyond 300s.
  activateRateLimit(1_000_000);
  // Gate must still be active at MAX_HINT_SECONDS - 1 ms.
  tickTo(MAX_HINT_SECONDS * 1_000 - 1);
  assert.equal(
    isRateLimited(),
    true,
    "gate must be active just before cap expiry",
  );
  // Gate must expire at MAX_HINT_SECONDS.
  tickTo(MAX_HINT_SECONDS * 1_000 + 1);
  assert.equal(isRateLimited(), false, "gate must expire at MAX_HINT_SECONDS");
});

test("hint exactly at MAX_HINT_SECONDS is honoured without clamping", () => {
  reset(0);
  activateRateLimit(MAX_HINT_SECONDS);
  tickTo(MAX_HINT_SECONDS * 1_000 - 1);
  assert.equal(isRateLimited(), true);
  tickTo(MAX_HINT_SECONDS * 1_000 + 1);
  assert.equal(isRateLimited(), false);
});

test("applyTauriRateLimitIfNeeded with oversized hint clamps to MAX_HINT_SECONDS", async () => {
  // This test imports applyTauriRateLimitIfNeeded separately and verifies that
  // the TS cap applies even when the message string contains a large hint value
  // (the Rust layer clamps in practice, but TS must be independently safe).
  reset(0);
  const { applyTauriRateLimitIfNeeded } = await import("./tauri.ts");
  // Simulate a message that somehow escaped the Rust cap (defence-in-depth).
  applyTauriRateLimitIfNeeded("relay rate-limited: retry in 1000000s");
  // Gate should cap at MAX_HINT_SECONDS * 1000 ms.
  tickTo(MAX_HINT_SECONDS * 1_000 - 1);
  assert.equal(
    isRateLimited(),
    true,
    "gate must still be active just before cap",
  );
  tickTo(MAX_HINT_SECONDS * 1_000 + 1);
  assert.equal(
    isRateLimited(),
    false,
    "gate must expire at MAX_HINT_SECONDS, not 1 000 000s",
  );
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

test("in-flight waitForRateLimit resolves when resetRateLimitGate is called", async () => {
  reset(0);
  activateRateLimit(30);

  let inFlightResolved = false;
  const inFlight = waitForRateLimit().then(() => {
    inFlightResolved = true;
  });

  // Not yet resolved before reset.
  await Promise.resolve();
  assert.equal(inFlightResolved, false);

  // reset resolves the in-flight awaiter immediately.
  resetRateLimitGate();
  await inFlight;
  assert.equal(inFlightResolved, true);

  // Gate is now clear.
  assert.equal(isRateLimited(), false);

  // A new wait should also resolve immediately.
  await waitForRateLimit();
});

// ── rateLimitRemainingMs ──────────────────────────────────────────────────────

test("rateLimitRemainingMs returns 0 when gate is inactive", () => {
  reset(0);
  assert.equal(rateLimitRemainingMs(), 0);
});

test("rateLimitRemainingMs returns remaining ms when gate is active", () => {
  reset(0);
  activateRateLimit(10); // expires at 10_000 ms
  setFakeNow(3_000);
  assert.equal(rateLimitRemainingMs(), 7_000);
});

test("rateLimitRemainingMs returns 0 after gate expires", () => {
  reset(0);
  activateRateLimit(5);
  tickTo(5_001);
  assert.equal(rateLimitRemainingMs(), 0);
});

// ── NOTICE → gate integration ─────────────────────────────────────────────────
// Verifies that the session NOTICE handler logic — a case-sensitive
// `startsWith("rate-limited:")` check followed by activateRateLimit — correctly
// arms the gate. The relay always emits lowercase prefixes.

test("rate-limited: NOTICE prefix (case-sensitive) activates the gate", () => {
  reset(0);
  const notice = "rate-limited: quota exceeded; retry in 8s";
  if (notice.startsWith("rate-limited:")) {
    activateRateLimit(parseRateLimitHint(notice));
  }
  assert.equal(isRateLimited(), true);
  tickTo(8_001);
  assert.equal(isRateLimited(), false);
});

test("Rate-limited: NOTICE with uppercase prefix does NOT activate the gate (relay always emits lowercase)", () => {
  reset(0);
  const notice = "Rate-limited: quota exceeded; retry in 8s";
  if (notice.startsWith("rate-limited:")) {
    activateRateLimit(parseRateLimitHint(notice));
  }
  // Uppercase is not emitted by the relay — gate should remain inactive.
  assert.equal(isRateLimited(), false);
});

// ── Cleanup ───────────────────────────────────────────────────────────────────

// Restore Date.now after all tests to avoid polluting subsequent test files.
test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
