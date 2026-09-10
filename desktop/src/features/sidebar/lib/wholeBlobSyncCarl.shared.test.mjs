// Carl-round regression suite for whole-blob sync (P1/C1, P2a-1/C2, P2b).
//
// P1/C1 — restored-outbox provenance (failed bootstrap):
//   A restored edit with queuedAt=200 vs a failed-bootstrap relay head at
//   createdAt=100 must PUBLISH (the edit is genuinely newer). Without the fix
//   the failed-bootstrap exception fires and publishes over the peer head, OR
//   the adopt path runs unconditionally and throws away the newer edit.
//   Mutation (P1): remove !pendingIsRestoredReplay guard from the exception
//     → exception fires, the edit publishes as if fresh.
//   Mutation (C1-adopt): remove pendingRestoredQueuedAt adopt-guard
//     → restored edit always adopts regardless of age.
//
// C2 — successful-bootstrap replay needs a baseline:
//   A restored edit with queuedAt=100 vs a successful bootstrap at H100
//   (createdAt=50) must PUBLISH (hook replays because queuedAt >= createdAt).
//   Without the fix publishBaseline stays {0,""} and the pre-publish fetch
//   sees H100 as an advance, adopting the edit away.
//   Mutation (C2): do not set publishBaseline from bootstrapResultHead in
//     publish(_, true) → baseline stays {0,""} → H100 adopted → no publish.
//
// P2a-1 (manager layer — blocked-bootstrap sequence):
//   Blocked H100 → click → live H102 arrives suppressed → bootstrap resolves
//   H100 → hook replay (isRestoredReplay=true) → H102 must remain a genuine
//   advance (ADOPT). At hook layer the replay is driven by publish(_, true)
//   which uses canonicalMax(current, bootstrapResultHead). Manager-level test
//   is sufficient here because the state machine (publishBaseline vs
//   bootstrapResultHead vs lastRemoteHead) is manager-internal.
//   Mutation: set publishBaseline = lastRemoteHead in publish(_, true)
//     → H102 folds into baseline → pre-publish sees equality → publish-over.
//
// P2b (manager layer — fetchRemoteBlob decrypt gap):
//   Click during periodic-fetch decrypt gap must not publish over the head.
//   Manager-level test is sufficient; the race is manager-internal.
//   Mutation: move recordRemoteHead back above decryptAndParse.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  makeHookTimerBed,
  makeHookStubs,
  installEchoTauri,
} from "./sidebarSyncTestHelpers.mjs";

const { stubRelay } = makeHookStubs();

// ─────────────────────────────────────────────────────────────────────────────
// Hook-layer suites: P1/C1 and C2 use the actual React hooks so the real
// bootstrap .then() callback, outbox read, queuedAt, and shouldReplay guard
// are exercised. Both sections and sort variants run via runWholeBlobCarlSuite.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Hook-layer P1/C1 and C2 regressions for a single whole-blob lane.
 *
 * @param {object} opts
 * @param {string}   opts.label              "sections"|"sort"
 * @param {string}   opts.outboxKeyPrefix    e.g. "buzz-channel-sections-outbox.v1"
 * @param {Function} opts.storageKey         (pubkey, relayUrl) => string (store key)
 * @param {Function} opts.writeOutboxKey     (pubkey, relayUrl) => v2 outbox key
 *                                           (legacy shared key — pre-v2 nonce path)
 * @param {Function} opts.readOutbox         (pubkey, relayUrl) => {store, queuedAt} | null
 * @param {Function} opts.useHook            the hook under test
 * @param {Function} opts.makeEditStore      () => store (the restored edit)
 * @param {Function} opts.makeRemoteStore    () => store (peer head content)
 * @param {Function} opts.assertHookState    (hookResult, label) => void (lane-specific hook oracle)
 */
export function runWholeBlobCarlSuite({
  label,
  outboxKeyPrefix: _outboxKeyPrefix,
  storageKey,
  writeOutboxKey,
  readOutbox,
  useHook,
  makeEditStore,
  makeRemoteStore,
  assertHookState,
}) {
  // ── P1/C1-publish: failed bootstrap → restored outbox (queuedAt=200) →
  //   relay head at createdAt=100 → MUST PUBLISH (restored edit is newer).
  //
  // Production sequence through the real hook:
  //   1. Outbox seeded with queuedAt=200 (prior session edit).
  //   2. Bootstrap fetch fails → bootstrapFailed=true.
  //   3. hook .then(): result.action="hold" → shouldReplay=true
  //      → publishSections(store, true, 200) → publish(_, true, 200)
  //      → publishBaseline = canonicalMax({0,""}, bootstrapResultHead={0,""}) = {0,""}
  //      → pendingRestoredQueuedAt = 200.
  //   4. writeOwnOutbox is called with nowSecs=200 — original stamp preserved.
  //   5. Debounce fires. fetchOwnBlobBeforePublish returns peerHead (createdAt=100).
  //      remoteAdvancedSince(100, {0,""}) = true.
  //      Failed-bootstrap exception: !pendingIsRestoredReplay suppresses it.
  //      Restored-replay adopt-guard: remote.createdAt(100) <= queuedAt(200)
  //      → PUBLISH (restored edit is genuinely newer than head).
  //
  // Mutations that must make this test red:
  //   M1 (exception not suppressed): remove !pendingIsRestoredReplay guard
  //      → exception fires → publishBaseline folds peerHead in → publish.
  //      Test PASSES with mutation: this mutation does not cause silent data-loss
  //      here, it publishes correctly but for the WRONG reason. The real defect
  //      is the stale-queuedAt remint. We catch that separately in C1-adopt.
  //
  //   M2 (adopt-guard absent, C1): remove pendingRestoredQueuedAt guard
  //      → restored replay always adopts when remoteAdvancedSince=true
  //      → publish=0, adopted=1 → test FAILS.
  //
  //   M3 (queuedAt remint): pass undefined instead of restoredQueuedAt to
  //      writeOwnOutbox → v2 outbox stamped at Date.now()/1000=300, not 200
  //      → queuedAt assertion below FAILS.
  test(`P1/C1 ${label}: failed-bootstrap hook replay — restored edit (queuedAt=200) must publish above older relay head (createdAt=100) and preserve original queuedAt in transferred outbox`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-c1-fail-${label}`;
    const relayUrl = `wss://r.c1fail.${label}`;

    const tauri = installEchoTauri(pubkey);
    const restoreRelay = stubRelay(relayClient);

    // Seed legacy outbox: queuedAt=200, store has the edit.
    const legacyKey = writeOutboxKey(pubkey, relayUrl);
    window.localStorage.setItem(
      legacyKey,
      JSON.stringify({ store: makeEditStore(), queuedAt: 200 }),
    );
    // Seed local store = edit store (so hook mounts with non-empty state).
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(makeEditStore()),
    );

    // peerHead: another peer's head, createdAt=100 (older than queuedAt=200).
    const peerHead = tauri.mintHead(
      makeRemoteStore(),
      100,
      `evt-c1-peer-${label}`,
    );
    peerHead.pubkey = pubkey;
    peerHead.kind = 30078;

    const publishCalls = [];
    let fetchCalls = 0;
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch: fail.
      if (fetchCalls === 1) return Promise.reject(new Error("bootstrap fail"));
      // Subsequent calls (pre-publish fetch): return peerHead.
      return [peerHead];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 300 * 1_000; // wall clock at t=300 — remint would stamp 300

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      // Bootstrap failed → hook .then() fired → shouldReplay=true (hold)
      // → publishSections(store, true, 200) called.
      // IMPORTANT 2: before the debounce fires, the v2 outbox must carry the
      // original queuedAt=200, NOT the reminted wall-clock value of 300.
      // Mutation (M3): pass undefined for nowSecs → stamps 300 → assertion fails.
      const outboxAfterTransfer = readOutbox(pubkey, relayUrl);
      assert.ok(
        outboxAfterTransfer !== null,
        `P1/C1 ${label}: v2 outbox must exist after hook replay call`,
      );
      assert.equal(
        outboxAfterTransfer.queuedAt,
        200,
        `P1/C1 ${label}: v2 outbox queuedAt must be the ORIGINAL 200, not the reminted wall-clock 300 — ` +
          `drop restoredQueuedAt override in writeOwnOutbox call → queuedAt=300`,
      );

      // Debounce scheduled. Fire it.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // With the fix: restored edit (queuedAt=200) > head (createdAt=100)
      //   → pendingRestoredQueuedAt guard → PUBLISH.
      // Mutation (C1-adopt): guard absent → adopt → publishCalls.length === 0.
      assert.ok(
        publishCalls.length > 0,
        `P1/C1 ${label}: restored edit (queuedAt=200) must publish above older head ` +
          `(createdAt=100) — drop pendingRestoredQueuedAt guard → adopt-away`,
      );
      // Verify the published store contains the edit, not the peer head.
      const plaintext = tauri.capturedPlaintext();
      assert.ok(plaintext !== null, "encrypt must have been called");
      const published = JSON.parse(plaintext);
      assert.deepEqual(
        published,
        makeEditStore(),
        `P1/C1 ${label}: published store must be the restored edit, not the peer head`,
      );
      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreRelay();
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });

  // ── P1/C1-stale: failed bootstrap → restored outbox (queuedAt=100) →
  //   relay head at createdAt=200 → MUST ADOPT (head is strictly newer).
  //
  // Bootstrap fails → shouldReplay=true → pendingRestoredQueuedAt=100.
  // Debounce: peerHead createdAt=200. !pendingIsRestoredReplay guard suppresses
  // the failed-bootstrap exception; restored adopt-guard: 200>100 → ADOPT.
  // Outbox cleared; zero publishes. Hook and storage reflect remote store.
  //
  // Mutation (!pendingIsRestoredReplay removed): exception fires → publishBaseline
  // absorbs 200 → publishes stale edit over newer head → test FAILS.
  test(`P1/C1-stale ${label}: failed-bootstrap hook replay — stale restored edit (queuedAt=100) must ADOPT newer relay head (createdAt=200), not publish over it`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-c1-stale-${label}`;
    const relayUrl = `wss://r.c1stale.${label}`;

    const tauri = installEchoTauri(pubkey);
    const restoreRelay = stubRelay(relayClient);

    // Seed legacy outbox: queuedAt=100 (stale — the relay head will be newer).
    const legacyKey = writeOutboxKey(pubkey, relayUrl);
    window.localStorage.setItem(
      legacyKey,
      JSON.stringify({ store: makeEditStore(), queuedAt: 100 }),
    );
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(makeEditStore()),
    );

    // newerHead: createdAt=200 (strictly newer than queuedAt=100).
    const newerHead = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-c1-stale-head-${label}`,
    );
    newerHead.pubkey = pubkey;
    newerHead.kind = 30078;

    const publishCalls = [];
    let fetchCalls = 0;
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      if (fetchCalls === 1) return Promise.reject(new Error("bootstrap fail"));
      return [newerHead];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 50 * 1_000; // well below queuedAt=100 so no clock confusion

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // With fix: stale edit (queuedAt=100) < head (createdAt=200) → ADOPT.
      // Mutation (!pendingIsRestoredReplay removed): exception fires → publishes stale edit over newer head.
      assert.equal(
        publishCalls.length,
        0,
        `P1/C1-stale ${label}: stale restored edit (queuedAt=100) must be ADOPTED AWAY by newer head ` +
          `(createdAt=200), not published over it — remove !pendingIsRestoredReplay guard → exception ` +
          `fires → publishes stale edit`,
      );

      // Flush React state updates triggered by onRemoteAdopted → setStore(applyRemote(remote)).
      // applyRemote's state updater writes to localStorage; act() flushes the commit phase.
      await act(async () => {
        for (let i = 0; i < 50; i++) await Promise.resolve();
      });

      // (a) Persisted lane storage must reflect the adopted remote store.
      // Mutation (onRemoteAdopted callback removed): adopted store never written → still editStore → fails.
      const persistedRaw = window.localStorage.getItem(
        storageKey(pubkey, relayUrl),
      );
      assert.ok(
        persistedRaw !== null,
        `P1/C1-stale ${label}: lane storage must exist after adopt`,
      );
      assert.deepEqual(
        JSON.parse(persistedRaw),
        makeRemoteStore(),
        `P1/C1-stale ${label}: lane storage must equal the adopted remote store — ` +
          `remove onRemoteAdopted callback → old edit store remains persisted`,
      );

      // (b) Hook/UI state must reflect the adopted remote store independently of storage.
      // Mutation (applyRemote returns prev): storage correct, React state stale → fails.
      assertHookState(hook.result.current, label);

      // (c) Losing own outbox must be cleared — head supersedes the stale edit.
      // Mutation (clearOutbox removed from clearPendingState): old outbox persists → not null → fails.
      assert.equal(
        readOutbox(pubkey, relayUrl),
        null,
        `P1/C1-stale ${label}: own outbox must be cleared after adopt — ` +
          `remove clearOutbox from clearPendingState → stale outbox remains, replays again after restart`,
      );

      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreRelay();
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });

  // ── C2: successful bootstrap → hook replay (queuedAt >= bootstrapHead.createdAt)
  //   → MUST PUBLISH above the bootstrap head, not adopt it away.
  //
  // Note: the hook layer always passes queuedAt through publishSections, so the
  // restored-replay adopt-guard (C1) also fires and protects the case where
  // remote.createdAt <= queuedAt. The independent C2 coverage is in the manager
  // layer (runWholeBlobC2Suite), where we drive publish(_, true, undefined) so
  // only the bootstrapResultHead baseline mechanism (C2) can protect the edit.
  // This hook test verifies the combined C1+C2 end-to-end path: outbox read,
  // queuedAt threading, shouldReplay guard, and the real .then() callback.
  //
  // Probe: queuedAt=100, H50 (createdAt=50), shouldReplay=true. With fix:
  // publishBaseline={50,id} → no advance → PUBLISH; without fix publishBaseline
  // stays {0,""} but C1 guard fires (50<=100) → also publishes. This test is
  // a combined integration path; mutation-causal C2 coverage lives below.
  test(`C2 ${label}: successful-bootstrap hook replay (queuedAt=100 >= head.createdAt=50) must publish above bootstrap head, not adopt it away`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-c2-${label}`;
    const relayUrl = `wss://r.c2.${label}`;

    const tauri = installEchoTauri(pubkey);
    const restoreRelay = stubRelay(relayClient);

    // Seed legacy outbox: queuedAt=100.
    const legacyKey = writeOutboxKey(pubkey, relayUrl);
    window.localStorage.setItem(
      legacyKey,
      JSON.stringify({ store: makeEditStore(), queuedAt: 100 }),
    );
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(makeEditStore()),
    );

    // Bootstrap head: H50 at createdAt=50.
    const H50 = tauri.mintHead(makeRemoteStore(), 50, `evt-c2-h50-${label}`);
    H50.pubkey = pubkey;
    H50.kind = 30078;
    // Tag so the hook's decryptAndParse can identify the d-tag.
    H50.tags = [
      ["d", label === "sections" ? "channel-sections" : "channel-sort"],
    ];

    const publishCalls = [];
    let fetchCalls = 0;
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch: return H50.
      if (fetchCalls === 1) return [H50];
      // Pre-publish fetch: return H50 (relay still has H50).
      return [H50];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 200 * 1_000;

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Combined C1+C2 path: restored edit (queuedAt=100) vs head (createdAt=50).
      // C2 fix: publishBaseline={50,id} → no advance → PUBLISH.
      // C1 guard also fires: 50<=100 → PUBLISH (provides redundant protection).
      assert.ok(
        publishCalls.length > 0,
        `C2 ${label}: hook replay (queuedAt=100 >= bootstrapHead.createdAt=50) ` +
          `must publish above bootstrap head`,
      );
      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreRelay();
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Manager-layer suites: P2a-1 and P2b test manager-internal state that cannot
// be driven from the hook layer without precise timing control. These are
// correctly labeled as manager-level tests.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Manager-layer C2 regression (successful-bootstrap replay needs a baseline).
 *
 * This is the mutation-causal test for C2. The hook layer cannot isolate C2
 * from C1 because whenever shouldReplay fires (queuedAt >= bootstrapHead),
 * queuedAt is threaded through and C1's guard also protects the same case.
 * At the manager layer we drive publish(_, true, undefined) — no queuedAt —
 * so C1's guard is disabled and only C2's bootstrapResultHead baseline can
 * prevent the adopt.
 *
 * Probe: bootstrap H50 → publish(store, true, undefined) → pre-publish H50
 *   → With fix: publishBaseline={50,id} → not advance → PUBLISH.
 *   → Mutation: publishBaseline={0,""} → H50 advance → adopt → publish=0.
 */
export function runWholeBlobC2Suite({
  label,
  Manager,
  publishEdit,
  publishReplay,
  subscribe,
  makeNonEmptyStore,
  makeEditStore,
  makeRemoteStore,
}) {
  test(`C2 ${label} (manager): successful-bootstrap replay without queuedAt must publish above bootstrap head; bootstrapResultHead baseline is the only protection`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-c2m-${label}`);

    const H50 = tauri.mintHead(makeRemoteStore(), 50, `evt-c2m-h50-${label}`);

    let fetchCalls = 0;
    mock.method(relayClient, "subscribeLive", (_f, cb) =>
      Promise.resolve(async () => {}),
    );
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Bootstrap fetch: return H50.
      if (fetchCalls === 1) return Promise.resolve([H50]);
      // Pre-publish fetch: return H50 (relay unchanged).
      return Promise.resolve([H50]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-c2m-${label}`, "wss://r.c2m");
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      await subscribe(manager, () => {});
      await manager.bootstrap(makeNonEmptyStore());
      for (let i = 0; i < 20; i++) await Promise.resolve();

      // Simulate hook replay WITHOUT queuedAt (undefined = no C1 guard).
      // Only C2's bootstrapResultHead baseline can prevent adopt here.
      publishReplay(manager, makeEditStore());

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls >= 1,
        true,
        `C2 ${label}: successful-bootstrap replay (no queuedAt) must PUBLISH above H50 — ` +
          `drop bootstrapResultHead from publish(_, true) → publishBaseline stays {0,""} → H50 adopted before publish`,
      );
      // (The post-publish confirmation may still adopt H50 if our event loses
      // the LWW confirm race — that is correct separate behaviour. The C2
      // defect is publish=0 + adopt=1 at the pre-publish fetch step.)
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

/**
 * Manager-layer P2a-1 regression (blocked-bootstrap baseline after hook replay).
 */
export function runWholeBlobP2a1Suite({
  label,
  Manager,
  publishEdit,
  publishReplay,
  subscribe,
  makeNonEmptyStore,
  makeEditStore,
  makeRemoteStore,
}) {
  // T-P2a1 (manager layer): blocked H100 → click → live H102 suppressed →
  //   bootstrap resolves H100 → publish(_, true) uses canonicalMax(H100,
  //   bootstrapResultHead=H100) = H100 → pre-publish returns H102 → ADOPT.
  //
  // Mutation: set publishBaseline = lastRemoteHead in publish(_, true)
  //   → publishBaseline = H102 → remoteAdvancedSince(H102, H102) = false
  //   → publishes pre-H102 content over H102 (H102's changes lost).
  test(`P2a-1 ${label} (manager): hook replay after blocked bootstrap must keep H100 baseline; H102 must be adopted as genuine advance`, async () => {
    let publishCalls = 0;
    let liveCallback = null;
    let releaseBootstrap = null;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a1-${label}`);

    const H100 = tauri.mintHead(makeRemoteStore(), 200, `evt-h100-${label}`);
    const H102 = tauri.mintHead(makeRemoteStore(), 400, `evt-h102-${label}`);

    mock.method(relayClient, "subscribeLive", (_f, cb) => {
      liveCallback = cb;
      return Promise.resolve(async () => {});
    });

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res([H100]);
        });
      return Promise.resolve([H102]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a1-${label}`, "wss://r.carl");
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      await subscribe(manager, () => {});

      const bootstrapPromise = manager.bootstrap(makeNonEmptyStore());
      publishEdit(manager, makeEditStore());

      while (liveCallback === null) await Promise.resolve();
      liveCallback(H102);
      for (let i = 0; i < 20; i++) await Promise.resolve();

      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();

      // Simulate hook .then() replay with isRestoredReplay=true.
      publishReplay(manager, makeEditStore());

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls,
        0,
        `P2a-1 ${label}: must ADOPT H102 (genuine advance), not publish over it`,
      );
      assert.equal(
        adopted.length,
        1,
        `P2a-1 ${label}: H102 must be adopted as genuine remote advance`,
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}

/**
 * Hook-layer P2a-1 regression (IMPORTANT 3).
 *
 * Drives the blocked-bootstrap sequence through the actual React hook so that
 * shouldReplay, outbox read, queuedAt threading, and the real .then() callback
 * are exercised. The manager-layer test (runWholeBlobP2a1Suite) verifies the
 * baseline state-machine; this test verifies the hook seam that feeds it.
 *
 * Sequence: park bootstrap → mount → click (writeOutbox queuedAt=50) →
 * deliver H102 live (suppressed by hasPendingEdit) → release bootstrap H100 →
 * .then() fires shouldReplay=true → debounce → publishBaseline={30,H100};
 * H102 advance → ADOPT. Hook and storage reflect H102; outbox cleared.
 *
 * Causality mutation: set publishBaseline = lastRemoteHead in publish(_, true)
 *   → publishBaseline = H102 → pre-publish sees equality → publishes over H102.
 */
export function runWholeBlobP2a1HookSuite({
  label,
  storageKey,
  readOutbox,
  useHook,
  makeEdit,
  makeRemoteStore,
  assertHookState,
}) {
  test(`P2a-1 ${label} (hook): blocked-bootstrap real hook replay — H102 must be adopted as genuine advance, not published over`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-p2a1h-${label}`;
    const relayUrl = `wss://r.p2a1h.${label}`;

    const tauri = installEchoTauri(pubkey);
    const restoreRelay = stubRelay(relayClient);

    // Date.now frozen at t=50s so click writes queuedAt=50.
    // H100: createdAt=30 (bootstrap head, shouldReplay = 50>=30 = true).
    // H102: createdAt=200 (live head, a genuine advance over bootstrap).
    const origDateNow = Date.now;
    Date.now = () => 50 * 1_000;

    const H100 = tauri.mintHead(
      makeRemoteStore(),
      30,
      `evt-h100-p2a1h-${label}`,
    );
    H100.pubkey = pubkey;
    H100.kind = 30078;
    H100.tags = [
      ["d", label === "sections" ? "channel-sections" : "channel-sort"],
    ];

    const H102 = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-h102-p2a1h-${label}`,
    );
    H102.pubkey = pubkey;
    H102.kind = 30078;

    const publishCalls = [];
    let liveCallback = null;
    let releaseBootstrap = null;
    let fetchCalls = 0;

    relayClient.subscribeLive = async (_f, cb) => {
      liveCallback = cb;
      return async () => {};
    };
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch: park until released.
      if (fetchCalls === 1) {
        return new Promise((res) => {
          releaseBootstrap = () => res([H100]);
        });
      }
      // Pre-publish fetch: H102 is now the relay head.
      return [H102];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Bootstrap is parked. Make a click through the real hook API.
      await act(async () => {
        makeEdit(hook.result.current);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Deliver H102 live — hasPendingEdit suppresses it, publishBaseline stays {0,""}.
      while (liveCallback === null) await Promise.resolve();
      await act(async () => {
        liveCallback(H102);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Release bootstrap with H100 → hook .then() fires:
      //   result.action="apply-remote"; shouldReplay = (outbox.queuedAt=50 >= H100.createdAt=30) = true
      //   → publishSections(outbox.store, true, 50) naturally, no manual call.
      // Also bootstrapResultHead = H100 = {createdAt:30, ...}.
      while (releaseBootstrap === null) await Promise.resolve();
      await act(async () => {
        releaseBootstrap();
        for (let i = 0; i < 60; i++) await Promise.resolve();
      });

      // Fire debounce. publish(_, true, 50): publishBaseline={30,H100}; pre-publish
      // fetch returns H102 (200>30) → remoteAdvancedSince=true; C1 adopt-guard
      // (200>queuedAt=50) → ADOPT. Mutation: publishBaseline=H102 → no advance → PUBLISH.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls.length,
        0,
        `P2a-1 ${label} (hook): real .then() replay with bootstrapResultHead baseline — ` +
          `H102 must be ADOPTED as a genuine advance, not published over — ` +
          `set publishBaseline=lastRemoteHead in publish(_, true) → H102 folds into baseline → publishes over H102`,
      );

      // Flush React state updates triggered by onRemoteAdopted → setStore(applyRemote(remote)).
      // applyRemote's state updater writes to localStorage; act() flushes the commit phase.
      await act(async () => {
        for (let i = 0; i < 50; i++) await Promise.resolve();
      });

      // Hook state and lane storage must reflect the adopted H102 store.
      // Mutation (onRemoteAdopted callback removed): H102 never written to storage → old edit persists → fails.
      const persistedRaw = window.localStorage.getItem(
        storageKey(pubkey, relayUrl),
      );
      assert.ok(
        persistedRaw !== null,
        `P2a-1 ${label} (hook): lane storage must exist after H102 adopt`,
      );
      assert.deepEqual(
        JSON.parse(persistedRaw),
        makeRemoteStore(),
        `P2a-1 ${label} (hook): lane storage must equal the adopted H102 store — ` +
          `remove onRemoteAdopted callback → own edit store remains persisted`,
      );

      // Hook/UI state must reflect the adopted H102 store independently of storage.
      // Mutation (applyRemote returns prev): storage correct, React state stale → fails.
      assertHookState(hook.result.current, label);

      // Own outbox written by the click must be cleared after H102 supersedes it.
      // Mutation (onRemoteAdopted callback removed): adopt never fires, outbox not cleared → not null → fails.
      assert.equal(
        readOutbox(pubkey, relayUrl),
        null,
        `P2a-1 ${label} (hook): own outbox must be cleared after H102 adopt — ` +
          `remove onRemoteAdopted callback → click outbox remains, replays stale edit on next restart`,
      );

      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreRelay();
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });
}

/**
 * Manager-layer C3 regression (confirmRetainedHead decrypt gap).
 *
 * Generation A publishes → confirm fetch finds foreign winner B → without the
 * fix, canonical tuple advances to B before decrypt; a fresh Click C then
 * freezes baseline=B; decrypt returns B; stale-gen adopt does not advance state
 * for C; C's pre-publish fetch sees B == its baseline and publishes pre-B
 * content OVER B (B's changes lost).
 *
 * Fix: advance lastRemoteHead for a foreign winner ONLY after decrypt succeeds.
 * Mutation: move recordRemoteHead back above decryptAndParse for non-own-ID case.
 */
export function runWholeBlobC3Suite({
  label,
  Manager,
  publishEdit,
  makeEditStore,
  makeRemoteStore,
}) {
  test(`C3 ${label} (manager): confirmRetainedHead must not advance lastRemoteHead before decrypt; click during confirm decrypt gap must not publish over foreign winner`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-c3-${label}`);

    // B: the foreign winner — createdAt=200 (must be > A's publish timestamp).
    // A's publish timestamp = clampPublishCreatedAt(0) = max(floor(Date.now/1000), 1).
    // We mock Date.now to return 50s so A's createdAt=50 < B's createdAt=200.
    const B = tauri.mintHead(makeRemoteStore(), 200, `evt-c3-b-${label}`);
    const origDateNow = Date.now;
    Date.now = () => 50 * 1_000; // A publishes at createdAt=50

    let releaseDecrypt = null;
    const orig = globalThis.window.__TAURI_INTERNALS__;
    let decryptCallCount = 0;
    globalThis.window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => {
        if (cmd === "nip44_decrypt_from_self") {
          decryptCallCount++;
          // The first decrypt call is from confirmRetainedHead — both bootstrap
          // and pre-publish preflight return absent so no decrypt fires there.
          // Gate the very first decrypt call (confirmRetainedHead's foreign winner).
          if (decryptCallCount === 1) {
            return new Promise((res, rej) => {
              if (releaseDecrypt === null) {
                // First gated call — park it.
                releaseDecrypt = () =>
                  orig.invoke(cmd, args).then(res).catch(rej);
              } else {
                // Subsequent calls resolve immediately.
                orig.invoke(cmd, args).then(res).catch(rej);
              }
            });
          }
          return orig.invoke(cmd, args);
        }
        return orig.invoke(cmd, args);
      },
    };

    let fetchCalls = 0;
    let publishedEventId = null;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // 1: bootstrap fetch — absent (no remote, so bootstrap result = {0,""}).
      if (fetchCalls === 1) return Promise.resolve([]);
      // 2: pre-publish preflight — absent (no head, publish proceeds).
      if (fetchCalls === 2) return Promise.resolve([]);
      // 3: confirmRetainedHead — returns foreign winner B (not our event).
      return Promise.resolve([B]);
    });
    mock.method(relayClient, "publishEvent", (evt) => {
      publishedEventId = evt.id ?? `our-evt-${publishCalls}`;
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-c3-${label}`, "wss://r.c3");
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Bootstrap: absent. baseline stays {0,""}.
      await manager.bootstrap(makeEditStore());

      // Click A: publish starts.
      publishEdit(manager, makeEditStore());
      await fireDelay(2000);
      // Let publish + confirm fetch start, then park on the confirm decrypt.
      for (let i = 0; i < 30; i++) await Promise.resolve();

      assert.ok(
        releaseDecrypt !== null,
        "confirm decrypt gate must have been hit (fetchCalls >= 3 and decrypt started)",
      );

      // Click C: arrives DURING the confirm decrypt gap. With mutation:
      // lastRemoteHead was already advanced to B → publishBaseline = B.
      // Pre-publish fetch returns B → remoteAdvancedSince(B, B) = false
      //   → publishes pre-B content OVER B.
      // With fix: lastRemoteHead = {0,""} → publishBaseline = {0,""}.
      // Pre-publish fetch returns B → remoteAdvancedSince(B, {0,""}) = true
      //   → ADOPT (B is a genuine advance; do not publish over it).
      publishEdit(manager, makeEditStore());

      releaseDecrypt();
      for (let i = 0; i < 20; i++) await Promise.resolve();

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // With fix: Click C's pre-publish fetch sees B as a genuine advance and
      // adopts it. publishCalls = 1 (A's publish only; C is adopted).
      // With mutation: publishCalls = 2 (A publishes, C also publishes over B).
      assert.equal(
        publishCalls,
        1,
        `C3 ${label}: click during confirmRetainedHead decrypt gap must NOT publish over foreign winner — ` +
          `move recordRemoteHead before decrypt → C baseline=B → C publishes over B`,
      );
      manager.destroy();
    } finally {
      globalThis.window.__TAURI_INTERNALS__ = orig;
      Date.now = origDateNow;
      restore();
      mock.reset();
    }
  });
}

/**
 * Manager-layer P2b regression (fetchRemoteBlob decrypt-gap).
 */
export function runWholeBlobP2bSuite({
  label,
  Manager,
  publishEdit,
  makeEditStore,
  makeRemoteStore,
}) {
  test(`P2b ${label} (manager): fetchRemoteBlob must not advance lastRemoteHead before decryptAndParse succeeds; click during decrypt gap must not publish over fetched head`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2b-${label}`);

    const H = tauri.mintHead(makeRemoteStore(), 200, `evt-p2b-h-${label}`);

    let releaseDecrypt = null;
    const orig = globalThis.window.__TAURI_INTERNALS__;
    let decryptCallCount = 0;
    globalThis.window.__TAURI_INTERNALS__ = {
      invoke: (cmd, args) => {
        if (cmd === "nip44_decrypt_from_self") {
          decryptCallCount++;
          if (decryptCallCount === 1) {
            return new Promise((res, rej) => {
              releaseDecrypt = () =>
                orig.invoke(cmd, args).then(res).catch(rej);
            });
          }
          return orig.invoke(cmd, args);
        }
        return orig.invoke(cmd, args);
      },
    };

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1) return Promise.resolve([]);
      if (fetchCalls === 2) return Promise.resolve([H]);
      return Promise.resolve([H]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2b-${label}`, "wss://r.carl");
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      await manager.bootstrap(makeEditStore());

      const fetchPromise = manager.fetchRemoteBlob();
      for (let i = 0; i < 10; i++) await Promise.resolve();

      publishEdit(manager, makeEditStore());

      assert.ok(releaseDecrypt !== null, "decrypt gate must have been hit");
      releaseDecrypt();
      await fetchPromise;
      for (let i = 0; i < 20; i++) await Promise.resolve();

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls,
        0,
        `P2b ${label}: click during fetchRemoteBlob decrypt gap must not publish over the fetched head`,
      );
      assert.equal(
        adopted.length >= 1,
        true,
        `P2b ${label}: the fetched head must be adopted (genuine advance)`,
      );
      manager.destroy();
    } finally {
      globalThis.window.__TAURI_INTERNALS__ = orig;
      restore();
      mock.reset();
    }
  });
}
