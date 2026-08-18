import assert from "node:assert/strict";
import test from "node:test";

import {
  AUTO_RESTART_QUIESCENCE_MS,
  decideAutoRestart,
  nextEdgeState,
} from "./autoRestartPolicy.ts";

// ── Chunk F policy matrix ────────────────────────────────────────────────────
//
// SAFETY-CRITICAL: stop is SIGTERM → ≤1s → SIGKILL with no in-process drain,
// so a wrong "fire" here kills a mid-turn agent. The never-fire rows below
// are exhaustive over every gate; each row flips exactly one input away from
// the all-green baseline to prove that gate alone holds the line.

/** All-green inputs: every gate open, window satisfied — the ONLY fire case. */
function greenInputs(overrides = {}) {
  return {
    autoRestartEnabled: true,
    needsRestart: true,
    working: false,
    workingSource: "none",
    connected: true,
    isLocalBackend: true,
    isRunning: true,
    edgeConsumed: false,
    quiescentForMs: AUTO_RESTART_QUIESCENCE_MS,
    ...overrides,
  };
}

test("fires only when every gate is green and the window has elapsed", () => {
  assert.equal(decideAutoRestart(greenInputs()), "fire");
});

// ── never-fire rows: one gate red at a time ─────────────────────────────────

const NEVER_FIRE_ROWS = [
  ["opt-out toggle off", { autoRestartEnabled: false }],
  ["no config drift", { needsRestart: false }],
  [
    "agent mid-turn (working, observer)",
    { working: true, workingSource: "observer" },
  ],
  [
    "typing source counts as working",
    { working: true, workingSource: "typing" },
  ],
  ["working flag alone defers (defensive)", { working: true }],
  [
    "source alone defers even if working flag lies (defensive)",
    { workingSource: "observer" },
  ],
  ["typing source alone defers", { workingSource: "typing" }],
  ["observer relay not connected", { connected: false }],
  ["remote backend", { isLocalBackend: false }],
  ["agent not running", { isRunning: false }],
  [
    "edge already consumed (one attempt per rising edge)",
    { edgeConsumed: true },
  ],
];

for (const [label, overrides] of NEVER_FIRE_ROWS) {
  test(`never fires: ${label}`, () => {
    assert.equal(
      decideAutoRestart(greenInputs(overrides)),
      "hold",
      `${label} must hold — a fire here is a kill`,
    );
  });
}

// ── the quiescence window ───────────────────────────────────────────────────

test("arms (does not fire) before the window elapses", () => {
  assert.equal(decideAutoRestart(greenInputs({ quiescentForMs: 0 })), "arm");
  assert.equal(
    decideAutoRestart(
      greenInputs({ quiescentForMs: AUTO_RESTART_QUIESCENCE_MS - 1 }),
    ),
    "arm",
  );
});

test("window is minutes-scale — far beyond the 25s turn-store prune", () => {
  // A relay hiccup makes a mid-turn agent look idle after 25s; the window
  // must dwarf that so the flicker resets it long before firing.
  assert.ok(AUTO_RESTART_QUIESCENCE_MS >= 2 * 60 * 1000);
});

// ── edge-trigger state machine ───────────────────────────────────────────────

test("falling needsRestart edge re-arms a consumed edge", () => {
  const consumed = { consumed: true, armedAt: null };
  const next = nextEdgeState(consumed, {
    needsRestart: false,
    isRunning: true,
  });
  assert.deepEqual(next, { consumed: false, armedAt: null });
});

test("agent stop re-arms a consumed edge (manual stop/start cycle can auto-fire again)", () => {
  const consumed = { consumed: true, armedAt: null };
  const next = nextEdgeState(consumed, {
    needsRestart: true,
    isRunning: false,
  });
  assert.deepEqual(next, { consumed: false, armedAt: null });
});

test("a held rising edge preserves consumed state (failed attempt badges only)", () => {
  const consumed = { consumed: true, armedAt: null };
  const next = nextEdgeState(consumed, { needsRestart: true, isRunning: true });
  assert.equal(next.consumed, true, "no retry until the edge cycles");
});

test("undefined prior state initializes un-consumed and un-armed", () => {
  assert.deepEqual(
    nextEdgeState(undefined, { needsRestart: true, isRunning: true }),
    { consumed: false, armedAt: null },
  );
});
