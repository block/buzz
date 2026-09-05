// Shared parameterized test suite for whole-blob React hooks
// (useChannelSections.ts, useChannelSortPreference.ts).
//
// Covers four behavioral invariants common to both hooks:
//   1. storage event while pending is deferred (hasPendingEdit guard)
//   2. live remote while pending defers to the optimistic edit
//   3. equal-timestamp tie-break applies the lower event id
//   4. reconnect adopts an advanced remote, never publishing over it
//
// Lane-specific tests (assignChannel/quota) stay in the lane files.

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import { makeHookStubs } from "./sidebarSyncTestHelpers.mjs";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});
after(() => dom.window.close());

const { stubRelay, stubTauri } = makeHookStubs();

export function runWholeBlobHookSuite({
  label,
  dTag,
  useHook,
  storageKey,
  readOutbox,
  // legacyOutboxKey(pubkey, relayUrl): returns the legacy shared outbox key string
  legacyOutboxKey,
  // makeEdit(result): call one mutation action on the hook result
  makeEdit,
  // makeB1Store(): serialized JSON string for the peer storage event
  makeB1Store,
  // assertNotContainsB1(result): B1's distinguishing state must not be in result
  assertNotContainsB1,
  // makeA2Edit(result): call a second mutation so A2 derives from A1
  makeA2Edit,
  // assertA2Derived(result): A2's state should derive from A1, not B1
  assertA2Derived,
  // makeLiveRemotePayload(): decrypt return for the live remote event
  makeLiveRemotePayload,
  // assertRemoteNotApplied(result): live remote must not have clobbered pending
  assertRemoteNotApplied,
  // makeDecryptById(id): decrypt return keyed by event id (for tie-break)
  makeDecryptById,
  // assertLowerIdWon(result) / assertHigherIdLost(result): tie-break checks
  assertLowerIdWon,
  assertHigherIdLost,
  // makeRemotePayload(): decrypt return for reconnect test
  makeRemotePayload,
  // assertLocalAdoptedAway(result): losing local edit must be gone
  assertLocalAdoptedAway,
}) {
  // Mutation: removing hasPendingEdit() guard lets B1 apply, so A2 derives from B1.
  test(`${label}: storage event while a local edit is pending is deferred, not applied`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");
    const restoreRelay = stubRelay(relayClient);
    const restoreTauri = stubTauri(`pk-${label}-sg`, null);
    const pubkey = `pk-${label}-sg`;
    const relayUrl = `wss://r.${label}-sg`;
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        await Promise.resolve();
        await Promise.resolve();
      });
      await act(async () => {
        makeEdit(hook.result.current);
      });
      const b1Store = makeB1Store();
      await act(async () => {
        window.localStorage.setItem(storageKey(pubkey, relayUrl), b1Store);
        window.dispatchEvent(
          new window.StorageEvent("storage", {
            key: storageKey(pubkey, relayUrl),
            newValue: b1Store,
            storageArea: window.localStorage,
          }),
        );
        await Promise.resolve();
        await Promise.resolve();
      });
      assertNotContainsB1(
        hook.result.current,
        "storage event while pending must not apply B1",
      );
      await act(async () => {
        makeA2Edit(hook.result.current);
      });
      assertA2Derived(hook.result.current, "A2 must derive from A1, not B1");
      hook.unmount();
    } finally {
      cleanup();
      restoreRelay();
      restoreTauri();
    }
  });

  // Mutation: reverting applyRemote's hasPendingEdit() guard clobbers the UI.
  test(`${label}: live remote while a local edit is pending defers to the pending edit`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");
    const live = {};
    const restoreRelay = stubRelay(relayClient, { live });
    const restoreTauri = stubTauri(`pk-${label}-lp`, () =>
      makeLiveRemotePayload(),
    );
    const pubkey = `pk-${label}-lp`;
    const relayUrl = `wss://r.${label}-lp`;
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 3; i++) await Promise.resolve();
      });
      assert.ok(live.cb, "live subscription installed");
      await act(async () => {
        makeEdit(hook.result.current);
      });
      assert.ok(readOutbox(pubkey, relayUrl), "local edit persisted to outbox");
      await act(async () => {
        live.cb({
          id: "remote-event",
          pubkey,
          created_at: 500,
          content: "cipher",
          kind: 30078,
          tags: [["d", dTag]],
          sig: "s",
        });
        await Promise.resolve();
      });
      assertRemoteNotApplied(
        hook.result.current,
        "pending local edit NOT overwritten by live remote",
      );
      assert.ok(readOutbox(pubkey, relayUrl), "outbox survives live remote");
      hook.unmount();
    } finally {
      cleanup();
      restoreRelay();
      restoreTauri();
    }
  });

  // Mutation: reverting applyRemote's >= back to <= converges on the larger id.
  test(`${label}: equal-timestamp tie-break applies the lower event id (relay canonical winner)`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");
    const live = {};
    const restoreRelay = stubRelay(relayClient, { live });
    const restoreTauri = stubTauri(`pk-${label}-tie`, (args) =>
      makeDecryptById(args?.ciphertext ?? ""),
    );
    const pubkey = `pk-${label}-tie`;
    const relayUrl = `wss://r.${label}-tie`;
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 3; i++) await Promise.resolve();
      });
      assert.ok(live.cb, "live subscription installed");
      const deliver = async (id) => {
        await act(async () => {
          live.cb({
            id,
            pubkey,
            created_at: 1000,
            content: id,
            kind: 30078,
            tags: [["d", dTag]],
            sig: "s",
          });
          await Promise.resolve();
        });
      };
      await deliver("bbbb");
      await deliver("aaaa");
      assertLowerIdWon(hook.result.current, "lower event id wins tie-break");
      assertHigherIdLost(hook.result.current, "larger id superseded");
      hook.unmount();
    } finally {
      cleanup();
      restoreRelay();
      restoreTauri();
    }
  });

  // Mutation: reverting reconnect handler to publishX(pending) resets the baseline.
  test(`${label}: reconnect adopts a remote that advanced while the edit was pending, never publishing over it`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");
    const reconnect = {};
    const publishCalls = [];
    let head = {
      pubkey: `pk-${label}-rc`,
      content: "remote-cipher",
      created_at: 100,
      id: "evt-100",
    };
    const origFetch = relayClient.fetchEvents;
    relayClient.fetchEvents = async () => [head];
    const restoreRelay = stubRelay(relayClient, { reconnect, publishCalls });
    relayClient.fetchEvents = async () => [head];
    const restoreTauri = stubTauri(`pk-${label}-rc`, () => makeRemotePayload());
    const pubkey = `pk-${label}-rc`;
    const relayUrl = `wss://r.${label}-rc`;
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 3; i++) await Promise.resolve();
      });
      assert.ok(reconnect.cb, "reconnect handler installed");
      await act(async () => {
        makeEdit(hook.result.current);
      });
      head = {
        pubkey: `pk-${label}-rc`,
        content: "remote-cipher",
        created_at: 200,
        id: "evt-200",
      };
      await act(async () => {
        reconnect.cb();
        for (let i = 0; i < 4; i++) await Promise.resolve();
      });
      assert.equal(
        publishCalls.length,
        0,
        "must adopt advanced remote on reconnect, never publish",
      );
      assertLocalAdoptedAway(
        hook.result.current,
        "losing local edit adopted away",
      );
      hook.unmount();
    } finally {
      cleanup();
      relayClient.fetchEvents = origFetch;
      restoreRelay();
      restoreTauri();
    }
  });

  // Mutation: removing queuedAt >= appliedHead.createdAt guard in the hook
  // replays the stale outbox above the applied head, overwriting device-B's
  // state. All four sub-cases advance the real 2s debounce via makeHookTimerBed
  // so the assertion is causal: the unfixed code fires publishEvent after the
  // debounce while the fixed code does not (strict <) or does (hold / equality).
  test(`${label}: P1 bootstrap outbox replay gate — strict < suppresses, equality/hold/legacy replays`, async () => {
    const { makeHookTimerBed } = await import("./sidebarSyncTestHelpers.mjs");
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    // Helper: mount the hook with a legacy outbox record at queuedAt and an
    // optional relay head, fire the 2s debounce, return publishEvent call count.
    async function runCase({ queuedAt, headCreatedAt, noHead = false }) {
      const bed = makeHookTimerBed();
      const pubkey = `pk-${label}-p1-${queuedAt}-${headCreatedAt}`;
      const relayUrl = `wss://r.${label}-p1`;
      // Write via the legacy key — recognised by the outbox reader for both
      // sections and sort hooks, and queuedAt is explicit in the envelope.
      const legKey = legacyOutboxKey(pubkey, relayUrl);
      window.localStorage.setItem(
        legKey,
        JSON.stringify({ store: JSON.parse(makeRemotePayload()), queuedAt }),
      );

      const publishCalls = [];
      const restoreRelay = stubRelay(relayClient, { publishCalls });
      const origFetch = relayClient.fetchEvents;
      relayClient.fetchEvents = async () => {
        if (noHead) return [];
        return [
          {
            pubkey,
            content: "good-cipher",
            created_at: headCreatedAt,
            id: `evt-${headCreatedAt}`,
          },
        ];
      };
      const restoreTauri = stubTauri(pubkey, () => makeRemotePayload());
      let hook = null;
      try {
        await act(async () => {
          hook = renderHook(() => useHook(pubkey, relayUrl));
          for (let i = 0; i < 8; i++) await Promise.resolve();
        });
        // Whether publish() was called is observable via the 2s debounce timer:
        // publish() schedules the timer synchronously. hasDelay(2000) is true
        // iff publish() fired (replay happened); false iff the guard suppressed.
        // This is causal: without the guard, the timer would be present for all
        // cases; with it, only the non-suppressed cases schedule the timer.
        return bed.hasDelay(2000) ? 1 : 0;
      } finally {
        hook?.unmount();
        cleanup();
        window.localStorage.removeItem(legKey);
        bed.restore();
        restoreRelay();
        restoreTauri();
        relayClient.fetchEvents = origFetch;
      }
    }

    // Case 1: strict queuedAt < headCreatedAt → suppressed (0 publish calls)
    {
      const calls = await runCase({ queuedAt: 100, headCreatedAt: 200 });
      assert.equal(
        calls,
        0,
        `${label}: stale outbox (queuedAt=100 < head=200) must NOT be replayed`,
      );
    }

    // Case 2: queuedAt === headCreatedAt → replays (≥1 publish call)
    {
      const calls = await runCase({ queuedAt: 200, headCreatedAt: 200 });
      assert.ok(
        calls >= 1,
        `${label}: same-second outbox (queuedAt=200 === head=200) MUST replay`,
      );
    }

    // Case 3: hold path (no relay head) with a v2 queuedAt > 0 → always replays
    {
      const calls = await runCase({
        queuedAt: 50,
        headCreatedAt: 0,
        noHead: true,
      });
      assert.ok(
        calls >= 1,
        `${label}: hold path (no relay head) MUST replay outbox regardless of queuedAt`,
      );
    }

    // Case 4: hold path with legacy queuedAt=0 → always replays (was silently
    //         stranded by the old guard: 0 > 0 was false on hold)
    {
      const calls = await runCase({
        queuedAt: 0,
        headCreatedAt: 0,
        noHead: true,
      });
      assert.ok(
        calls >= 1,
        `${label}: hold path with legacy queuedAt=0 MUST replay (was stranded by old 0>0 guard)`,
      );
    }
  });
}
