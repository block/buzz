import assert from "node:assert/strict";
import test from "node:test";

import { CONVERGENCE_FRAME_CAP, convergenceStep } from "./scrollConvergence.ts";

function input(overrides) {
  return {
    targetMessageId: "target",
    indexByMessageId: new Map([["target", 100]]),
    lastIssuedIndex: null,
    librarySettled: false,
    stalledOffTarget: false,
    framesUsed: 0,
    ...overrides,
  };
}

// --- re-aim / staleness guard ------------------------------------------------

test("convergenceStep: first frame aims at the resolved index, not yet done", () => {
  const step = convergenceStep(input({ lastIssuedIndex: null }));
  assert.equal(step.nextIndex, 100);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

test("convergenceStep: re-resolves a shifted index from the map each frame", () => {
  // A prepend shifted the target from 100 to 105. The library is still chasing
  // the old index (lastIssuedIndex 100); the reducer must aim at the NEW index
  // so the adapter re-issues scrollToIndex(105). This is the staleness guard.
  const step = convergenceStep(
    input({
      indexByMessageId: new Map([["target", 105]]),
      lastIssuedIndex: 100,
    }),
  );
  assert.equal(step.nextIndex, 105);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

test("convergenceStep: target removed mid-settle stops with converged=false", () => {
  // Target deleted from the map while the loop was chasing it. Terminate so the
  // adapter clears the highlight instead of chasing a vanished row.
  const step = convergenceStep(
    input({
      indexByMessageId: new Map(), // target gone
      lastIssuedIndex: 100,
      framesUsed: 3,
    }),
  );
  assert.equal(step.nextIndex, null);
  assert.equal(step.done, true);
  assert.equal(step.converged, false);
});

// --- settle ------------------------------------------------------------------

test("convergenceStep: library settled while aiming at current index converges", () => {
  const step = convergenceStep(
    input({ lastIssuedIndex: 100, librarySettled: true }),
  );
  assert.equal(step.nextIndex, 100);
  assert.equal(step.done, true);
  assert.equal(step.converged, true);
});

test("convergenceStep: a settle reported WHILE re-aiming is ignored", () => {
  // The index just moved (105) but the library reports settled — that settle is
  // on the OLD index (100), so it must NOT count as convergence. The reducer
  // keeps going and aims at the new index.
  const step = convergenceStep(
    input({
      indexByMessageId: new Map([["target", 105]]),
      lastIssuedIndex: 100,
      librarySettled: true,
    }),
  );
  assert.equal(step.nextIndex, 105);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

test("convergenceStep: aiming at current but not yet settled keeps waiting", () => {
  // Library is chasing the right index but its offset hasn't stabilized. The
  // reducer returns the same index (so the adapter re-issues NOTHING — issuing
  // would reset the library's stableFrames and prevent settling) and waits.
  const step = convergenceStep(
    input({ lastIssuedIndex: 100, librarySettled: false }),
  );
  assert.equal(step.nextIndex, 100);
  assert.equal(step.reissue, false);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

// --- off-target stall (liveness) ---------------------------------------------

test("convergenceStep: stalled off-target while aiming at current re-issues same index", () => {
  // The library's offset stopped moving but never reached the current index's
  // target (its internal reconcile deadlocked after rows re-measured). The
  // reducer signals a same-index re-issue to kick it — the loop continues.
  const step = convergenceStep(
    input({ lastIssuedIndex: 100, stalledOffTarget: true }),
  );
  assert.equal(step.nextIndex, 100);
  assert.equal(step.reissue, true);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

test("convergenceStep: a stall reported WHILE re-aiming does not re-issue", () => {
  // The index just moved (105) but the library reports a stall on the OLD index
  // (100). The reducer re-aims at the new index normally; the stale stall must
  // NOT trigger a same-index kick (there is no current-index stall to kick).
  const step = convergenceStep(
    input({
      indexByMessageId: new Map([["target", 105]]),
      lastIssuedIndex: 100,
      stalledOffTarget: true,
    }),
  );
  assert.equal(step.nextIndex, 105);
  assert.equal(step.reissue, false);
  assert.equal(step.done, false);
  assert.equal(step.converged, false);
});

test("convergenceStep: a settle takes priority over a concurrent stall flag", () => {
  // Defensive: the adapter computes settle and stall as mutually exclusive, but
  // if both arrive, a genuine settle must win (converge) rather than spin on a
  // pointless re-issue.
  const step = convergenceStep(
    input({
      lastIssuedIndex: 100,
      librarySettled: true,
      stalledOffTarget: true,
    }),
  );
  assert.equal(step.done, true);
  assert.equal(step.converged, true);
  assert.equal(step.reissue, false);
});

// --- frame cap ---------------------------------------------------------------

test("convergenceStep: terminates at the frame cap without converging", () => {
  // A row that never settles (librarySettled stays false) must still stop at the
  // cap rather than spin forever.
  const step = convergenceStep(
    input({
      lastIssuedIndex: 100,
      librarySettled: false,
      framesUsed: CONVERGENCE_FRAME_CAP - 1,
    }),
  );
  assert.equal(step.done, true);
  assert.equal(step.converged, false);
  assert.equal(step.nextIndex, 100);
});

test("convergenceStep: frame cap bounds a perpetually shifting target", () => {
  // Drive the loop the way the adapter would: the target index moves every
  // frame, so the library never settles. The loop must terminate at the cap.
  let lastIssuedIndex = null;
  let framesUsed = 0;
  let done = false;
  let converged = true;

  while (framesUsed < CONVERGENCE_FRAME_CAP + 5) {
    const movingIndex = 100 + framesUsed; // shifts every frame
    const step = convergenceStep(
      input({
        indexByMessageId: new Map([["target", movingIndex]]),
        lastIssuedIndex,
        librarySettled: false,
        framesUsed,
      }),
    );
    lastIssuedIndex = step.nextIndex;
    framesUsed += 1;
    if (step.done) {
      done = step.done;
      converged = step.converged;
      break;
    }
  }

  assert.equal(done, true);
  assert.equal(converged, false);
  assert.ok(framesUsed <= CONVERGENCE_FRAME_CAP);
});

test("convergenceStep: converges once a re-aimed index then settles", () => {
  // Realistic flow: frame 0 aims (lastIssued null -> 100), frame 1 the library
  // is chasing 100 and reports settled -> converged.
  const aim = convergenceStep(input({ lastIssuedIndex: null }));
  assert.equal(aim.nextIndex, 100);
  assert.equal(aim.done, false);

  const settle = convergenceStep(
    input({
      lastIssuedIndex: aim.nextIndex,
      librarySettled: true,
      framesUsed: 1,
    }),
  );
  assert.equal(settle.done, true);
  assert.equal(settle.converged, true);
});
