/**
 * Unit tests for tauri.ts — focused on `applyTauriRateLimitIfNeeded`, the
 * extracted `relay rate-limited:` classifier that activates the shared
 * rate-limit gate when Rust emits an HTTP 429 error prefix.
 *
 * Testing the exported production function (not a local copy) ensures any
 * change to the classifier logic is immediately covered here.
 */
import assert from "node:assert/strict";
import test from "node:test";

// ── Fake-timer + gate setup ───────────────────────────────────────────────────

let fakeNow = 0;
const pendingTimers = new Map();
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

const origDateNow = Date.now;
function setFakeNow(ms) {
  fakeNow = ms;
  Date.now = () => fakeNow;
}

globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
};

setFakeNow(0);

const { isRateLimited, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

// Import the production classifier from tauri.ts — tests must exercise the
// real function, not a local copy, so a logic change is always caught here.
const { applyTauriRateLimitIfNeeded } = await import("./tauri.ts");

function resetGate(startMs = 0) {
  pendingTimers.clear();
  nextTimerId = 1;
  setFakeNow(startMs);
  resetRateLimitGate();
}

// ── applyTauriRateLimitIfNeeded: relay rate-limited: prefix ───────────────────

test("relay rate-limited: prefix activates the rate-limit gate", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: retry in 10s");
  assert.equal(isRateLimited(), true, "gate must be active after 429 error");
});

test("relay rate-limited: prefix parses the retry hint and arms the gate duration", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: retry in 7s");
  // Gate should be active at 6s.
  setFakeNow(6_000);
  assert.equal(isRateLimited(), true);
  // Gate should expire after 7s.
  tickTo(7_001);
  assert.equal(isRateLimited(), false);
});

test("relay rate-limited: with no hint uses the 10s default", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: quota exceeded");
  tickTo(9_999);
  assert.equal(isRateLimited(), true);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

test("non-rate-limited error does not activate the gate", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay returned 404 Not Found");
  assert.equal(
    isRateLimited(),
    false,
    "gate must remain inactive for unrelated errors",
  );
});

test("relay rate-limited: prefix check is case-sensitive (Rust always emits lowercase)", () => {
  resetGate(0);
  // The prefix from Rust is always lowercase; mixed-case must not trigger it.
  applyTauriRateLimitIfNeeded("Relay rate-limited: retry in 5s");
  assert.equal(
    isRateLimited(),
    false,
    "uppercase prefix must not activate gate (relay emits lowercase only)",
  );
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
