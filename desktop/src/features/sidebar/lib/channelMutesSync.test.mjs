// Compact wire-contract adapter for ChannelMuteSyncManager.
// Shared engine invariants are in mergeLaneSync.shared.test.mjs.
// This file asserts only mutes-specific wiring: event kind, d-tag, payload shape, subsumption, and typed API.

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  parseMutePayload,
  readChannelMutesOutboxWithMeta,
} from "./channelMutesStorage.ts";
import { ChannelMuteSyncManager } from "./channelMutesSync.ts";
import {
  installEchoTauri,
  installFakeWindow,
  makeFakeWindow,
} from "./sidebarSyncTestHelpers.mjs";

const RELAY = "wss://r.test";

test("mutes wire: kind=30078, d-tag='channel-mutes', payload has 'channels' not 'sections'", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  let publishedEvent = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    publishedEvent = evt;
    return Promise.resolve();
  });
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-wire-mutes");
  try {
    const m = new ChannelMuteSyncManager("pk-wire-mutes", RELAY);
    m.publishMutes({
      version: 1,
      channels: { ch1: { muted: true, updatedAt: 1, rev: 0 } },
    });
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 20));
    assert.ok(publishedEvent !== null, "publish must have been called");
    assert.equal(publishedEvent.kind, 30078, "kind must be 30078");
    const dTag = publishedEvent.tags.find((t) => t[0] === "d")?.[1];
    assert.equal(dTag, "channel-mutes", "d-tag must not be 'channel-stars'");
    const plaintext = tauri.capturedPlaintext();
    const parsed = JSON.parse(plaintext);
    assert.ok("channels" in parsed, "payload must have 'channels' field");
    assert.ok(!("sections" in parsed), "payload must not have 'sections'");
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("mutes wire: parser delegates to parseMutePayload (muted field, rejects starred)", () => {
  const valid = {
    version: 1,
    channels: { a: { muted: true, updatedAt: 1, rev: 0 } },
  };
  const parsed = parseMutePayload(valid);
  assert.ok(parsed !== null, "valid mute payload parses");
  assert.equal(parsed.channels.a.muted, true, "muted field present");
  const starPayload = {
    version: 1,
    channels: { a: { starred: true, updatedAt: 1, rev: 0 } },
  };
  const rejected = parseMutePayload(starPayload);
  assert.deepEqual(
    rejected?.channels ?? {},
    {},
    "starred entry rejected by mutes parser",
  );
});

test("mutes wire: outbox/subsumption callbacks are wired to mutes storage (not stars)", async () => {
  // Drive the full manager publish cycle: publish sets the mutes outbox;
  // a confirming fetch returns a subsuming head → discardPending clears mutes outbox.
  // A copy/paste mutation wiring stars outbox/subsumption to the mutes config would:
  //   (a) write to the stars outbox prefix (readChannelMutesOutbox returns null), OR
  //   (b) check isStarsStoreSubsumedBy instead of isMutesStoreSubsumedBy, failing to
  //       confirm subsumption on a muted head → outbox never clears.
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", () => Promise.resolve());
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-outbox-mutes");
  try {
    const m = new ChannelMuteSyncManager("pk-outbox-mutes", RELAY);
    const store = {
      version: 1,
      channels: { ch: { muted: true, updatedAt: 100, rev: 1 } },
    };
    m.publishMutes(store);
    // Outbox must be written synchronously on publish — proves writeChannelMutesOutbox is wired.
    assert.ok(
      readChannelMutesOutboxWithMeta("pk-outbox-mutes", RELAY) !== null,
      "publishMutes must write to the mutes outbox (not stars)",
    );
    // Drive through a full publish cycle with a subsuming head. After discardPending,
    // the mutes outbox must be cleared — proves clearChannelMutesOutbox + isMutesStoreSubsumedBy
    // are wired (a stars-subsumption mutation would see starred=undefined → not subsumed → no clear).
    const subsumingHead = tauri.mintHead(store, 50, "evt-sub-mutes");
    subsumingHead.tags = [["d", "channel-mutes"]];
    mock.method(relayClient, "fetchEvents", () =>
      Promise.resolve([subsumingHead]),
    );
    fw._fireTimer(); // fires debounce → doPublish → fetchOwnBlob → publish → confirmRetained
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(
      readChannelMutesOutboxWithMeta("pk-outbox-mutes", RELAY),
      null,
    );
    m.destroy();
  } finally {
    tauri.restore();
    restore();
    mock.reset();
  }
});

test("mutes wire: typed API (publishMutes, getPendingMuteStore, fetchRemoteMutes, cancelPendingMutePublish)", () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  try {
    const m = new ChannelMuteSyncManager("pk-api-m", RELAY);
    assert.equal(m.getPendingMuteStore(), null, "no pending initially");
    m.publishMutes({
      version: 1,
      channels: { c: { muted: true, updatedAt: 1, rev: 0 } },
    });
    assert.ok(m.getPendingMuteStore() !== null, "publishMutes sets pending");
    m.cancelPendingMutePublish();
    assert.ok(typeof m.cancelPendingMutePublish === "function");
    assert.ok(
      typeof m.fetchRemoteMutes === "function",
      "fetchRemoteMutes exists",
    );
    m.destroy();
  } finally {
    restore();
    mock.reset();
  }
});

// Mutation: removing preservedKey from mergeWithRemote call lets the clicked
// channel be evicted at 501 entries (Kalvin P3 — mutes lane).
test("P3: clicked channel is preserved through pre-publish mergeWithRemote at capacity boundary (501 entries)", async () => {
  const MAX = 500;
  const clickedId = "ch-clicked-mute";
  const clickedEntry = { muted: true, updatedAt: 1, rev: 1 }; // oldest updatedAt

  // Local: clicked channel + 499 others with updatedAt=100
  const localChannels = { [clickedId]: clickedEntry };
  for (let i = 0; i < MAX - 1; i++) {
    localChannels[`ch-local-${i}`] = { muted: false, updatedAt: 100, rev: 0 };
  }
  const localStore = { version: 1, channels: localChannels };

  // Remote: same 499 non-clicked channels + one fresh channel not in local
  const remoteChannels = {};
  for (let i = 0; i < MAX - 1; i++) {
    remoteChannels[`ch-local-${i}`] = { muted: false, updatedAt: 100, rev: 0 };
  }
  remoteChannels["ch-remote-new"] = { muted: false, updatedAt: 100, rev: 0 };
  const remoteStore = { version: 1, channels: remoteChannels };

  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  const tauri = installEchoTauri("pk-p3-preserve-mutes");
  const remoteHead = tauri.mintHead(remoteStore, 50, "evt-remote-mutes");
  remoteHead.tags = [["d", "channel-mutes"]];

  let fetchCalls = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCalls++;
    if (fetchCalls === 1) return Promise.resolve([remoteHead]);
    return Promise.resolve([]);
  });
  let publishedEvent = null;
  mock.method(relayClient, "publishEvent", (evt) => {
    publishedEvent = evt;
    return Promise.resolve();
  });

  try {
    const m = new ChannelMuteSyncManager("pk-p3-preserve-mutes", RELAY);
    m.publishMutes(localStore, clickedId);
    fw._fireTimer();
    await new Promise((r) => setTimeout(r, 30));
    assert.ok(publishedEvent !== null, "publish must have been attempted");
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
