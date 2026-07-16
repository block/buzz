import assert from "node:assert/strict";
import test from "node:test";

import {
  isReconciledFor,
  reconcileObserverArchive,
} from "./useObserverArchiveSeed.ts";
import { ArchiveSyncManager } from "./archiveSyncManager.ts";

// ── Fake deps factory ────────────────────────────────────────────────────────

function makeDeps({
  policyOn = false,
  mergeShouldFail = false,
  flagShouldFail = false,
} = {}) {
  const calls = { merge: [], setChoice: [] };

  return {
    calls,
    observerArchiveDefaultEnabled: async () => {
      if (flagShouldFail) throw new Error("flag check failed");
      return policyOn;
    },
    mergeSaveSubscriptionKinds: async (kind) => {
      if (mergeShouldFail) throw new Error("merge failed");
      calls.merge.push({ kind });
    },
    setExplicitChoice: (pubkey, enabled) => {
      calls.setChoice.push({ pubkey, enabled });
    },
  };
}

/** Helper: wait for microtasks/promises to settle */
function tick() {
  return new Promise((r) => setTimeout(r, 0));
}

// ── Internal policy build ────────────────────────────────────────────────────

test("test_internal_policy_marker_null_seeds_24200", async () => {
  const deps = makeDeps({ policyOn: true });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200);
  assert.deepEqual(deps.calls.setChoice, [{ pubkey: "pk1", enabled: true }]);
});

test("test_internal_policy_marker_0_still_seeds_24200", async () => {
  const deps = makeDeps({ policyOn: true });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1, "must merge even with stale marker");
  assert.equal(deps.calls.merge[0].kind, 24200);
});

test("test_internal_policy_marker_1_still_seeds_24200", async () => {
  const deps = makeDeps({ policyOn: true });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1, "must reconcile even with marker 1");
});

// ── OSS build — policy-off is a pure no-op ──────────────────────────────────

test("test_oss_marker_null_no_merge", async () => {
  const deps = makeDeps({ policyOn: false });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 0, "OSS must not merge");
  assert.equal(deps.calls.setChoice.length, 0, "OSS must not write marker");
});

test("test_oss_marker_0_no_merge", async () => {
  const deps = makeDeps({ policyOn: false });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 0, "OSS must not merge");
  assert.equal(deps.calls.setChoice.length, 0, "OSS must not write marker");
});

test("test_oss_marker_1_no_merge", async () => {
  const deps = makeDeps({ policyOn: false });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 0, "OSS must not merge");
  assert.equal(deps.calls.setChoice.length, 0, "OSS must not write marker");
});

// ── Failure behavior ─────────────────────────────────────────────────────────

test("test_merge_failure_rejects_no_marker_persisted", async () => {
  const deps = makeDeps({ policyOn: true, mergeShouldFail: true });

  await assert.rejects(() => reconcileObserverArchive("pk1", deps), {
    message: "merge failed",
  });
  assert.equal(
    deps.calls.setChoice.length,
    0,
    "must not persist marker on merge failure",
  );
});

test("test_flag_check_failure_rejects", async () => {
  const deps = makeDeps({ flagShouldFail: true });

  await assert.rejects(() => reconcileObserverArchive("pk1", deps), {
    message: "flag check failed",
  });
  assert.equal(deps.calls.merge.length, 0);
  assert.equal(deps.calls.setChoice.length, 0);
});

// ── Startup ordering (real ArchiveSyncManager + real reconciler) ─────────────

test("test_archive_sync_blocked_until_reconciliation", async () => {
  let resolveFlag;
  const flagPromise = new Promise((resolve) => {
    resolveFlag = resolve;
  });

  const subscribeCalls = [];
  const fakeRelay = {
    subscribeLive(filter, _callback) {
      subscribeCalls.push(filter);
      return Promise.resolve(async () => {});
    },
  };

  const reconcilerDeps = {
    observerArchiveDefaultEnabled: () => flagPromise,
    mergeSaveSubscriptionKinds: async () => {},
    setExplicitChoice: () => {},
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

  // Start reconciliation (pending — flag check not yet resolved).
  const reconciling = reconcileObserverArchive("pk1", reconcilerDeps);

  // Before reconciliation resolves, manager must not have been started.
  await tick();
  assert.equal(
    subscribeCalls.length,
    0,
    "subscribeLive must not run before reconciliation",
  );

  // Resolve reconciliation — now start the manager (simulating the gate).
  resolveFlag(true);
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
  const reconcilerDeps = makeDeps({ policyOn: true, mergeShouldFail: true });

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
    await reconcileObserverArchive("pk1", reconcilerDeps);
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
  const depsA = makeDeps({ policyOn: true });
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
  const depsB = makeDeps({ policyOn: true });
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
  const depsA = makeDeps({ policyOn: true });
  await reconcileObserverArchive("pkA", depsA);
  reconciledPubkey = "pkA";

  // Identity changes to B — B's reconciliation fails.
  const depsB = makeDeps({ policyOn: true, mergeShouldFail: true });
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

// ── Metric seed independence ─────────────────────────────────────────────────

test("test_metric_seed_remains_independently_deferrable", async () => {
  const deps = makeDeps({ policyOn: true });
  await reconcileObserverArchive("pk1", deps);

  assert.equal(deps.calls.merge.length, 1);
  assert.equal(deps.calls.merge[0].kind, 24200, "must only touch kind 24200");
});
