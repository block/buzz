/**
 * Work-block rendering while a turn is live and when it finishes: the rail, the
 * live window, the fold animation, and the reader's disclosure choice.
 * Orphaned work and streaming cost live in
 * `AgentSessionWorkBlock.orphaned.test.mjs`; the shared rig is
 * `AgentSessionWorkBlockTestRig.mjs`.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  renderBlock,
  setPrefersReducedMotion,
  step,
} from "./AgentSessionWorkBlockTestRig.mjs";

// ── Live ─────────────────────────────────────────────────────────────────────

test("a live block shows no header line — the rail is the status", async () => {
  const view = await renderBlock([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(
    view.summary(),
    null,
    "a header while live would only restate what the arriving steps show",
  );
  assert.equal(view.stepCount(), 2);
});

test("a live block windows to the last three steps with the rest behind a disclosure", async () => {
  const items = ["a", "b", "c", "d", "e"].map((id) => step(id));
  items[4] = step("e", { status: "executing", completedAt: null });
  const view = await renderBlock(items, { streamingItemId: "e" });

  assert.equal(view.stepCount(), 3, "only the live window renders on the rail");
  const disclosure = view.previousSteps();
  assert.ok(disclosure, "older steps sit behind a disclosure");
  assert.match(disclosure.textContent, /2 previous steps/);
});

test("expanding previous steps reveals the older steps in place", async () => {
  const { act } = await import("@testing-library/react");
  const items = ["a", "b", "c", "d", "e"].map((id) => step(id));
  items[4] = step("e", { status: "executing", completedAt: null });
  const view = await renderBlock(items, { streamingItemId: "e" });

  assert.equal(view.stepCount(), 3);
  await act(async () => {
    view.previousSteps().click();
  });
  assert.equal(
    await view.settleToStepCount(5),
    5,
    "all five steps are now on the rail",
  );
});

// ── Finished ─────────────────────────────────────────────────────────────────

test("a block that was already finished on mount folds to an N steps line", async () => {
  const view = await renderBlock([step("a"), step("b"), step("c")]);
  const summary = view.summary();
  assert.ok(summary, "a finished block gets its summary line");
  assert.match(summary.textContent, /3 steps/);
  assert.equal(summary.getAttribute("aria-expanded"), "false");
  assert.equal(view.stepCount(), 0, "the rail is collapsed away");
});

test("a finished block containing a failure names the failure in its folded line", async () => {
  const view = await renderBlock([
    step("a"),
    step("b", { isError: true, status: "failed" }),
    step("c"),
  ]);
  assert.match(
    view.summary().textContent,
    /3 steps · 1 failed/,
    "a failure must never hide behind a neutral count",
  );
});

test("clicking the folded line expands the whole rail", async () => {
  const { act } = await import("@testing-library/react");
  const view = await renderBlock([step("a"), step("b"), step("c")]);
  assert.equal(view.stepCount(), 0);

  await act(async () => {
    view.summary().click();
  });

  assert.equal(await view.settleToStepCount(3), 3);
  assert.equal(view.summary().getAttribute("aria-expanded"), "true");
});

test("a block expanded by the reader while live shows every step, not just the window", async () => {
  const { act } = await import("@testing-library/react");
  const items = ["a", "b", "c", "d", "e"].map((id) => step(id));
  items[4] = step("e", { status: "executing", completedAt: null });
  const view = await renderBlock(items, { streamingItemId: "e" });

  assert.equal(view.stepCount(), 3);
  await act(async () => {
    view.previousSteps().click();
  });
  assert.equal(
    await view.settleToStepCount(5),
    5,
    "a reader who asked to see the work sees all of it",
  );

  // And the window does NOT come back as more work streams in: the reader's
  // choice is not re-decided on every append.
  await act(async () => {
    view.stream(
      [...items, step("f", { status: "executing", completedAt: null })],
      "f",
    );
  });
  assert.equal(
    await view.settleToStepCount(6),
    6,
    "a reader-expanded live block keeps showing everything as it grows",
  );
});

// ── Fold animation ───────────────────────────────────────────────────────────

test("a block that finishes while mounted stays open for a paint so the collapse is visible", async () => {
  const { act } = await import("@testing-library/react");
  const live = [
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ];
  const view = await renderBlock(live);
  assert.equal(view.summary(), null, "live: no header");
  assert.equal(view.stepCount(), 2);

  // The turn finishes: the same block id re-renders with settled steps.
  await act(async () => {
    view.stream([step("a"), step("b")]);
  });

  // Still open on the commit right after finishing — that open state is what
  // gives the height animation something to collapse FROM. A block that jumped
  // straight to closed would swap a rail for a one-line summary between frames.
  assert.equal(
    view.stepCount(),
    2,
    "the rail is still mounted for the settle frame",
  );
  assert.ok(
    view.summary(),
    "the summary line appears as soon as work finishes",
  );

  // After the settle frames it closes — once the collapse animation has run.
  assert.equal(await view.settleToStepCount(0), 0, "the block settles closed");
});

test("under reduced motion a finishing block folds immediately, with no settle frames", async () => {
  const { act } = await import("@testing-library/react");
  setPrefersReducedMotion(true);

  const view = await renderBlock([
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ]);
  assert.equal(view.stepCount(), 2);

  await act(async () => {
    view.stream([step("a"), step("b")]);
  });

  assert.equal(
    view.stepCount(),
    0,
    "reduced motion skips the animation, so there is nothing to hold open for",
  );
  assert.ok(
    view.summary(),
    "it still folds to a summary line — only the animation is skipped",
  );
});

// ── Reader choice ────────────────────────────────────────────────────────────

/**
 * The echo trap, and why this block is structurally immune to it.
 *
 * `<details>` fires `toggle` for programmatic `open` changes as well as for
 * clicks, indistinguishably — so a policy-driven open echoes back looking like
 * a reader choice and pins the row to its first policy state forever. That trap
 * cost time on the tool-run card.
 *
 * This block cannot hit it, because its disclosure is a `<button>` whose only
 * state change is the click handler firing: there is no browser-generated echo
 * to mistake for intent. That is a real invariant and not an accident of the
 * current markup — switching the trigger to `<details>` would reintroduce the
 * trap — so it is asserted rather than assumed. An earlier version of this test
 * dispatched a synthetic `toggle` at the block and asserted it changed nothing,
 * which passed for the wrong reason: nothing was listening for `toggle` at all.
 */
test("the block's disclosure has no toggle echo to mistake for a reader choice", async () => {
  const { act } = await import("@testing-library/react");
  const live = [
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ];
  const view = await renderBlock(live);

  await act(async () => {
    view.stream([step("a"), step("b")]);
  });
  assert.equal(await view.settleToStepCount(0), 0, "policy folds it");

  const trigger = view.summary();
  assert.equal(
    trigger.tagName,
    "BUTTON",
    "a button has no programmatic-toggle echo; <details> would need the hook's guard",
  );
  assert.equal(
    view.q("details"),
    null,
    "no <details> anywhere in the block, so no echo can be generated",
  );

  // And the fold is genuinely repeatable: policy opens it for the next live
  // phase and folds it again, which is the exact behaviour a recorded echo
  // would have disabled.
  await act(async () => {
    view.stream(
      [
        step("a"),
        step("b"),
        step("c", { status: "executing", completedAt: null }),
      ],
      "c",
    );
  });
  assert.equal(
    await view.settleToStepCount(3),
    3,
    "policy re-opens for new work",
  );

  await act(async () => {
    view.stream([step("a"), step("b"), step("c")]);
  });
  assert.equal(
    await view.settleToStepCount(0),
    0,
    "and folds again — policy transitions still work",
  );
});

test("a reader who opens a finished block keeps it open as the transcript moves on", async () => {
  const { act } = await import("@testing-library/react");
  const view = await renderBlock([step("a"), step("b")]);
  assert.equal(view.stepCount(), 0);

  await act(async () => {
    view.summary().click();
  });
  assert.equal(await view.settleToStepCount(2), 2);

  // A later append re-renders the block; the reader's choice must survive it.
  await act(async () => {
    view.stream([step("a"), step("b"), step("c")]);
  });
  assert.equal(
    await view.settleToStepCount(3),
    3,
    "the reader's choice outlives later policy transitions",
  );
});

test("a reader who folds a live block keeps it folded while work continues", async () => {
  const { act } = await import("@testing-library/react");
  const live = [
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ];
  const view = await renderBlock(live);
  assert.equal(view.stepCount(), 2, "live blocks open themselves");

  // While live there is no summary trigger, so the reader's route to folding is
  // the block's own disclosure change handler. Drive it the way a click would.
  await act(async () => {
    view.stream([step("a"), step("b")]);
  });
  assert.equal(await view.settleToStepCount(0), 0, "policy folded it");

  await act(async () => {
    view.summary().click();
  });
  assert.equal(await view.settleToStepCount(2), 2, "reader opened it");

  await act(async () => {
    view.stream(
      [
        step("a"),
        step("b"),
        step("c", { status: "executing", completedAt: null }),
      ],
      "c",
    );
  });
  assert.equal(
    await view.settleToStepCount(3),
    3,
    "the reader's open choice persists into the next live phase",
  );
});

// ── Rail ─────────────────────────────────────────────────────────────────────

test("the rail marks running, failed and settled steps with distinct glyph states", async () => {
  const view = await renderBlock(
    [
      step("a"),
      step("b", { isError: true, status: "failed" }),
      step("c", { status: "executing", completedAt: null }),
    ],
    { streamingItemId: "c" },
  );

  assert.deepEqual(view.glyphStates(), ["settled", "failed", "running"]);
  assert.equal(view.summary(), null, "a running step keeps the block live");
});

test("a running step pulses and a failed one does not", async () => {
  const view = await renderBlock(
    [
      step("a", { isError: true, status: "failed" }),
      step("b", { status: "executing", completedAt: null }),
    ],
    { streamingItemId: "b" },
  );
  const [failed, running] = view.qa("[data-step-state]");
  assert.deepEqual(
    view.pulseStates(),
    ["running"],
    "work in flight pulses and a settled failure does not",
  );
  assert.equal(
    failed.classList.contains("motion-safe:animate-pulse"),
    false,
    "a settled failure is not in flight",
  );
  assert.equal(
    running.classList.contains("motion-safe:animate-pulse"),
    true,
    "the running step is the one that pulses",
  );
});

/**
 * Reduced motion has to reach this pulse through the CLASS, not the hook.
 *
 * `useWorkBlockMotionEnabled` skips the fold's height animation, but the running
 * glyph's pulse is a CSS keyframe animation applied by a Tailwind utility, which
 * no hook can switch off — so the guard has to be `motion-safe:animate-pulse`.
 * Compiled with this repo's Tailwind (4.3.0), `animate-pulse` is
 * `animation: var(--animate-pulse)` with no guard, while
 * `motion-safe:animate-pulse` wraps exactly that declaration in
 * `@media (prefers-reduced-motion: no-preference)`. jsdom does not evaluate
 * media queries, so the assertion is on the class: the bare form would keep
 * pulsing forever for a reader who asked for no motion, and none of the app's 20
 * `prefers-reduced-motion: reduce` blocks matches `.animate-pulse` (all are
 * scoped to `buzz-*`/`motion-*`/`t-skel-*` classes), so nothing else would catch
 * it.
 *
 * The preference is irrelevant to which class is emitted, and that is the point:
 * asserting under BOTH settings is what proves the guard is in the class rather
 * than in a conditional that reduced motion could flip.
 */
test("a running glyph's pulse is guarded so reduced motion silences it", async () => {
  for (const reduced of [false, true]) {
    setPrefersReducedMotion(reduced);
    const view = await renderBlock(
      [step("a"), step("b", { status: "executing", completedAt: null })],
      { streamingItemId: "b" },
    );
    const [, running] = view.qa("[data-step-state]");
    assert.equal(
      running.classList.contains("motion-safe:animate-pulse"),
      true,
      `the pulse is reduced-motion-safe (prefersReducedMotion=${reduced})`,
    );
    assert.deepEqual(
      view.unguardedPulseStates(),
      [],
      `no glyph pulses unconditionally (prefersReducedMotion=${reduced})`,
    );
    const { cleanup } = await import("@testing-library/react");
    cleanup();
  }
});
