// Shared parameterized test suite for merge-lane React hooks (useChannelStars.ts, useChannelMutes.ts).
// Usage: import { runMergeLaneHookSuite } from "./mergeLaneHook.shared.test.mjs";
//   runMergeLaneHookSuite({ label: "stars", ... });

import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";
import {
  installEchoTauri,
  makeHookStubs,
  makeHookTimerBed,
} from "./sidebarSyncTestHelpers.mjs";

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

const { stubRelay } = makeHookStubs();

export function runMergeLaneHookSuite({
  label,
  entryValueField,
  idsField,
  trueAction,
  falseAction,
  dTag,
  outboxKeyPrefix,
  MAX_ENTRIES,
  readStore,
  storageKey,
  useHook,
  makePayload,
}) {
  const trueLabel = trueAction.replace("Channel", "").toLowerCase();
  const falseLabel = falseAction.replace("Channel", "").toLowerCase();

  test(`${label}: same-second ${trueLabel} and ${falseLabel} mutations survive at capacity`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    const restore = stubRelay(relayClient);
    const originalDateNow = Date.now;
    const updatedAt = 1_234_567;
    Date.now = () => updatedAt * 1_000;

    const relayUrl = "wss://relay.example";
    const channels = Object.fromEntries(
      Array.from({ length: MAX_ENTRIES }, (_, index) => [
        `z-channel-${String(index).padStart(3, "0")}`,
        { [entryValueField]: true, updatedAt, rev: 0 },
      ]),
    );

    try {
      for (const [pubkey, action, expectedValue] of [
        [`pk-${trueLabel}`, trueAction, true],
        [`pk-${falseLabel}`, falseAction, false],
      ]) {
        window.localStorage.setItem(
          storageKey(pubkey, relayUrl),
          JSON.stringify({ version: 1, channels }),
        );
        const { result, unmount } = renderHook(() => useHook(pubkey, relayUrl));
        act(() => result.current[action]("a-target"));
        const persisted = readStore(pubkey, relayUrl);
        assert.equal(Object.keys(persisted.channels).length, MAX_ENTRIES);
        assert.equal(
          persisted.channels["a-target"][entryValueField],
          expectedValue,
        );
        unmount();
      }
    } finally {
      cleanup();
      Date.now = originalDateNow;
      restore();
    }
  });

  // Monotonic mint: far-future live event + persisted high-water + same-second clicks.
  // Mutation: dropping maxUpdatedAtSeen/maxRevSeen makes later clicks mint below observed high-water.
  test(`${label}: monotonic mint — persisted high-water + far-future live event + same-second clicks advance rev`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    const live = {};
    const restore = stubRelay(relayClient, { live });
    const origTauri = window.__TAURI_INTERNALS__;
    const origDateNow = Date.now;
    Date.now = () => 100 * 1_000;
    const FUTURE = 500;
    window.__TAURI_INTERNALS__ = {
      invoke: (cmd) => {
        if (cmd === "nip44_decrypt_from_self")
          return Promise.resolve(
            makePayload({
              shared: { [entryValueField]: false, updatedAt: FUTURE, rev: 7 },
            }),
          );
        return Promise.reject(new Error(`unmocked ${cmd}`));
      },
    };
    const pubkey = `pk-${label}-mono`;
    window.localStorage.setItem(
      storageKey(pubkey, "wss://r"),
      makePayload({
        shared: { [entryValueField]: true, updatedAt: FUTURE, rev: 4 },
      }),
    );
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, "wss://r"));
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });
      await act(async () => {
        live.cb({
          id: "future-head",
          pubkey,
          created_at: FUTURE,
          content: "cipher",
          kind: 30078,
          tags: [["d", dTag]],
          sig: "s",
        });
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });
      await act(async () => hook.result.current[trueAction]("shared"));
      let p = readStore(pubkey, "wss://r");
      assert.equal(
        p.channels.shared[entryValueField],
        true,
        "first click applied",
      );
      assert.equal(
        p.channels.shared.updatedAt,
        FUTURE,
        "updatedAt held at observed high-water",
      );
      assert.equal(p.channels.shared.rev, 8, "rev = maxRevSeen(7) + 1");
      await act(async () => hook.result.current[falseAction]("shared"));
      p = readStore(pubkey, "wss://r");
      assert.equal(
        p.channels.shared[entryValueField],
        false,
        "second click applied",
      );
      assert.equal(
        p.channels.shared.updatedAt,
        FUTURE,
        "updatedAt still fixed",
      );
      assert.equal(p.channels.shared.rev, 9, "rev advanced 8→9");
      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      window.__TAURI_INTERNALS__ = origTauri;
      restore();
    }
  });

  // Bootstrap replay seam: bootstrap calls publishStars/publishMutes(outbox), which drives
  // a full timer→publish→confirm cycle for the resumed edit before any new click.
  // Mutations: (a) removing storageEvent listener; (b) skipping bootstrap publish(outbox);
  // (c) clearing on any ACK.
  test(`${label}: bootstrap replay — resumed edit drives timer/publish/confirm; non-subsuming retains key; subsuming clears`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    let fetchResult = [];
    const restore = stubRelay(relayClient, {});
    relayClient.fetchEvents = async () => fetchResult;

    const { timers, fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 100 * 1_000;
    const pubkey = `pk-${label}-xwin`;
    const relayUrl = "wss://r";
    const encodedRelay = encodeURIComponent(relayUrl);

    window.localStorage.setItem(
      `${outboxKeyPrefix}:${pubkey}:${encodedRelay}`,
      makePayload({
        resumed: { [entryValueField]: true, updatedAt: 90, rev: 2 },
      }),
    );

    const tauri = installEchoTauri(pubkey);
    const nsHead = tauri.mintHead(
      {
        version: 1,
        channels: { other: { [entryValueField]: false, updatedAt: 1, rev: 0 } },
      },
      50,
      "evt-ns",
    );
    nsHead.tags = [["d", dTag]];
    const subHead = tauri.mintHead(
      {
        version: 1,
        channels: {
          resumed: { [entryValueField]: true, updatedAt: 90, rev: 2 },
        },
      },
      60,
      "evt-sub",
    );
    subHead.tags = [["d", dTag]];

    const v2Prefix = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:`;
    const v2Keys = () =>
      Array.from({ length: window.localStorage.length }, (_, i) =>
        window.localStorage.key(i),
      ).filter((k) => k?.startsWith(v2Prefix) && k.split(":").length >= 5);

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });
      assert.ok(
        v2Keys().length > 0,
        "bootstrap must write v2 key — skipping publish(outbox) breaks this",
      );

      // Debounce fires (fetchResult=[]): publish resumed → empty confirm → retry.
      // retryDelayMs starts at 2000ms and doubles each failed confirm.
      await fireDelay(2000);
      assert.ok(v2Keys().length > 0, "v2 key survives empty confirm");
      assert.ok(
        [...timers.values()].some((t) => t.ms === 2000),
        "retry scheduled after empty confirm — scheduleRetry not called breaks this",
      );

      // Non-subsuming retry (delay=2000ms): other-only head does not subsume resumed → retry.
      // retryDelayMs now doubles to 4000ms for the next cycle.
      fetchResult = [nsHead];
      await fireDelay(2000);
      assert.ok(
        v2Keys().length > 0,
        "v2 key survives non-subsuming confirm — clearing on any ACK breaks this",
      );
      assert.ok(
        [...timers.values()].some((t) => t.ms === 4000),
        "retry rescheduled (4000ms) after non-subsuming confirmation",
      );

      // Subsuming retry (delay=4000ms): head carries resumed → discardPending → key cleared.
      fetchResult = [subHead];
      await fireDelay(4000);
      assert.equal(
        v2Keys().length,
        0,
        "v2 key cleared on subsuming confirm — clearing on any ACK does not reach here",
      );

      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreTimers();
      restore();
    }
  });

  // Click-before-opposite-peer-arrival: local true-click on "shared" at t=200 s, then a
  // peer window writes shared=false at (updatedAt=900, rev=12) via StorageEvent. Peer's
  // NEWER tuple wins max-merge → stored=false/900/12, idsField evicts "shared". The real
  // 3 000 ms reconcile tick fires and fetches the authoritative relay head at the
  // observably DISTINCT tuple (updatedAt=950, rev=13) → stored entry advances to 950/13.
  //
  // Bootstrap returns empty — no relay head on mount — so click mints at updatedAt=200
  // and peer's 900 unambiguously wins max-merge.
  //
  // Mutations:
  //   (a) Drop storage listener → peer StorageEvent never runs max-merge; stored shared
  //       stays true/200/1 and idsField keeps "shared" — fails the arrival tuple assert.
  //   (b) Suppress fetchRemote* inside the scheduled tick → fetchCount stays at
  //       fetchBefore — fails "reconcile fetch fired".
  //   (c) Perform the fetch but drop/ignore the fetched result → stored entry stays at
  //       false/900/12 (arrival value), not the relay head's 950/13 — fails the final
  //       tuple assert.
  test(`${label}: click-before-opposite-peer-arrival — peer tuple evicts click; reconcile applies distinct relay head`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");
    let fetchCount = 0;
    let lastFetchFilter = null;
    const restore = stubRelay(relayClient, {});
    const tauri = installEchoTauri(`pk-${label}-cpeer`);
    const pubkey = `pk-${label}-cpeer`;
    const relayUrl = "wss://r";
    // Bootstrap returns empty; reconcile head is set only after the storage event.
    let fetchResult = [];
    relayClient.fetchEvents = async (f) => {
      fetchCount++;
      lastFetchFilter = f;
      return fetchResult;
    };
    // Relay authoritative head: shared=false at the DISTINCT tuple (updatedAt=950, rev=13).
    // This is observably different from the peer's arrival (900, 12) so reconcile
    // application can be proven: if applyRemote is a no-op the final stored entry
    // stays at 900/12 instead of advancing to 950/13.
    const authoritativeStore = {
      version: 1,
      channels: {
        shared: { [entryValueField]: false, updatedAt: 950, rev: 13 },
      },
    };
    const relayHead = tauri.mintHead(authoritativeStore, 951, "evt-auth");
    relayHead.tags = [["d", dTag]];
    relayHead.pubkey = pubkey;
    relayHead.kind = 30078;
    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 200 * 1_000;
    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });
      // Local true-click first: no relay head seen → mints updatedAt=200, rev=1.
      await act(async () => hook.result.current[trueAction]("shared"));
      assert.ok(
        hook.result.current[idsField].has("shared"),
        "local click applied immediately",
      );
      {
        const p = readStore(pubkey, relayUrl);
        assert.equal(
          p.channels.shared[entryValueField],
          true,
          "stored shared=true after local click",
        );
        assert.equal(
          p.channels.shared.updatedAt,
          200,
          "click minted at updatedAt=200",
        );
      }
      // Peer writes shared=false at (updatedAt=900, rev=12) — NEWER than click (200).
      // Max-merge after the storage event selects the peer's tuple as the winner.
      // Mutation (a): drop listener → this StorageEvent is never processed; stored
      // shared stays true/200/1; arrival tuple assert below fails.
      window.localStorage.setItem(
        storageKey(pubkey, relayUrl),
        makePayload({
          shared: { [entryValueField]: false, updatedAt: 900, rev: 12 },
        }),
      );
      await act(async () => {
        window.dispatchEvent(
          new dom.window.StorageEvent("storage", {
            key: storageKey(pubkey, relayUrl),
          }),
        );
        for (let i = 0; i < 30; i++) await Promise.resolve();
      });
      // Peer's tuple wins: stored entry is exactly false/900/12.
      {
        const p = readStore(pubkey, relayUrl);
        assert.equal(
          p.channels.shared[entryValueField],
          false,
          "arrival: shared=false — mutation (a) leaves true",
        );
        assert.equal(
          p.channels.shared.updatedAt,
          900,
          "arrival: updatedAt=900 — mutation (a) leaves 200",
        );
        assert.equal(
          p.channels.shared.rev,
          12,
          "arrival: rev=12 — mutation (a) leaves 1",
        );
      }
      assert.ok(
        !hook.result.current[idsField].has("shared"),
        "peer false evicts local click from idsField — mutation (a) leaves it present",
      );
      // Fire the 3 000 ms reconcile timer; the relay returns the DISTINCT authoritative
      // head at (950, 13). After applyRemote the stored entry must advance to 950/13.
      fetchResult = [relayHead];
      const fetchBefore = fetchCount;
      await act(async () => {
        await fireDelay(3000);
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });
      // Mutation (b): suppress fetchRemote* inside tick → fetchCount unchanged.
      assert.ok(
        fetchCount > fetchBefore,
        "reconcile fetch fired — mutation (b) suppressing fetchRemote* breaks this",
      );
      assert.deepEqual(
        lastFetchFilter,
        {
          kinds: [30078],
          authors: [pubkey],
          "#d": [dTag],
          limit: 1,
        },
        "reconcile fetch filter exact shape",
      );
      // Mutation (c): fetch fires but result is dropped → stored entry stays at 900/12.
      {
        const p = readStore(pubkey, relayUrl);
        assert.equal(
          p.channels.shared[entryValueField],
          false,
          "after reconcile: shared=false",
        );
        assert.equal(
          p.channels.shared.updatedAt,
          950,
          "after reconcile: updatedAt advanced to 950 — mutation (c) leaves 900",
        );
        assert.equal(
          p.channels.shared.rev,
          13,
          "after reconcile: rev advanced to 13 — mutation (c) leaves 12",
        );
      }
      assert.ok(
        !hook.result.current[idsField].has("shared"),
        "reconcile: shared stays absent from idsField",
      );
      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreTimers();
      restore();
    }
  });

  // P3 reconnect: hook's registered subscribeToReconnects callback must use
  // retryReconnect*Publish(), NOT publish*(pending), so pendingPreservedKey is
  // not reset. With 500 local entries and 1 fresh remote entry, merged = 501.
  // Without the fix, publish*(pending) resets pendingPreservedKey → the clicked
  // channel is evicted. Reverting the hook's reconnect effect to publish*(pending)
  // fails this test while leaving the manager-level tests green.
  //
  // Failing mutation: revert hook reconnect from retryReconnect*Publish() to
  // publish*(outbox or pending).
  test(`${label}: P3 reconnect — registered reconnect callback preserves clicked channel through 501-entry merge`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    const MAX = MAX_ENTRIES;
    const clickedId = `ch-rc-click-${label}`;
    const relayUrl = "wss://relay-p3-rc";

    // Local: clicked channel (oldest) + 499 others at updatedAt=100
    const localChannels = {
      [clickedId]: { [entryValueField]: true, updatedAt: 1, rev: 1 },
    };
    for (let i = 0; i < MAX - 1; i++) {
      localChannels[`ch-rc-${i}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }
    // Remote: same 499 + one fresh channel → merged = 501
    const remoteChannels = {};
    for (let i = 0; i < MAX - 1; i++) {
      remoteChannels[`ch-rc-${i}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }
    remoteChannels[`ch-rc-new-${label}`] = {
      [entryValueField]: false,
      updatedAt: 100,
      rev: 0,
    };

    const pubkey = `pk-p3-rc-${label}`;
    const reconnect = {};
    const restore = stubRelay(relayClient, { reconnect });
    const tauri = installEchoTauri(pubkey);
    const remoteHead = tauri.mintHead(
      { version: 1, channels: remoteChannels },
      50,
      `evt-rc-${label}`,
    );
    remoteHead.tags = [["d", dTag]];
    remoteHead.pubkey = pubkey;
    remoteHead.kind = 30078;

    const publishCalls = [];
    // Always return remoteHead so both the reconnect-callback fetch
    // (fetchRemote*) AND the subsequent pre-publish fetch inside doPublish()
    // see the 500-entry remote — together with the 500-entry pending store
    // that is 501 entries, triggering the capacity-bound merge that evicts
    // the clicked channel when pendingPreservedKey is reset.
    // Without the fix (hook reverts to publish*(pending)), pendingPreservedKey
    // is cleared before doPublish runs, so the 501-entry merge evicts the click.
    relayClient.fetchEvents = async () => [remoteHead];
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    // Use timer bed so the 2s click debounce does not fire unexpectedly.
    // retryReconnectPublish cancels the debounce and calls startCycle directly,
    // so no timer needs to be fired after the reconnect — just drain microtasks.
    const { restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 100 * 1_000;

    // Seed local store so the hook mounts with 500 entries
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({ version: 1, channels: localChannels }),
    );

    let hook = null;
    try {
      // Mount and let bootstrap settle (bootstrap fetch applies the remote head)
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 20; i++) await Promise.resolve();
      });

      // Click the channel — registers it in pendingPreservedKey, schedules 2s debounce
      await act(async () => hook.result.current[trueAction](clickedId));

      // Trigger the registered reconnect callback (exercises the real production seam).
      // The callback: fetchRemote*() → retryReconnect*Publish() → startCycle() → doPublish().
      // retryReconnectPublish cancels the debounce and drives the cycle directly.
      // doPublish then calls fetchRemote* again for the pre-publish merge, getting
      // remoteHead (500 entries) → 501-entry mergeWithRemote → evicts click without key.
      assert.ok(
        typeof reconnect.cb === "function",
        "subscribeToReconnects must register a callback",
      );
      publishCalls.length = 0;
      await act(async () => {
        // Drive the reconnect callback's async chain: fetchRemote → retryReconnect.
        const p = reconnect.cb();
        // Drain microtasks to let fetchRemote resolve and retryReconnectPublish run.
        for (let i = 0; i < 100; i++) await Promise.resolve();
        await p;
        // Drain doPublish's internal async chain: fetchOwn → publishEvent → confirm.
        for (let i = 0; i < 100; i++) await Promise.resolve();
      });

      assert.ok(
        publishCalls.length > 0,
        "publish must have fired after reconnect",
      );
      const plaintext = tauri.capturedPlaintext();
      assert.ok(plaintext !== null, "encrypt must have been called");
      const published = JSON.parse(plaintext);
      assert.ok(
        clickedId in published.channels,
        `clicked channel "${clickedId}" must survive 501-entry merge after reconnect — ` +
          `reverting hook reconnect from retryReconnect*Publish() to publish*(pending) ` +
          `resets pendingPreservedKey before doPublish runs and fails this`,
      );

      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreTimers();
      restore();
      window.localStorage.clear();
    }
  });

  // P3 restart: after a quit the prior window's outbox record is foreign (the
  // session nonce is gone). Bootstrap must recover the preservedKey from the
  // foreign record and forward it to publishStars/publishMutes.
  //
  // This test uses TWO foreign records so it exercises both the per-record fold
  // eviction (mutation c) and the isOwn-filter / bootstrap-forward mutations.
  // Without bestKey threaded through the readOutboxWithMeta reduce, folding
  // record 1 (500 entries) into record 2 (1 entry) totals 501 and evicts the
  // click before the final bound can protect it.
  //
  // Failing mutations:
  // (a) Revert to readOutboxPreservedKey (isOwn filter): returns undefined for
  //     the foreign record → publish with no key → clicked channel evicted.
  // (b) Drop bootstrap's preservedKey forward (publish(outbox, undefined)):
  //     same eviction even if key was recovered.
  // (c) Remove bestKey from per-record reduce in readOutboxWithMeta:
  //     fold evicts click at record-2 merge before final bound runs.
  test(`${label}: P3 restart — multi-record foreign-nonce outbox; preservedKey threads fold and bootstrap; clicked channel survives 501-entry merge`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");
    const { relayClient } = await import("@/shared/api/relayClient");

    const MAX = MAX_ENTRIES;
    const clickedId = `ch-restart-click-${label}`;
    const relayUrl = "wss://relay-p3-restart";
    const pubkey = `pk-p3-restart-${label}`;
    const encodedRelay = encodeURIComponent(relayUrl);

    // Record 1 (foreign, carries preservedKey):
    //   clicked channel (oldest updatedAt=1) + 499 base entries at updatedAt=100
    const record1Channels = {
      [clickedId]: { [entryValueField]: true, updatedAt: 1, rev: 1 },
    };
    for (let i = 0; i < MAX - 1; i++) {
      record1Channels[`ch-rs-${i}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }
    // Record 2 (foreign, no preservedKey): one additional entry.
    // Folding record1 (500) + record2 (1 new) = 501 without bestKey → click evicted.
    const record2Channels = {
      [`ch-rs-extra-${label}`]: {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      },
    };

    // Seed local store (matches record1 so bootstrap sees the clicked channel)
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({ version: 1, channels: record1Channels }),
    );

    // Seed TWO FOREIGN-NONCE outbox envelopes (models two prior window records after quit).
    // Neither nonce matches this window's outboxWindowNonce(); both are foreign.
    // Only record 1 carries preservedKey (max queuedAt → selected as bestKey).
    const foreignKey1 = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:foreign-nonce-xyz:0000`;
    window.localStorage.setItem(
      foreignKey1,
      JSON.stringify({
        store: { version: 1, channels: record1Channels },
        queuedAt: 1000,
        preservedKey: clickedId,
      }),
    );
    const foreignKey2 = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:foreign-nonce-abc:0001`;
    window.localStorage.setItem(
      foreignKey2,
      JSON.stringify({
        store: { version: 1, channels: record2Channels },
        queuedAt: 900,
        // No preservedKey — second window did not click
      }),
    );

    // Remote: same 499 base + one new → merged with resumed outbox = 501
    const remoteChannels = {};
    for (let i = 0; i < MAX - 1; i++) {
      remoteChannels[`ch-rs-${i}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }
    remoteChannels[`ch-rs-new-${label}`] = {
      [entryValueField]: false,
      updatedAt: 100,
      rev: 0,
    };

    const restore = stubRelay(relayClient);
    const tauri = installEchoTauri(pubkey);
    const remoteHead = tauri.mintHead(
      { version: 1, channels: remoteChannels },
      50,
      `evt-rs-${label}`,
    );
    remoteHead.tags = [["d", dTag]];
    remoteHead.pubkey = pubkey;
    remoteHead.kind = 30078;

    const publishCalls = [];
    let fetchCalls = 0;
    // Bootstrap fetch returns hold (empty) so publishStars is always called
    // unconditionally — the subsumed guard only suppresses on apply-remote.
    // The per-publish mergeWithRemote fetch (fetchCalls === 2) returns the
    // 500-entry remote head, triggering the 501-entry merge that evicts the
    // clicked channel without preservedKey.
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      return fetchCalls === 2 ? [remoteHead] : [];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    // Use the timer bed so the 2s debounce is captured; fire it to trigger
    // the bootstrap publish cycle.
    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 100 * 1_000;

    let hook = null;
    try {
      // Mount and drive bootstrap (outbox replay calls publish, schedules debounce)
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      // Fire the 2s debounce to trigger the publish cycle
      await fireDelay(2000);
      // Drain doPublish's internal chain (fetchOwn → publishEvent → confirm)
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.ok(
        publishCalls.length > 0,
        "bootstrap must trigger a publish for the resumed outbox",
      );
      const plaintext = tauri.capturedPlaintext();
      assert.ok(plaintext !== null, "encrypt must have been called");
      const published = JSON.parse(plaintext);
      assert.ok(
        clickedId in published.channels,
        `clicked channel "${clickedId}" must survive 501-entry merge after restart — ` +
          `readOutboxPreservedKey with isOwn filter returns undefined for foreign record and fails this; ` +
          `also fails when bestKey is dropped from per-record fold in readOutboxWithMeta`,
      );

      hook.unmount();
    } finally {
      cleanup();
      Date.now = origDateNow;
      tauri.restore();
      restoreTimers();
      restore();
      window.localStorage.clear();
    }
  });
}
