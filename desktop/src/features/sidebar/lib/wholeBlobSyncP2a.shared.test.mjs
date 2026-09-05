// Shared P2a queue-until-bootstrap regression suite for WholeBlobSyncManager.
//
// Exports runWholeBlobP2aSuite({ label, Manager, publishEdit, makeNonEmptyStore,
// makeEditStore, makeRemoteStore }) and is exercised by both lane test files:
//   useChannelSections.test.mjs      — sections lane (ChannelSectionSyncManager)
//   useChannelSortPreference.test.mjs — sort lane    (ChannelSortSyncManager)
//
// Five causal regressions:
//
// T1 (load-bearing): fresh device, relay HAS a head, click during unresolved
//   bootstrap → edit publishes above relay head.
//   Mutation: drop `bootstrapStarted &&` guard in publish() → debounce fires
//   against {0,""} baseline → remoteAdvancedSince returns true → adopt.
//
// T2: blocked bootstrap → click → live peer head B arrives → bootstrap
//   resolves → B is adopted (normal LWW, not published over).
//   Mutation: releaseDeferred uses mutable lastRemoteHead instead of snapshot
//   → publishBaseline folds B in → pre-publish sees equality → publish over B.
//
// T3: non-empty mount, click during blocked bootstrap, relay confirms absent
//   → the edit (not the mount snapshot) is the published payload.
//   Mutation: drop `if (this.pendingStore === null)` guard in bootstrap's
//   publishFn → seed calls publish(localStore), bumps generation, overwrites
//   outbox with stale mount snapshot → edit lost.
//
// T4: failed bootstrap → click → first pre-publish fetch throws → live peer
//   head arrives via subscribeToX → retry must ADOPT, never publish over it.
//   Mutation: drop `!this.bootstrapFailedExternalHeadObserved` from the
//   failed-bootstrap exception → exception fires, publishes over the peer head.
//
// T5 (Kalvin's original sequence): failed bootstrap → click → no live head →
//   pre-publish fetch finds a relay head → edit publishes above it.
//   Mutation: remove the entire bootstrapFailed exception block from
//   fetchOwnBlobBeforePublish → hard-adopt against {0,""} fires, click lost.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  makeHookTimerBed,
  installEchoTauri,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.p2a";

/**
 * Register P2a queue-until-bootstrap causal regressions for a single lane.
 *
 * @param {object} opts
 * @param {string} opts.label         — Lane label used in test names ("sections"|"sort")
 * @param {Function} opts.Manager     — Concrete manager class (ChannelSectionSyncManager|ChannelSortSyncManager)
 * @param {Function} opts.publishEdit — (manager, store) => void — calls the lane publish method
 * @param {Function} opts.subscribe   — (manager, cb) => Promise<unsubscribe> — calls subscribeToX
 * @param {Function} opts.makeNonEmptyStore  — () => non-empty store (for mount / localStore arg)
 * @param {Function} opts.makeEditStore      — () => store representing the user's click
 * @param {Function} opts.makeMountStore     — () => stale mount snapshot (distinct from edit)
 * @param {Function} opts.makeRemoteStore    — () => remote head store (for relay mock)
 */
export function runWholeBlobP2aSuite({
  label,
  Manager,
  publishEdit,
  subscribe,
  makeNonEmptyStore,
  makeEditStore,
  makeMountStore,
  makeRemoteStore,
}) {
  // T1: load-bearing fresh-device case.
  // Mutation: revert publish() guard to unconditional timer scheduling.
  // Expected: edit publishes above relay head (publishCalls >= 1, adopted = 0).
  test(`P2a ${label}: click during unresolved bootstrap on fresh device publishes above the relay head`, async () => {
    let releaseBootstrap = null;
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t1-${label}`);

    let storedHead = [
      tauri.mintHead(makeRemoteStore(), 200, `evt-relay-${label}`),
    ];
    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res(storedHead);
        });
      return Promise.resolve(storedHead);
    });
    mock.method(relayClient, "publishEvent", (e) => {
      publishCalls++;
      storedHead = [e];
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t1-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Bootstrap blocked; click during unresolved bootstrap.
      const bootstrapPromise = manager.bootstrap(makeNonEmptyStore());
      publishEdit(manager, makeEditStore());

      // Release bootstrap → bootstrapResolved=true, releaseDeferred snaps
      // publishBaseline to {200,"evt-relay-<label>"}, schedules debounce.
      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Pre-publish fetch returns the relay head → baseline matches → publish.
      // Mutation: baseline was {0,""} → remoteAdvancedSince = true → adopt.
      assert.equal(adopted.length, 0, "edit must not be adopted away");
      assert.ok(publishCalls >= 1, "edit must publish above the relay head");
      assert.equal(
        manager.getPendingStore(),
        null,
        "pending cleared after publish",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T2: blocked bootstrap → click → live peer head B → bootstrap resolves → adopt B.
  // Mutation: releaseDeferred uses mutable lastRemoteHead (live B folds in) →
  //   pre-publish sees equality → publish over B instead of adopting.
  test(`P2a ${label}: post-click live peer head arriving before bootstrap resolves is adopted after bootstrap, not published over`, async () => {
    let releaseBootstrap = null;
    let publishCalls = 0;
    let liveCallback = null;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t2-${label}`);

    const bootHead = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-boot-${label}`,
    );
    const peerHead = tauri.mintHead(
      makeRemoteStore(),
      400,
      `evt-peer-${label}`,
    );

    // subscribeLive: capture the event callback so we can deliver peerHead.
    mock.method(relayClient, "subscribeLive", (_f, cb) => {
      liveCallback = cb;
      return Promise.resolve(async () => {});
    });

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Call 1 = bootstrap (blocked), returns bootHead.
      // Subsequent calls (pre-publish fetch) return peerHead, simulating a
      // genuine peer publish that arrived AFTER the click.
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res([bootHead]);
        });
      return Promise.resolve([peerHead]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t2-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Subscribe so subscribeLive callback is captured.
      await subscribe(manager, () => {});

      // Bootstrap blocked; click during unresolved bootstrap.
      const bootstrapPromise = manager.bootstrap(makeNonEmptyStore());
      publishEdit(manager, makeEditStore());

      // Deliver live peer head B BEFORE bootstrap resolves.
      // With the fix: releaseDeferred will use bootstrapResultHead snapshot
      // ({200,"evt-boot-<label>"}), not the mutable lastRemoteHead (which is
      // now {400,"evt-peer-<label>"}). So publishBaseline = {200,...} and B
      // remains a genuine advance at pre-publish time.
      while (liveCallback === null) await Promise.resolve();
      liveCallback(peerHead);
      // Give the async decrypt a tick to complete.
      for (let i = 0; i < 20; i++) await Promise.resolve();

      // Release bootstrap.
      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Pre-publish returns {400,"evt-peer"}: remoteAdvancedSince({400},{200}) = true
      // → adopt fires. Mutation (mutable lastRemoteHead): publishBaseline = {400,...},
      // pre-publish sees equality → publish over B.
      assert.equal(
        publishCalls,
        0,
        "edit must not publish over the live peer head",
      );
      assert.equal(
        adopted.length,
        1,
        "live peer head B must be adopted (normal LWW)",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T6: baseline regression — live B fully applies before click, then bootstrap
  //   resolves with older A. canonicalMax must keep the queue-time baseline B so
  //   the click publishes above B; plain replacement regresses baseline to A and
  //   the pre-publish fetch adopts B away, discarding the click.
  // Mutation: revert releaseDeferred to plain replacement
  //   (this.publishBaseline = { ...bootstrapResultHead }) →
  //   publishBaseline regresses B→A → pre-publish sees B as advance → adopt
  //   (0 publishes, 1 adopt).
  test(`P2a ${label}: click authored from a live peer head B publishes above B when bootstrap resolves with older head A`, async () => {
    let releaseBootstrap = null;
    let publishCalls = 0;
    let liveCallback = null;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t6-${label}`);

    // Head A: the head bootstrap will return (older, lower createdAt).
    const headA = tauri.mintHead(makeRemoteStore(), 200, `evt-boot-a-${label}`);
    // Head B: the live peer head that arrives and fully applies before the click
    // (higher createdAt, so canonicalMax picks B over A).
    const headB = tauri.mintHead(makeRemoteStore(), 400, `evt-live-b-${label}`);

    // subscribeLive: capture the callback so we can deliver headB.
    mock.method(relayClient, "subscribeLive", (_f, cb) => {
      liveCallback = cb;
      return Promise.resolve(async () => {});
    });

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Call 1 = bootstrap fetch (blocked), resolves to [headA].
      // Later calls (pre-publish fetch + confirmation) resolve to storedHead,
      // which starts as [headB] and is updated to [publishedEvent] after publish.
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res([headA]);
        });
      return Promise.resolve(storedHead);
    });
    let storedHead = [headB];
    mock.method(relayClient, "publishEvent", (e) => {
      publishCalls++;
      storedHead = [e]; // relay retains our event → confirmation succeeds
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t6-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Subscribe so the live callback is captured.
      await subscribe(manager, () => {});

      // Bootstrap blocked; NO edit pending yet.
      const bootstrapPromise = manager.bootstrap(makeNonEmptyStore());

      // Deliver live head B; let it fully apply (no edit in flight).
      while (liveCallback === null) await Promise.resolve();
      liveCallback(headB);
      for (let i = 0; i < 20; i++) await Promise.resolve();

      // Click authored from B: publish() freezes publishBaseline = B (lastRemoteHead).
      publishEdit(manager, makeEditStore());

      // Bootstrap resolves with older A.
      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();

      // releaseDeferred fires. Fix: canonicalMax(B, A) = B → publishBaseline
      // stays B. Pre-publish fetch returns storedHead=[headB]: remoteAdvancedSince(B, B)
      // = false → publish. publishEvent mock updates storedHead=[publishedEvent].
      // confirmRetainedHead fetches storedHead=[publishedEvent] → event.id matches
      // → confirmed → pending cleared.
      // Mutation (plain replace): publishBaseline = A → pre-publish sees B as
      // advance → adopt fires (0 publishes, 1 adopt) → click lost.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.ok(
        publishCalls >= 1,
        "click authored from B must publish above B, not adopt B away",
      );
      assert.equal(
        adopted.length,
        0,
        "B must not be adopted — click was authored from it",
      );
      assert.equal(
        manager.getPendingStore(),
        null,
        "pending cleared after successful publish",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T3: non-empty mount, click during blocked bootstrap, relay confirms absent
  //   → the edit (not the mount snapshot) is the published payload.
  // Mutation: drop `if (this.pendingStore === null)` guard in bootstrap's
  //   publishFn → seed calls publish(localStore), bumps generation, overwrites
  //   outbox with stale mount snapshot → stale mount is published, click lost.
  test(`P2a ${label}: relay-absent bootstrap does not clobber an already-pending edit with the stale mount snapshot`, async () => {
    let releaseBootstrap = null;
    let publishCalls = 0;
    const publishedPayloads = [];
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t3-${label}`);

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      // Call 1 = bootstrap (blocked), resolves absent.
      // Subsequent calls (pre-publish fetch, confirmation) return storedHead
      // so confirmation can confirm our event.
      if (fetchCalls === 1)
        return new Promise((res) => {
          releaseBootstrap = () => res([]);
        });
      return Promise.resolve(storedHead);
    });
    let storedHead = [];
    mock.method(relayClient, "publishEvent", (e) => {
      publishCalls++;
      publishedPayloads.push(e.content);
      storedHead = [e]; // relay retains our event
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t3-${label}`, RELAY);
      manager.setOnRemoteAdopted(() => {});

      // Mount store is the stale mount snapshot.
      const mountStore = makeMountStore();
      // Edit store is the click that happened DURING bootstrap.
      const editStore = makeEditStore();

      // Bootstrap blocked; user clicks BEFORE bootstrap resolves.
      const bootstrapPromise = manager.bootstrap(mountStore);
      publishEdit(manager, editStore);

      // Release bootstrap → relay absent, lastHead=0 → runBootstrap would
      // call publishFn(mountStore). With the fix: pendingStore !== null, so
      // the guard skips the seed call; the pending edit survives.
      while (releaseBootstrap === null) await Promise.resolve();
      releaseBootstrap();
      await bootstrapPromise;
      for (let i = 0; i < 50; i++) await Promise.resolve();

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // The edit must have published. We verify the pending store was cleared
      // (not adopted away) and the publish call happened.
      // Mutation: two publishes fire (one from editStore, one seed from mountStore
      // — in practice the seed clobbers the outbox and only mountStore is published).
      assert.ok(publishCalls >= 1, "the pending edit must publish");
      assert.equal(
        manager.getPendingStore(),
        null,
        "pending cleared after the edit publishes",
      );
      // The published plaintext must encode the clicked edit, not the stale mount snapshot.
      // With the mutation, bootstrap's seed-publish clobbers the edit in pendingStore
      // and the stale mountStore is published instead.
      const published = tauri.capturedPlaintext();
      assert.ok(published !== null, "a plaintext was captured");
      const parsed = JSON.parse(published);
      assert.ok(
        !JSON.stringify(parsed).includes('"mount"'),
        "published payload must not be the stale mount snapshot",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T4: failed bootstrap → click → first pre-publish fetch THROWS → live peer
  //   head arrives via subscribeToX → retry must adopt, never publish over it.
  // Mutation: drop `!this.bootstrapFailedExternalHeadObserved` from the
  //   failed-bootstrap exception in fetchOwnBlobBeforePublish → exception fires
  //   even though an external head was observed → publishes over peer.
  test(`P2a ${label}: live peer head observed after failed bootstrap disarms exception; retry adopts the peer head`, async () => {
    let publishCalls = 0;
    let liveCallback = null;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t4-${label}`);

    const peerHead = tauri.mintHead(
      makeRemoteStore(),
      300,
      `evt-t4-peer-${label}`,
    );

    mock.method(relayClient, "subscribeLive", (_f, cb) => {
      liveCallback = cb;
      return Promise.resolve(async () => {});
    });

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1) return Promise.reject(new Error("bootstrap fail"));
      // Call 2 = first pre-publish fetch: THROW (network error).
      if (fetchCalls === 2)
        return Promise.reject(new Error("pre-publish fail"));
      // Subsequent calls: return peerHead (retry pre-publish fetch).
      return Promise.resolve([peerHead]);
    });
    mock.method(relayClient, "publishEvent", () => {
      publishCalls++;
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t4-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Subscribe to capture live callback.
      await subscribe(manager, () => {});

      // Bootstrap fails: bootstrapFailed=true, bootstrapResolved=true.
      await manager.bootstrap(makeNonEmptyStore());

      // Click after failed bootstrap: publish() schedules immediately
      // (bootstrapResolved=true), publishBaseline={0,""}.
      publishEdit(manager, makeEditStore());

      // First debounce fires; pre-publish fetch throws → scheduleRetry.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Live peer head arrives AFTER the first pre-publish cycle threw.
      // This disarms the failed-bootstrap exception.
      while (liveCallback === null) await Promise.resolve();
      liveCallback(peerHead);
      for (let i = 0; i < 20; i++) await Promise.resolve();

      // Retry fires: pre-publish fetch returns peerHead.
      // bootstrapFailedExternalHeadObserved=true → exception suppressed.
      // remoteAdvancedSince(peerHead, {0,""}) = true → ADOPT.
      // Mutation: exception still fires → fold + publish over peerHead.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(publishCalls, 0, "must not publish over the peer head");
      assert.equal(adopted.length, 1, "peer head must be adopted");
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });

  // T5 (Kalvin's original P2a sequence): failed bootstrap → click → no live
  //   head → pre-publish fetch finds a relay head → edit publishes above it.
  // Mutation: remove the entire bootstrapFailed exception block →
  //   hard-adopt against {0,""} baseline fires, edit lost.
  test(`P2a ${label}: failed bootstrap with no external observation publishes the edit above the relay head`, async () => {
    let publishCalls = 0;
    const { fireDelay, restore } = makeHookTimerBed();
    const tauri = installEchoTauri(`pk-p2a-t5-${label}`);

    let storedHead = [
      tauri.mintHead(makeRemoteStore(), 100, `evt-t5-${label}`),
    ];

    let fetchCalls = 0;
    mock.method(relayClient, "fetchEvents", () => {
      fetchCalls++;
      if (fetchCalls === 1) return Promise.reject(new Error("bootstrap fail"));
      return Promise.resolve(storedHead);
    });
    mock.method(relayClient, "publishEvent", (e) => {
      publishCalls++;
      storedHead = [e];
      return Promise.resolve();
    });

    try {
      const manager = new Manager(`pk-p2a-t5-${label}`, RELAY);
      const adopted = [];
      manager.setOnRemoteAdopted((r) => adopted.push(r));

      // Bootstrap fails.
      await manager.bootstrap(makeNonEmptyStore());

      // Click after failed bootstrap.
      publishEdit(manager, makeEditStore());

      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // Pre-publish fetch returns the relay head: bootstrapFailed exception fires
      // (no external head observed), folds head in, publishes above it.
      // Mutation: exception removed → remoteAdvancedSince(head, {0,""}) = true
      // → adopt fires, publishCalls=0, adopted.length=1.
      assert.equal(adopted.length, 0, "edit must not be adopted");
      assert.ok(publishCalls >= 1, "edit must publish above the relay head");
      assert.equal(
        manager.getPendingStore(),
        null,
        "pending cleared after publish",
      );
      manager.destroy();
    } finally {
      tauri.restore();
      restore();
      mock.reset();
    }
  });
}
