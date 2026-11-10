/**
 * Boundary tests for useUnreadChannels — exercises the real parent-to-owner
 * boundary between useUnreadChannels and useObservedUnreadPersistence.
 *
 * These tests mount the FULL production hook (via createRoot + act) to verify
 * that markChannelRead and markAllChannelsRead are scope-safe: a stale callback
 * captured under scope A cannot corrupt scope B's refs or storage after a
 * scope switch. Coverage for both happy paths (current scope mutates correctly)
 * and stale paths (scope-A callback rejects after B loads) is included.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  installDOMShim,
  installFreshStorage,
  seedStorage,
  mountUnreadChannels,
} from "./observedUnreadTestHarness.mjs";

// DOM shim must run before any React import (harness imports React at parse time).
installDOMShim();
installFreshStorage();

import { readObservedUnreadFromStorage } from "./observedUnreadStorage.ts";
import { act } from "react";

// ── Helpers ───────────────────────────────────────────────────────────────────

const RELAY = "wss://relay.example.com";

// ── Tests ─────────────────────────────────────────────────────────────────────

test("markChannelRead happy path: current scope removes channel from observed refs and persists snapshot", async () => {
  installFreshStorage();

  const PUBKEY = "pubkey-happy-mcr";
  const readAt = seedStorage(PUBKEY, RELAY, "channel-1");

  const harness = await mountUnreadChannels({ pubkey: PUBKEY });

  // markChannelRead with readAt >= observed latest triggers clearObserved,
  // which delegates to observedPersistence.removeChannel. act() needed because
  // markChannelRead calls bumpLatestVersion (useReducer).
  await act(async () => {
    harness.markChannelRead("channel-1", readAt);
  });
  harness.flushStorage();

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(
    stored === null || !stored.has("channel-1"),
    "channel-1 must be absent from storage after markChannelRead under current scope",
  );

  await harness.unmount();
});

test("markChannelRead happy path: topLevelOnly=true leaves observed refs intact (no clearObserved)", async () => {
  installFreshStorage();

  const PUBKEY = "pubkey-happy-mcr-tlo";
  const readAt = seedStorage(PUBKEY, RELAY, "channel-tlo");

  const harness = await mountUnreadChannels({ pubkey: PUBKEY });

  // topLevelOnly=true passes observedLatest as undefined, so clearObserved stays false.
  await act(async () => {
    harness.markChannelRead("channel-tlo", readAt, { topLevelOnly: true });
  });
  harness.flushStorage();

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(
    stored?.has("channel-tlo"),
    "channel-tlo must remain in storage when topLevelOnly=true",
  );

  await harness.unmount();
});

test("markChannelRead stale: scope-A callback rejects after scope B loads — B storage survives flush", async () => {
  installFreshStorage();

  const PUBKEY_A = "pubkey-a-mcr";
  const PUBKEY_B = "pubkey-b-mcr";
  // Shared channel ensures stale A callback targets a channel present in B's hydrated refs.
  const SHARED_CHANNEL = "channel-shared";

  const readAtA = seedStorage(PUBKEY_A, RELAY, SHARED_CHANNEL, "evt-a");
  seedStorage(PUBKEY_B, RELAY, SHARED_CHANNEL, "evt-b");

  const harness = await mountUnreadChannels({ pubkey: PUBKEY_A });
  const staleMarkChannelRead = harness.markChannelRead;

  // Switch to B; hydration flushes A and loads B's storage.
  await harness.render(PUBKEY_B);

  assert.ok(
    readObservedUnreadFromStorage(PUBKEY_B, RELAY)?.has(SHARED_CHANNEL),
    "B's channel-shared must be present before the stale call",
  );

  // Stale A call must be rejected by the scope fence; B's refs stay intact.
  await act(async () => {
    staleMarkChannelRead(SHARED_CHANNEL, readAtA);
  });
  harness.flushStorage();

  const storedBAfter = readObservedUnreadFromStorage(PUBKEY_B, RELAY);
  assert.ok(
    storedBAfter?.has(SHARED_CHANNEL),
    "B's channel-shared must survive the post-stale-call flush (stale scope-A markChannelRead must not corrupt B's refs)",
  );

  await harness.unmount();
});

test("markAllChannelsRead happy path: current scope clears all observed refs and clears storage bucket", async () => {
  installFreshStorage();

  const PUBKEY = "pubkey-happy-mar";
  seedStorage(PUBKEY, RELAY, "channel-1");

  const harness = await mountUnreadChannels({ pubkey: PUBKEY });

  // clearAll cancels any pending write and removes the storage bucket.
  await act(async () => {
    harness.markAllChannelsRead();
  });

  const stored = readObservedUnreadFromStorage(PUBKEY, RELAY);
  assert.ok(
    stored === null || stored.size === 0,
    "storage bucket must be empty after markAllChannelsRead under current scope",
  );

  await harness.unmount();
});

test("markAllChannelsRead stale: scope-A callback rejects after scope B loads — B storage survives flush", async () => {
  installFreshStorage();

  const PUBKEY_A = "pubkey-a-mar";
  const PUBKEY_B = "pubkey-b-mar";

  seedStorage(PUBKEY_A, RELAY, "channel-1");
  seedStorage(PUBKEY_B, RELAY, "channel-2");

  const harness = await mountUnreadChannels({ pubkey: PUBKEY_A });
  const staleMarkAllChannelsRead = harness.markAllChannelsRead;

  // Switch to B; hydration flushes A and loads B's storage (channel-2).
  await harness.render(PUBKEY_B);

  assert.ok(
    readObservedUnreadFromStorage(PUBKEY_B, RELAY)?.has("channel-2"),
    "B's channel-2 must be present before the stale call",
  );

  // Stale A call must be rejected by the scope fence; B's refs stay intact.
  await act(async () => {
    staleMarkAllChannelsRead();
  });
  harness.flushStorage();

  const storedBAfter = readObservedUnreadFromStorage(PUBKEY_B, RELAY);
  assert.ok(
    storedBAfter?.has("channel-2"),
    "B's channel-2 must survive the post-stale-call flush (stale scope-A markAllChannelsRead must not wipe B's refs)",
  );

  await harness.unmount();
});
