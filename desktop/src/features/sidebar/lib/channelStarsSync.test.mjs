// Compact wire-contract adapter for ChannelStarSyncManager.
// Shared engine invariants are in mergeLaneSync.shared.test.mjs.
// This file asserts only stars-specific wiring: event kind, d-tag, payload shape, parser delegation, and typed API.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  parseStarPayload,
  readChannelStarsOutboxWithMeta,
} from "./channelStarsStorage.ts";
import { ChannelStarSyncManager } from "./channelStarsSync.ts";
import {
  installEchoTauri,
  installFakeWindow,
  makeFakeWindow,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.test";

test("stars wire: kind=30078, d-tag='channel-stars', payload has 'channels' not 'sections'", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  let publishedEvent = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    publishedEvent = evt;
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-wire-stars");
  try {
    const m = new ChannelStarSyncManager("pk-wire-stars", RELAY);
    m.publishStars({
      version: 1,
      channels: { ch1: { starred: true, updatedAt: 1, rev: 0 } },
    });
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(publishedEvent !== null, "publish must have been called");
    assert.equal(publishedEvent.kind, 30078, "kind must be 30078");
    const dTag = publishedEvent.tags.find((t) => t[0] === "d")?.[1];
    assert.equal(dTag, "channel-stars", "d-tag must be 'channel-stars'");
    const plaintext = tauri.capturedPlaintext();
    const parsed = JSON.parse(plaintext);
    assert.ok("channels" in parsed, "payload must have 'channels' field");
    assert.ok(!("sections" in parsed), "payload must not have 'sections'");
    assert.ok(!("groups" in parsed), "payload must not have 'groups'");
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("stars wire: parser delegates to parseStarPayload (starred field, version guard)", () => {
  const valid = {
    version: 1,
    channels: { a: { starred: true, updatedAt: 1, rev: 0 } },
  };
  const parsed = parseStarPayload(valid);
  assert.ok(parsed !== null, "valid star payload parses");
  assert.equal(parsed.channels.a.starred, true, "starred field present");
  const mutePayload = {
    version: 1,
    channels: { a: { muted: true, updatedAt: 1, rev: 0 } },
  };
  const rejected = parseStarPayload(mutePayload);
  assert.deepEqual(
    rejected?.channels ?? {},
    {},
    "muted entry rejected by stars parser",
  );
});

test("stars wire: outbox/subsumption callbacks are wired to stars storage (not mutes)", async () => {
  // Drive the full manager publish cycle: publish sets the stars outbox;
  // a confirming fetch returns a subsuming head → discardPending clears stars outbox.
  // A copy/paste mutation wiring mutes outbox/subsumption to the stars config would:
  //   (a) write to the mutes outbox prefix (readChannelStarsOutbox returns null), OR
  //   (b) check isMutesStoreSubsumedBy instead of isStarsStoreSubsumedBy, failing to
  //       confirm subsumption on a starred head → outbox never clears.
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-outbox-stars");
  try {
    const m = new ChannelStarSyncManager("pk-outbox-stars", RELAY);
    const store = {
      version: 1,
      channels: { ch: { starred: true, updatedAt: 100, rev: 1 } },
    };
    m.publishStars(store);
    // Outbox must be written synchronously on publish — proves writeChannelStarsOutbox is wired.
    assert.ok(
      readChannelStarsOutboxWithMeta("pk-outbox-stars", RELAY) !== null,
      "publishStars must write to the stars outbox (not mutes)",
    );
    // Drive through a full publish cycle: fire debounce, return a subsuming head on
    // both fetches (fetchOwnBlob + confirmRetainedHeadSubsumes). After discardPending,
    // the stars outbox must be cleared — proves clearChannelStarsOutbox + isStarsStoreSubsumedBy
    // are wired (a mutes-subsumption mutation would see muted=undefined → not subsumed → no clear).
    const subsumingHead = tauri.mintHead(store, 50, "evt-sub");
    subsumingHead.tags = [["d", "channel-stars"]];
    mock.method(relayClient, "fetchEvents", () =>
      Promise.resolve([subsumingHead]),
    );
    fw._fireTimer(); // fires debounce → doPublish → fetchOwnBlob → publish → confirmRetained
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(
      readChannelStarsOutboxWithMeta("pk-outbox-stars", RELAY),
      null,
    );
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("stars wire: typed API (publishStars, getPendingStarStore, fetchRemoteStars, cancelPendingStarPublish)", () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const m = new ChannelStarSyncManager("pk-api", RELAY);
    assert.equal(m.getPendingStarStore(), null, "no pending initially");
    m.publishStars({
      version: 1,
      channels: { c: { starred: true, updatedAt: 1, rev: 0 } },
    });
    assert.ok(m.getPendingStarStore() !== null, "publishStars sets pending");
    m.cancelPendingStarPublish();
    assert.ok(typeof m.cancelPendingStarPublish === "function");
    assert.ok(
      typeof m.fetchRemoteStars === "function",
      "fetchRemoteStars exists",
    );
    m.destroy();
  } finally {
    restore();
    mock.reset();
  }
});

// Mutation: removing preservedKey from mergeWithRemote call lets the clicked
// channel be evicted at 501 entries (Kalvin P3).
test("P3: clicked channel is preserved through pre-publish mergeWithRemote at capacity boundary (501 entries)", async () => {
  // Local store: 500 entries where the clicked channel has the OLDEST updatedAt
  // (so it would be evicted by boundStarStore if not preserved).
  // Remote store: the same 499 oldest channels plus one fresh channel not in local.
  // Merged result: 501 channels. Without preservedKey, the clicked channel
  // (oldest updatedAt=1) is evicted. With preservedKey it must survive.
  const MAX = 500;
  const clickedId = "ch-clicked";
  const clickedEntry = { starred: true, updatedAt: 1, rev: 1 }; // oldest updatedAt

  // Local: clicked channel + 499 others with updatedAt=100
  const localChannels = { [clickedId]: clickedEntry };
  for (let i = 0; i < MAX - 1; i++) {
    localChannels[`ch-local-${i}`] = { starred: false, updatedAt: 100, rev: 0 };
  }
  const localStore = { version: 1, channels: localChannels };

  // Remote: same 499 non-clicked channels + one fresh channel not in local
  const remoteChannels = {};
  for (let i = 0; i < MAX - 1; i++) {
    remoteChannels[`ch-local-${i}`] = {
      starred: false,
      updatedAt: 100,
      rev: 0,
    };
  }
  remoteChannels["ch-remote-new"] = { starred: false, updatedAt: 100, rev: 0 };
  const remoteStore = { version: 1, channels: remoteChannels };

  // Pre-publish fetch returns the remote store (decryptable).
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-p3-preserve");
  // Mint the remote head so the manager can decrypt it.
  const remoteHead = tauri.mintHead(remoteStore, 50, "evt-remote");
  remoteHead.tags = [["d", "channel-stars"]];

  let fetchCalls = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCalls++;
    // First fetch (pre-publish): return the remote head.
    // Subsequent fetch (confirmRetainedHeadSubsumes): return the merged published event.
    if (fetchCalls === 1) return Promise.resolve([remoteHead]);
    return Promise.resolve([]); // simplified: no confirmation needed for this test
  });
  let publishedEvent = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    publishedEvent = evt;
    return Promise.resolve();
  });

  try {
    const m = new ChannelStarSyncManager("pk-p3-preserve", RELAY);
    // Publish with preservedKey — the clicked channel must survive the merge.
    m.publishStars(localStore, clickedId);
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 30));
    // The published event was produced by mergeWithRemote(local, remote, clickedId).
    // After merging: 501 channels → bounded to 500 with clickedId preserved.
    assert.ok(publishedEvent !== null, "publish must have been attempted");
    // Decrypt the published event to verify clicked channel survived.
    const plaintext = tauri.capturedPlaintext();
    assert.ok(plaintext !== null, "encrypt must have been called");
    const published = JSON.parse(plaintext);
    assert.ok(
      clickedId in published.channels,
      `clicked channel "${clickedId}" must survive 501-entry merge when preservedKey is passed`,
    );
    assert.ok(
      "ch-remote-new" in published.channels,
      "new remote channel must be present in merged result",
    );
    assert.equal(
      Object.keys(published.channels).length,
      MAX,
      `merged result must be bounded to ${MAX} entries`,
    );
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

// P3 reconnect and P3 remount/restart scenarios are in mergeLaneHook.shared.test.mjs,
// parameterized across both lanes, triggering the actual registered reconnect callback
// and seeding foreign-nonce outbox envelopes to model quit/restart (Kalvin P3).
