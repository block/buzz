// Carl P2 second-click regression (IMPORTANT/P2, Carl review 5096781519).
//
// Two timing variants — both must hold:
//
// Variant A (debounce window): A1 click pending (debounce timer running) →
//   live H102 suppressed → A2 second click → debounce fires.
//   Mutation (unconditional reseed / wasIdle || !inDebounceWindow):
//     publishBaseline = H102 → A2 published over H102.
//
// Variant B (in-flight): A1's debounce fires → A1 preflight parked →
//   live H102 suppressed → A2 second click → A1 preflight released (stale) →
//   A2 cycle runs. Mutation (wasIdle || !inDebounceWindow, timer already null):
//     publishBaseline = H102 → A2 published over H102.
//
// With correct fix (wasIdle || lastIsOwnAttempt — seed only for new sequence
// or own in-flight attempt; preserve for suppressed live remote):
//   Both variants: H102 ∉ ambiguousAttemptIds → lastIsOwnAttempt=false →
//   publishBaseline preserved as {0,""} → remoteAdvancedSince = true
//   → ADOPT H102. H102's remote-only changes are preserved.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  makeHookTimerBed,
  installEchoTauri,
} from "./sidebarSyncTestHelpers.mjs";

/**
 * Hook-layer Carl P2 second-click regression for a single whole-blob lane.
 *
 * @param {object}   opts
 * @param {string}   opts.label           "sections"|"sort"
 * @param {Function} opts.useHook         the hook under test
 * @param {Function} opts.makeEdit1       (hookResult) => void — first click
 * @param {Function} opts.makeEdit2       (hookResult) => void — second click
 * @param {Function} opts.makeRemoteStore () => store (live remote H102 content)
 */
export function runWholeBlobCarlP2SecondClickSuite({
  label,
  useHook,
  makeEdit1,
  makeEdit2,
  makeRemoteStore,
}) {
  // --- Variant A: second click while debounce timer is still running -----------
  test(`Carl-P2 ${label} [debounce-window]: second click must not reseed baseline from suppressed live head — H102 must remain a genuine advance (adopt)`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-p2-sc-dw-${label}`;
    const relayUrl = `wss://r.p2sc.dw.${label}`;

    const tauri = installEchoTauri(pubkey);

    const H102 = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-h102-p2sc-dw-${label}`,
    );
    H102.pubkey = pubkey;
    H102.kind = 30078;
    H102.tags = [
      ["d", label === "sections" ? "channel-sections" : "channel-sort"],
    ];

    const live = { cb: null };
    const origFetch = relayClient.fetchEvents;
    const origSubscribeLive = relayClient.subscribeLive;
    const origPublish = relayClient.publishEvent;
    const origReconnects = relayClient.subscribeToReconnects;

    const publishCalls = [];
    let fetchCalls = 0;

    relayClient.fetchEvents = async () => {
      fetchCalls++;
      if (fetchCalls === 1) return []; // bootstrap: no remote head
      return [H102]; // pre-publish: H102 is now the relay head
    };
    relayClient.subscribeLive = async (_f, cb) => {
      live.cb = cb;
      return async () => {};
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };
    relayClient.subscribeToReconnects = () => () => {};

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 80; i++) await Promise.resolve();
      });

      // A1: first click — debounce timer scheduled at 2000ms.
      await act(async () => {
        makeEdit1(hook.result.current);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // H102 arrives live while debounce is running — suppressed by hasPendingEdit.
      assert.ok(
        live.cb !== null,
        `${label}: subscribeLive callback must be registered`,
      );
      await act(async () => {
        live.cb(H102);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // A2: second click — debounce timer still running (pendingStore !== null).
      // Fix: wasIdle=false, H102 ∉ ambiguousAttemptIds → lastIsOwnAttempt=false
      // → baseline preserved {0,""}. Mutation catches this via Variant B.
      await act(async () => {
        makeEdit2(hook.result.current);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Fire debounce. Pre-publish fetch returns H102.
      // Fix: baseline={0,""} → remoteAdvancedSince → ADOPT. publishCalls===0.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls.length,
        0,
        `Carl-P2 ${label} [debounce-window]: H102 must be adopted — ` +
          `reseeding baseline from suppressed live head silently overwrites H102`,
      );

      hook.unmount();
    } finally {
      cleanup();
      tauri.restore();
      relayClient.fetchEvents = origFetch;
      relayClient.subscribeLive = origSubscribeLive;
      relayClient.publishEvent = origPublish;
      relayClient.subscribeToReconnects = origReconnects;
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });

  // --- Variant B: second click while A1's publish is in flight -----------------
  // A1's debounce has fired (debounceTimer=null, publishInFlight=true).
  // H102 arrives suppressed. A2 click: wasIdle=false, H102 ∉ ambiguousAttemptIds
  // → lastIsOwnAttempt=false. Old predicate (wasIdle || !inDebounceWindow) reseeds
  // baseline to H102 → A2 publishes over H102.
  // Correct predicate (wasIdle || lastIsOwnAttempt): preserves → adopts H102.
  test(`Carl-P2 ${label} [in-flight]: second click after debounce fired must not reseed baseline from suppressed live head — H102 must remain a genuine advance (adopt)`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const pubkey = `pk-p2-sc-if-${label}`;
    const relayUrl = `wss://r.p2sc.if.${label}`;

    const tauri = installEchoTauri(pubkey);

    const H102 = tauri.mintHead(
      makeRemoteStore(),
      200,
      `evt-h102-p2sc-if-${label}`,
    );
    H102.pubkey = pubkey;
    H102.kind = 30078;
    H102.tags = [
      ["d", label === "sections" ? "channel-sections" : "channel-sort"],
    ];

    const live = { cb: null };
    const origFetch = relayClient.fetchEvents;
    const origSubscribeLive = relayClient.subscribeLive;
    const origPublish = relayClient.publishEvent;
    const origReconnects = relayClient.subscribeToReconnects;

    const publishCalls = [];
    let fetchCalls = 0;

    // A1-preflight resolver: lets us park A1's fetch until after A2 clicks.
    let releaseA1Preflight = null;

    relayClient.fetchEvents = async () => {
      fetchCalls++;
      if (fetchCalls === 1) return []; // bootstrap
      if (fetchCalls === 2) {
        // A1 preflight — park until we've delivered H102 and clicked A2.
        await new Promise((resolve) => {
          releaseA1Preflight = resolve;
        });
        return []; // returns empty so A1 would publish — but gen CAS exits it
      }
      // fetchCalls >= 3: A2 preflight — H102 is now the relay head.
      return [H102];
    };
    relayClient.subscribeLive = async (_f, cb) => {
      live.cb = cb;
      return async () => {};
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };
    relayClient.subscribeToReconnects = () => () => {};

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();

    let hook = null;
    try {
      // Mount + bootstrap.
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 80; i++) await Promise.resolve();
      });

      // A1 click — publishBaseline frozen to {0,""} (wasIdle=true).
      await act(async () => {
        makeEdit1(hook.result.current);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Fire A1's debounce: debounceTimer→null, startCycle()→doPublish(A1,1)
      // → fetchOwnBlobBeforePublish → fetchEvents call #2 → PARKED.
      // publishInFlight=true, pendingStore=A1, pendingGeneration=1.
      await act(async () => {
        // fireDelay fires the timer fn and drains microtasks; the async
        // doPublish chain starts but parks at the awaited fetchEvents.
        await fireDelay(2000);
        for (let i = 0; i < 30; i++) await Promise.resolve();
      });

      // H102 arrives while A1's preflight is parked — suppressed by hasPendingEdit.
      assert.ok(
        live.cb !== null,
        `${label}: subscribeLive callback must be registered`,
      );
      await act(async () => {
        live.cb(H102);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // A2 click — pendingStore !== null (A1 still pending), debounceTimer=null
      // (timer already fired). Fix: wasIdle=false, H102 ∉ ambiguousAttemptIds
      // → lastIsOwnAttempt=false → baseline preserved {0,""}.
      // Mutation: wasIdle || !inDebounceWindow → inDebounceWindow=false → reseeds to H102.
      await act(async () => {
        makeEdit2(hook.result.current);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Release A1's parked preflight → returns [] → A1's doPublish checks
      // gen(1) !== pendingGeneration(2) → exits stale without publishing.
      // startCycle() fires from finally → A2's cycle starts → schedules
      // a fresh 2000ms debounce for A2's pending edit.
      await act(async () => {
        assert.ok(
          releaseA1Preflight !== null,
          `${label}: A1 preflight must have been parked`,
        );
        releaseA1Preflight();
        for (let i = 0; i < 60; i++) await Promise.resolve();
      });

      // Fire A2's debounce timer so doPublish(A2) runs → fetchEvents #3 →
      // remoteAdvancedSince arbitration. Without this the timer sits non-null
      // and publishCalls===0 passes vacuously under both predicates.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // A2's cycle: pre-publish fetch returns H102 (fetchCalls=3).
      // Fix: baseline={0,""} → remoteAdvancedSince(H102,{0,""})=true → ADOPT.
      // Broken: baseline=H102 → equality → publishEvent called → test fails.
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.equal(
        publishCalls.length,
        0,
        `Carl-P2 ${label} [in-flight]: second click after debounce fired must not reseed baseline — ` +
          `wasIdle||!inDebounceWindow reseeds H102 as baseline → A2 publishes over H102`,
      );

      hook.unmount();
    } finally {
      cleanup();
      tauri.restore();
      relayClient.fetchEvents = origFetch;
      relayClient.subscribeLive = origSubscribeLive;
      relayClient.publishEvent = origPublish;
      relayClient.subscribeToReconnects = origReconnects;
      restoreTimers();
      window.localStorage.clear();
      mock.reset();
    }
  });
}
