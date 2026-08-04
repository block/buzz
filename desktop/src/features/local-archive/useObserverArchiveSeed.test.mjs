import assert from "node:assert/strict";
import test from "node:test";

import {
  isReconciledFor,
  reconcileObserverArchive,
  startReconciliation,
} from "./useObserverArchiveSeed.ts";
import { ArchiveSyncManager } from "./archiveSyncManager.ts";

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({ mergeShouldFail = false } = {}) {
  const calls = { merge: [] };

  return {
    calls,
    mergeSaveSubscriptionKinds: async (kind) => {
      if (mergeShouldFail) throw new Error("merge failed");
      calls.merge.push({ kind });
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
  await reconcileObserverArchive(deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200);
});

// ── Failure behavior ─────────────────────────────────────────────────────────

test("test_merge_failure_rejects", async () => {
  const deps = makeDeps({ mergeShouldFail: true });

  await assert.rejects(() => reconcileObserverArchive(deps), {
    message: "merge failed",
  });
});

// ── Startup ordering (real ArchiveSyncManager + real reconciler) ─────────────

test("test_archive_sync_blocked_until_reconciliation", async () => {
  let resolveMerge;
  const mergePromise = new Promise((resolve) => {
    resolveMerge = resolve;
  });

  const subscribeCalls = [];
  const fakeRelay = {
    subscribeLive(filter, _callback) {
      subscribeCalls.push(filter);
      return Promise.resolve(async () => {});
    },
  };

  const reconcilerDeps = {
    mergeSaveSubscriptionKinds: () => mergePromise,
  };

  const manager = new ArchiveSyncManager({
    relayClient: fakeRelay,
    listSaveSubscriptions: async () => [
      {
        scopeType: "owner_p",
        scopeValue: "pk1",
        kinds: [24200],
        identityPubkey: "pk1",
        relayUrl: "wss://r",
        createdAt: 0,
      },
    ],
    archiveEvents: async () => ({ persisted: 0, dropped: 0 }),
    onSubscriptionChange: () => () => {},
  });

  // Start reconciliation (pending — merge not yet resolved).
  const reconciling = reconcileObserverArchive(reconcilerDeps);

  // Before reconciliation resolves, manager must not have been started.
  await tick();
  assert.equal(
    subscribeCalls.length,
    0,
    "subscribeLive must not run before reconciliation",
  );

  // Resolve reconciliation — now start the manager (simulating the gate).
  resolveMerge();
  await reconciling;
  await manager.start();

  assert.ok(
    subscribeCalls.length > 0,
    "subscribeLive must run after gate opens",
  );
  const hasOwnerP = subscribeCalls.some((f) => f["#p"]?.length > 0);
  assert.ok(hasOwnerP, "subscription must use owner_p (#p) filter");
  const hasKind24200 = subscribeCalls.some((f) => f.kinds?.includes(24200));
  assert.ok(hasKind24200, "subscription filter must include kind 24200");

  manager.destroy();
});

test("test_archive_sync_blocked_on_reconciliation_rejection", async () => {
  const reconcilerDeps = makeDeps({ mergeShouldFail: true });

  const subscribeCalls = [];
  const fakeRelay = {
    subscribeLive(filter) {
      subscribeCalls.push(filter);
      return Promise.resolve(async () => {});
    },
  };

  const manager = new ArchiveSyncManager({
    relayClient: fakeRelay,
    listSaveSubscriptions: async () => [
      {
        scopeType: "owner_p",
        scopeValue: "pk1",
        kinds: [24200],
        identityPubkey: "pk1",
        relayUrl: "wss://r",
        createdAt: 0,
      },
    ],
    archiveEvents: async () => ({ persisted: 0, dropped: 0 }),
    onSubscriptionChange: () => () => {},
  });

  // Reconciliation rejects — gate must remain closed.
  let rejected = false;
  try {
    await reconcileObserverArchive(reconcilerDeps);
  } catch {
    rejected = true;
  }
  assert.ok(rejected, "reconciliation must reject on merge failure");

  // Manager must NOT start after failed reconciliation.
  assert.equal(
    subscribeCalls.length,
    0,
    "subscribeLive must not run after failed reconciliation",
  );

  manager.destroy();
});

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
  await reconcileObserverArchive(depsA);
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
  await reconcileObserverArchive(depsB);
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
  await reconcileObserverArchive(depsA);
  reconciledPubkey = "pkA";

  // Identity changes to B — B's reconciliation fails.
  const depsB = makeDeps({ mergeShouldFail: true });
  try {
    await reconcileObserverArchive(depsB);
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
  await reconcileObserverArchive(deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200, "must only touch kind 24200");
});
