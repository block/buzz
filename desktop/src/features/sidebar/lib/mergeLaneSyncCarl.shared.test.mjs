// Carl-round P3 regression suite for merge-lane hooks (useChannelStars, useChannelMutes).
//
// Carl P3: isStarsStoreSubsumedBy (and mutes twin) calls mergeStores(head, candidate)
// WITHOUT preservedKey. At the 500-cap, mergeStores can evict the clicked entry X and
// return a store equal to head — "proving" subsumption even though X was never retained.
// Two consumers of this proof are fixed:
//
// P3-bootstrap: bootstrap suppression (useChannelStars.ts:126-128 / useChannelMutes.ts)
//   skips the publish when isStarsStoreSubsumedBy returns true. Without preservedKey,
//   a full-head bootstrap with 500 entries + candidate containing clicked X (501 total)
//   → mergeStores evicts X → returns head → "subsumed" → publish skipped → X lost.
//   Fix: pass outboxMeta.preservedKey to isStarsStoreSubsumedBy.
//   Mutation: drop the preservedKey argument from the isStarsStoreSubsumedBy call.
//
// P3-ack: confirmRetainedHeadSubsumes (mergeLaneSyncManager.ts:358) calls
//   config.isSubsumedBy(store, remote.store) without pendingPreservedKey.
//   Same eviction → returns true → discardPending clears outbox → X lost.
//   Fix: pass this.pendingPreservedKey to config.isSubsumedBy.
//   Mutation: drop pendingPreservedKey from the isSubsumedBy call.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import {
  makeHookTimerBed,
  makeHookStubs,
  installEchoTauri,
} from "./sidebarSyncTestHelpers.mjs";
import { relayClient } from "@/shared/api/relayClient";

const { stubRelay } = makeHookStubs();

/**
 * Register the P3 capacity-boundary subsumption regressions for a merge lane.
 *
 * @param {object} opts
 * @param {string} opts.label            — "stars"|"mutes"
 * @param {number} opts.MAX_ENTRIES      — capacity cap (500)
 * @param {string} opts.entryValueField  — "starred"|"muted"
 * @param {string} opts.trueAction       — "starChannel"|"muteChannel"
 * @param {string} opts.dTag             — "channel-stars"|"channel-mutes"
 * @param {string} opts.outboxKeyPrefix  — outbox localStorage prefix
 * @param {Function} opts.readStore      — (pubkey, relayUrl) => store
 * @param {Function} opts.storageKey     — (pubkey, relayUrl) => string
 * @param {Function} opts.useHook        — the hook under test
 * @param {Function} opts.makePayload    — (channels) => JSON string
 */
export function runMergeLaneCarlSuite({
  label,
  MAX_ENTRIES,
  entryValueField,
  trueAction,
  dTag,
  outboxKeyPrefix,
  readStore,
  storageKey,
  useHook,
  makePayload,
}) {
  const clickedId = `ch-p3-click-${label}`;
  const relayUrl = `wss://relay-p3-carl-${label}`;

  // P3-bootstrap: 500-entry relay head + candidate with clicked channel X
  // → isStarsStoreSubsumedBy without preservedKey evicts X → "subsumed=true"
  // → publish skipped → X lost.
  // Fix: pass outboxMeta.preservedKey → direct membership check protects X.
  // Mutation: drop preservedKey from isStarsStoreSubsumedBy call.
  test(`P3-bootstrap ${label}: 500-cap relay head must not subsume clicked channel X; bootstrap must publish to preserve X`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");

    const pubkey = `pk-p3-boot-${label}`;
    const encodedRelay = encodeURIComponent(relayUrl);

    // Relay head: 500 entries, all at updatedAt=100, rev=0.
    // None of them is clickedId.
    const headChannels = {};
    for (let i = 0; i < MAX_ENTRIES; i++) {
      headChannels[`ch-boot-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }

    // Outbox candidate: 499 of the head entries + clicked X (updatedAt=1, rev=1).
    // X is the oldest, so mergeStores(head, candidate) at cap=500 evicts X.
    const outboxChannels = {
      [clickedId]: {
        [entryValueField]: true,
        updatedAt: 1,
        rev: 1,
      },
    };
    for (let i = 0; i < MAX_ENTRIES - 1; i++) {
      outboxChannels[`ch-boot-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }

    // Seed local store (matches outbox channels so bootstrap sees clicked channel)
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({ version: 1, channels: outboxChannels }),
    );

    // Seed outbox: foreign nonce with preservedKey = clickedId.
    const foreignKey = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:foreign-p3-boot:0000`;
    window.localStorage.setItem(
      foreignKey,
      JSON.stringify({
        store: { version: 1, channels: outboxChannels },
        queuedAt: 1,
        preservedKey: clickedId,
      }),
    );

    const tauri = installEchoTauri(pubkey);
    const remoteHead = tauri.mintHead(
      { version: 1, channels: headChannels },
      50,
      `evt-p3-boot-${label}`,
    );
    remoteHead.tags = [["d", dTag]];
    remoteHead.pubkey = pubkey;
    remoteHead.kind = 30078;

    const publishCalls = [];
    let fetchCalls = 0;
    const restoreRelay = stubRelay(relayClient);
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      // Bootstrap: return 500-entry head.
      if (fetchCalls === 1) return [remoteHead];
      // Pre-publish merge fetch: return same head.
      if (fetchCalls === 2) return [remoteHead];
      // Confirm: return published event.
      return publishCalls.length > 0
        ? [publishCalls[publishCalls.length - 1]]
        : [remoteHead];
    };
    relayClient.publishEvent = async (evt) => {
      publishCalls.push(evt);
    };

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 100 * 1_000;

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      // Bootstrap returns apply-remote with 500-entry head.
      // outboxMeta.preservedKey = clickedId.
      // With the fix: isStarsStoreSubsumedBy(outbox, head, clickedId) →
      //   direct check: head.channels[clickedId] is absent → return false
      //   → subsumed=false → publishStars/publishMutes called.
      // With the mutation: isStarsStoreSubsumedBy(outbox, head) →
      //   mergeStores(head, outbox) at cap=500 evicts clickedId → equals head
      //   → subsumed=true → publish skipped → clickedId lost.

      // Fire the 2s debounce to trigger the publish cycle.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.ok(
        publishCalls.length > 0,
        `P3-bootstrap ${label}: publish must fire when clicked channel is absent from relay head — ` +
          `drop preservedKey from isSubsumedBy call → mergeStores evicts click → subsumed=true → publish skipped`,
      );
      const plaintext = tauri.capturedPlaintext();
      assert.ok(plaintext !== null, "encrypt must have been called");
      const published = JSON.parse(plaintext);
      assert.ok(
        clickedId in published.channels,
        `P3-bootstrap ${label}: clicked channel "${clickedId}" must survive 500-entry bootstrap subsumption check`,
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

  // P3-ack: confirmRetainedHeadSubsumes calls isSubsumedBy without
  // pendingPreservedKey. At the 500-cap, the same eviction certifies the click
  // was retained even though it was not. discardPending clears the outbox.
  // Fix: pass this.pendingPreservedKey to config.isSubsumedBy.
  // Mutation: drop pendingPreservedKey from the confirmRetainedHeadSubsumes call.
  test(`P3-ack ${label}: confirmRetainedHeadSubsumes must use pendingPreservedKey; must not clear outbox when relay head does not contain clicked channel`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");

    const pubkey = `pk-p3-ack-${label}`;
    const encodedRelay = encodeURIComponent(relayUrl);

    // Local: clicked channel (updatedAt=1, oldest) + 499 others.
    const localChannels = {
      [clickedId]: { [entryValueField]: true, updatedAt: 1, rev: 1 },
    };
    for (let i = 0; i < MAX_ENTRIES - 1; i++) {
      localChannels[`ch-ack-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }

    // Remote head returned by confirmRetainedHeadSubsumes: 500 entries that do
    // NOT include clickedId, but do include one new entry that was not in local.
    const remoteHeadChannels = {};
    for (let i = 0; i < MAX_ENTRIES - 1; i++) {
      remoteHeadChannels[`ch-ack-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }
    remoteHeadChannels[`ch-ack-new-${label}`] = {
      [entryValueField]: false,
      updatedAt: 100,
      rev: 0,
    };
    // mergeStores(remoteHead, pending_with_click, NO_KEY) → 501 entries →
    // evict clickedId (oldest) → equals remoteHead → isSubsumedBy returns true
    // → discardPending clears outbox → click lost.
    // Fix: isSubsumedBy(pending, remoteHead, clickedId) →
    //   headEntry = remoteHead.channels[clickedId] = undefined → return false
    //   → scheduleRetry.

    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({ version: 1, channels: localChannels }),
    );

    const tauri = installEchoTauri(pubkey);
    const confirmHead = tauri.mintHead(
      { version: 1, channels: remoteHeadChannels },
      50,
      `evt-p3-ack-${label}`,
    );
    confirmHead.tags = [["d", dTag]];
    confirmHead.pubkey = pubkey;
    confirmHead.kind = 30078;

    let fetchCalls = 0;
    const restoreRelay = stubRelay(relayClient);
    relayClient.fetchEvents = async () => {
      fetchCalls++;
      // Bootstrap: absent (so publish is always triggered, no subsumed check).
      if (fetchCalls === 1) return [];
      // Pre-publish merge: also absent or same local.
      if (fetchCalls === 2) return [];
      // confirmRetainedHeadSubsumes: return remote head without clickedId.
      return [confirmHead];
    };
    relayClient.publishEvent = async (evt) => {};

    const { fireDelay, restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    // Clock at t=1s so the click produces updatedAt=1 — the oldest entry among
    // all 500+1 channels (others are at updatedAt=100). This ensures the clicked
    // channel is the one evicted by the capacity-bounded merge at 501 entries,
    // making the pendingPreservedKey-drop mutation genuinely causal.
    Date.now = () => 1 * 1_000;

    let hook = null;
    const v2Prefix = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:`;
    const v2Keys = () =>
      Array.from({ length: window.localStorage.length }, (_, i) =>
        window.localStorage.key(i),
      ).filter((k) => k?.startsWith(v2Prefix) && k.split(":").length >= 5);

    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });

      // Click: registers pendingPreservedKey = clickedId.
      await act(async () => hook.result.current[trueAction](clickedId));

      // Fire 2s debounce: fetchOwn → publishEvent → confirmRetainedHeadSubsumes.
      // confirmRetainedHeadSubsumes returns confirmHead (no clickedId).
      // With the fix: isSubsumedBy(published, confirmHead, clickedId) →
      //   headEntry=undefined → false → scheduleRetry (outbox KEPT).
      // With the mutation: isSubsumedBy(published, confirmHead) →
      //   mergeStores evicts clickedId → true → discardPending → outbox CLEARED.
      await fireDelay(2000);
      for (let i = 0; i < 100; i++) await Promise.resolve();

      assert.ok(
        v2Keys().length > 0,
        `P3-ack ${label}: outbox must be retained when relay head does not contain clicked channel — ` +
          `drop pendingPreservedKey from confirmRetainedHeadSubsumes → isSubsumedBy evicts click → discardPending clears outbox`,
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

  // P3-reclaim: reclaimSubsumedStarsOutbox (or mutes twin) calls
  // reclaimSubsumedOutbox → isSubsumedBy(record.store, head). Before the fix
  // record.preservedKey was discarded, so a foreign outbox with a clicked
  // channel absent from a 500-entry head was deleted as "subsumed."
  // Fix: reclaimSubsumedOutbox passes record.preservedKey to isSubsumedBy.
  // Mutation: isSubsumedBy(record.store, head) without preservedKey
  //   → 501-entry merge evicts clickedId → subsumed=true → record deleted.
  test(`P3-reclaim ${label}: reclaimSubsumedOutbox must pass preservedKey; foreign outbox with clicked channel absent from 500-entry head must not be deleted`, async () => {
    const { act, cleanup, renderHook } = await import("@testing-library/react");

    const pubkey = `pk-p3-reclaim-${label}`;
    const encodedRelay = encodeURIComponent(relayUrl);

    // Relay head: 500 entries at updatedAt=100, none includes clickedId.
    const headChannels = {};
    for (let i = 0; i < MAX_ENTRIES; i++) {
      headChannels[`ch-reclaim-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }

    // Foreign outbox (another window): 499 of the head entries + clickedId
    // (updatedAt=1, oldest). preservedKey = clickedId.
    const foreignChannels = {
      [clickedId]: {
        [entryValueField]: true,
        updatedAt: 1,
        rev: 1,
      },
    };
    for (let i = 0; i < MAX_ENTRIES - 1; i++) {
      foreignChannels[`ch-reclaim-${String(i).padStart(4, "0")}`] = {
        [entryValueField]: false,
        updatedAt: 100,
        rev: 0,
      };
    }

    // Seed local store (no pending edit on this window).
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({ version: 1, channels: headChannels }),
    );

    // Seed a FOREIGN outbox key (different nonce so this window doesn't own it).
    const foreignKey = `${outboxKeyPrefix}:${pubkey}:${encodedRelay}:foreign-p3-reclaim:0001`;
    window.localStorage.setItem(
      foreignKey,
      JSON.stringify({
        store: { version: 1, channels: foreignChannels },
        queuedAt: 1,
        preservedKey: clickedId,
      }),
    );

    const tauri = installEchoTauri(pubkey);
    const relayHead = tauri.mintHead(
      { version: 1, channels: headChannels },
      50,
      `evt-p3-reclaim-${label}`,
    );
    relayHead.tags = [["d", dTag]];
    relayHead.pubkey = pubkey;
    relayHead.kind = 30078;

    const restoreRelay = stubRelay(relayClient);
    relayClient.fetchEvents = async () => [relayHead];
    relayClient.publishEvent = async () => {};

    const { restore: restoreTimers } = makeHookTimerBed();
    const origDateNow = Date.now;
    Date.now = () => 50 * 1_000;

    let hook = null;
    try {
      await act(async () => {
        hook = renderHook(() => useHook(pubkey, relayUrl));
        for (let i = 0; i < 40; i++) await Promise.resolve();
      });
      // Let bootstrap and reclamation settle.
      for (let i = 0; i < 100; i++) await Promise.resolve();

      // With the fix: reclaimSubsumedOutbox calls isSubsumedBy(foreignStore, head, clickedId)
      //   → direct check: head.channels[clickedId] = undefined → not subsumed → KEPT.
      // Mutation: isSubsumedBy(foreignStore, head) without preservedKey
      //   → 501-entry merge evicts clickedId → subsumed=true → foreignKey deleted.
      const stillPresent = window.localStorage.getItem(foreignKey) !== null;
      assert.ok(
        stillPresent,
        `P3-reclaim ${label}: foreign outbox with clicked channel absent from 500-entry ` +
          `relay head must not be reclaimed — drop preservedKey from reclaimSubsumedOutbox ` +
          `→ 501-entry merge evicts click → subsumed=true → record deleted`,
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
