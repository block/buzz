import assert from "node:assert/strict";
import test from "node:test";

import {
  isReconciledFor,
  reconcileObserverArchive,
  startReconciliation,
} from "./useObserverArchiveSeed.ts";

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({ mergeShouldFail = false, explicitChoice = "unset" } = {}) {
  const calls = { merge: [] };
  // Simulates a per-pubkey localStorage map. "unset" means no choice stored.
  const choices = new Map();
  if (explicitChoice !== "unset") {
    // Pre-populate a choice for any pubkey that asks (single-pubkey tests).
    choices.set("__default__", explicitChoice);
  }

  return {
    calls,
    mergeSaveSubscriptionKinds: async (kind) => {
      if (mergeShouldFail) throw new Error("merge failed");
      calls.merge.push({ kind });
    },
    readExplicitChoice: (pubkey) => {
      if (choices.has(pubkey)) return choices.get(pubkey);
      if (choices.has("__default__")) return choices.get("__default__");
      return "unset";
    },
    setExplicitChoice: (pubkey, enabled) => {
      choices.set(pubkey, enabled);
    },
  };
}

/** Helper: wait for microtasks/promises to settle */
function tick() {
  return new Promise((r) => setTimeout(r, 0));
}

// ── Reconciliation always seeds 24200 ────────────────────────────────────────

test("test_reconcile_always_seeds_24200", async () => {
  const deps = makeDeps();
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200);
});

// ── Failure behavior ─────────────────────────────────────────────────────────

test("test_merge_failure_rejects", async () => {
  const deps = makeDeps({ mergeShouldFail: true });

  await assert.rejects(() => reconcileObserverArchive("pk1", deps), {
    message: "merge failed",
  });
});

// ── Explicit opt-out survives restart ────────────────────────────────────────

test("test_reconcile_explicit_optout_skips_merge", async () => {
  // Simulate a user who explicitly opted out (choice stored as false).
  const deps = makeDeps({ explicitChoice: false });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(
    deps.calls.merge.length,
    0,
    "merge must NOT fire when user has explicitly opted out",
  );
});

test("test_reconcile_storage_error_treated_as_fail_closed", async () => {
  // Storage errors return `true` (not "unset"), which means reconcile
  // treats them as an already-set choice and skips the merge.
  // This prevents auto-seeding from silently overriding a stored opt-out
  // that we couldn't read due to the error.
  const deps = makeDeps();
  // Override readExplicitChoice to simulate a storage error returning `true`.
  deps.readExplicitChoice = () => true;

  await reconcileObserverArchive("pk1", deps);

  assert.equal(
    deps.calls.merge.length,
    0,
    "merge must NOT fire when storage error returns fail-closed true",
  );
});

test("test_reconcile_explicit_optin_already_seeded_skips_merge", async () => {
  // Simulate a user who already has an explicit opt-in recorded (already
  // seeded on a prior run). The new tri-state model skips merge for any
  // non-"unset" choice — re-merging is idempotent but wasteful.
  const deps = makeDeps({ explicitChoice: true });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(
    deps.calls.merge.length,
    0,
    "merge must NOT fire when choice is already recorded as opted-in",
  );
});

test("test_reconcile_no_prior_choice_seeds_and_records_choice", async () => {
  const deps = makeDeps(); // no explicitChoice set
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1, "merge must fire on first run");
  // After reconciliation the choice should now be recorded as true.
  assert.equal(
    deps.readExplicitChoice("pk1"),
    true,
    "stored choice must be true after seed",
  );
});

test("test_reconcile_toggle_off_then_restart_does_not_remerge", async () => {
  // This is the exact failure mode Paul described:
  // 1. User toggles OFF → setExplicitChoice("pk1", false)
  // 2. App restarts → reconcileObserverArchive runs again
  // 3. Expected: merge is NOT called (opt-out preserved)
  const deps = makeDeps();

  // Simulate the card's handleObserverToggle(false) path: explicit opt-out stored.
  deps.setExplicitChoice("pk1", false);

  // Simulate app restart — reconciliation fires.
  await reconcileObserverArchive("pk1", deps);

  assert.equal(
    deps.calls.merge.length,
    0,
    "merge must NOT fire after explicit opt-out on app restart",
  );
});

// ── Startup ordering ─────────────────────────────────────────────────────────
//
// The two ordering tests that lived here drove `ArchiveSyncManager` directly.
// That manager is gone: archive sync runs in Rust and the renderer keeps only
// the start gate. The same invariant — no listener opens before kind 24200 is
// seeded — is now asserted against the real gate in useArchiveSync.test.mjs.

// ── Identity-scoped readiness (exercises exported isReconciledFor) ──────────

test("test_isReconciledFor_null_returns_false", () => {
  assert.equal(isReconciledFor(null, "pk1"), false);
  assert.equal(isReconciledFor(null, undefined), false);
});

test("test_isReconciledFor_undefined_pubkey_returns_false", () => {
  assert.equal(isReconciledFor("pk1", undefined), false);
});

test("test_isReconciledFor_matching_returns_true", () => {
  assert.equal(isReconciledFor("pk1", "pk1"), true);
});

test("test_isReconciledFor_mismatch_returns_false", () => {
  assert.equal(isReconciledFor("pkA", "pkB"), false);
});

test("test_identity_change_resets_readiness", async () => {
  let reconciledPubkey = null;

  // Identity A reconciles successfully.
  const depsA = makeDeps();
  await reconcileObserverArchive("pkA", depsA);
  reconciledPubkey = "pkA";
  assert.equal(
    isReconciledFor(reconciledPubkey, "pkA"),
    true,
    "A is reconciled",
  );

  // Identity changes to B — gate must be false before B reconciles.
  assert.equal(
    isReconciledFor(reconciledPubkey, "pkB"),
    false,
    "gate must be false for new identity before reconciliation",
  );

  // B reconciles successfully.
  const depsB = makeDeps();
  await reconcileObserverArchive("pkB", depsB);
  reconciledPubkey = "pkB";
  assert.equal(
    isReconciledFor(reconciledPubkey, "pkB"),
    true,
    "B is reconciled",
  );
  assert.equal(
    isReconciledFor(reconciledPubkey, "pkA"),
    false,
    "gate must be false for previous identity",
  );
});

test("test_identity_change_b_failure_stays_closed", async () => {
  let reconciledPubkey = null;

  // Identity A reconciles successfully.
  const depsA = makeDeps();
  await reconcileObserverArchive("pkA", depsA);
  reconciledPubkey = "pkA";

  // Identity changes to B — B's reconciliation fails.
  const depsB = makeDeps({ mergeShouldFail: true });
  try {
    await reconcileObserverArchive("pkB", depsB);
    reconciledPubkey = "pkB";
  } catch {
    // B failed — reconciledPubkey stays "pkA" (stale).
  }

  // Gate for B must be false (stale A pubkey !== current B).
  assert.equal(
    isReconciledFor(reconciledPubkey, "pkB"),
    false,
    "gate must be false when B reconciliation fails",
  );
});

// ── startReconciliation lifecycle (cancellation guard) ──────────────────────
//
// These exercise the actual effect/cleanup code path extracted into
// `startReconciliation`, rather than only the pure `isReconciledFor` helper
// or manually-sequenced fakes. Mirrors what React calls on unmount / before
// re-running an effect with new deps (identity switch).

test("test_startReconciliation_calls_onReady_after_success", async () => {
  const deps = makeDeps();
  const readyCalls = [];

  startReconciliation("pk1", deps, (pubkey) => readyCalls.push(pubkey));
  await tick();

  assert.deepEqual(readyCalls, ["pk1"]);
  assert.equal(deps.calls.merge.length, 1);
});

test("test_startReconciliation_unmount_before_resolve_suppresses_onReady", async () => {
  let resolveMerge;
  const mergePromise = new Promise((resolve) => {
    resolveMerge = resolve;
  });
  const deps = {
    mergeSaveSubscriptionKinds: () => mergePromise,
    readExplicitChoice: () => "unset",
    setExplicitChoice: () => {},
  };
  const readyCalls = [];

  const cancel = startReconciliation("pk1", deps, (pubkey) =>
    readyCalls.push(pubkey),
  );

  // Unmount (or re-run effect) before the merge resolves.
  cancel();
  resolveMerge();
  await tick();

  assert.deepEqual(
    readyCalls,
    [],
    "onReady must not fire for a cancelled reconciliation",
  );
});

test("test_startReconciliation_identity_switch_stale_completion_suppressed", async () => {
  let resolveMergeA;
  const mergePromiseA = new Promise((resolve) => {
    resolveMergeA = resolve;
  });
  const depsA = {
    mergeSaveSubscriptionKinds: () => mergePromiseA,
    readExplicitChoice: () => "unset",
    setExplicitChoice: () => {},
  };
  const depsB = makeDeps();
  const readyCalls = [];
  const onReady = (pubkey) => readyCalls.push(pubkey);

  // Start reconciling for pkA (pending), then switch identity to pkB before
  // A resolves — this is exactly what the hook's effect does when `pubkey`
  // changes: it calls the previous effect's cleanup (cancelA) before
  // starting the new effect.
  const cancelA = startReconciliation("pkA", depsA, onReady);
  cancelA();
  startReconciliation("pkB", depsB, onReady);

  // A's merge now resolves late — its stale completion must not fire.
  resolveMergeA();
  await tick();

  assert.deepEqual(
    readyCalls,
    ["pkB"],
    "only the current identity's completion should fire",
  );
});

test("test_startReconciliation_failure_does_not_call_onReady", async () => {
  const deps = makeDeps({ mergeShouldFail: true });
  const readyCalls = [];

  startReconciliation("pk1", deps, (pubkey) => readyCalls.push(pubkey));
  await tick();

  assert.deepEqual(readyCalls, [], "onReady must not fire on failure");
});

// ── Metric seed independence ─────────────────────────────────────────────────

test("test_metric_seed_remains_independently_deferrable", async () => {
  const deps = makeDeps();
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200, "must only touch kind 24200");
});
