/**
 * Native-mode tests for the observed-unread store.
 *
 * Every other suite in this directory runs with no `window.__TAURI_INTERNALS__`,
 * so `invokeTauri` throws and the hook takes the localStorage fallback. That
 * makes the whole native protocol untested — the first test here fails if the
 * native path is not entered, so the rest cannot silently become tautologies.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  installDOMShim,
  installFreshStorage,
  makeObservedEvent,
  mountHook,
  mountUnreadChannels,
} from "./observedUnreadTestHarness.mjs";
import {
  installNativeRig,
  makeStubRelayClient,
} from "./observedUnreadNativeRig.mjs";

installDOMShim();
installFreshStorage();

import { act } from "react";

const RELAY = "wss://relay.example.com";
const NOW_S = Math.floor(Date.now() / 1_000);

const DEFAULT_PROPS = {
  relay: RELAY,
  isReady: true,
  readStateVersion: 0,
  getTs: () => null,
  getOwn: () => null,
};

function makeRefs() {
  return {
    eventsRef: { current: new Map() },
    latestRef: { current: new Map() },
  };
}

/** Let the hook's promise chain settle (open/ingest are async). */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

test("native no-op marker delta does not notify the renderer", async () => {
  installFreshStorage();
  let harness;
  let notifications = 0;
  const rig = installNativeRig();
  try {
    harness = await mountHook(
      {
        ...DEFAULT_PROPS,
        pubkey: "pk-no-op-marker",
        getTs: () => NOW_S,
        onPruned: () => {
          notifications += 1;
        },
      },
      makeRefs(),
    );
    await settle();
    assert.equal(notifications, 1, "opening the native snapshot notifies once");

    harness.api.syncMarkers(["channel-empty"]);
    await settle();

    assert.equal(
      rig.requests("observed_unread_ingest").length,
      1,
      "the marker must still advance the native revision and ack sequence",
    );
    assert.equal(
      notifications,
      1,
      "an empty projection delta must not trigger a renderer feedback render",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("native snapshotRequired response still reopens and notifies", async () => {
  installFreshStorage();
  let harness;
  let notifications = 0;
  const scope = { pubkey: "pk-snapshot-required", relayUrl: RELAY };
  const rig = installNativeRig();
  try {
    harness = await mountHook(
      {
        ...DEFAULT_PROPS,
        pubkey: scope.pubkey,
        getTs: () => NOW_S,
        onPruned: () => {
          notifications += 1;
        },
      },
      makeRefs(),
    );
    await settle();
    rig.scope(scope).lastSequence = -1;

    harness.api.syncMarkers(["channel-gap"]);
    await settle();
    await settle();

    assert.equal(
      rig.requests("observed_unread_open_scope").length,
      2,
      "a sequence gap must reopen the scope even when it carries no projection rows",
    );
    assert.equal(
      notifications,
      2,
      "the replacement snapshot must still notify the renderer",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

// ── Entry: the boundary that makes every other test meaningful ────────────────

test("native mode is ENTERED: the hook opens the scope over the bridge", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  try {
    harness = await mountHook(
      { ...DEFAULT_PROPS, pubkey: "pk-entry" },
      makeRefs(),
    );
    await settle();

    assert.equal(
      rig.requests("observed_unread_open_scope").length,
      1,
      "the hook must call observed_unread_open_scope — if this fails, the suite is measuring the localStorage fallback and every assertion below is vacuous",
    );
    assert.equal(
      harness.api.isNative(),
      true,
      "isNative() must be true after a successful open",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("native mode is NOT entered when the bridge fails, and the hook says so", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig({
    failCommands: new Set(["observed_unread_open_scope"]),
  });
  try {
    harness = await mountHook(
      { ...DEFAULT_PROPS, pubkey: "pk-entry-fail" },
      makeRefs(),
    );
    await settle();

    assert.equal(
      harness.api.isNative(),
      false,
      "a failed open must leave the hook on the declared fallback path",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("native ingest rejection heals the chain, re-syncs projection, and preserves the next mutation", async () => {
  installFreshStorage();
  let harness;
  const failCommands = new Set();
  const rig = installNativeRig({ failCommands });
  const scope = { pubkey: "pk-ingest-recovery", relayUrl: RELAY };
  try {
    harness = await mountHook(
      {
        ...DEFAULT_PROPS,
        pubkey: scope.pubkey,
        getTs: () => NOW_S,
      },
      makeRefs(),
    );
    await settle();

    const store = rig.scope(scope);
    store.events.set("evt-recovery", {
      id: "evt-recovery",
      channelId: "channel-recovery",
      createdAt: NOW_S + 1,
      rootId: null,
      highPriority: false,
      countsTowardBadge: true,
      countsTowardAppBadge: true,
    });
    failCommands.add("observed_unread_ingest");

    harness.api.syncMarkers(["channel-failed"]);
    await settle();
    await settle();
    failCommands.delete("observed_unread_ingest");

    assert.equal(
      harness.api.projectionsRef.current.get("channel-recovery")?.count,
      1,
      "reopening after the rejected transaction must replace the optimistic projection with the authoritative snapshot",
    );
    assert.equal(
      harness.api.isNative(),
      true,
      "a successful reopen must keep native persistence healthy",
    );

    harness.api.syncMarkers(["channel-next"]);
    await settle();

    assert.equal(
      store.markers.get("channel-next"),
      NOW_S,
      "the mutation after a rejected ingest must still reach the native store",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("native ingest and reopen rejection declares native persistence unhealthy", async () => {
  installFreshStorage();
  let harness;
  const failCommands = new Set();
  const rig = installNativeRig({ failCommands });
  try {
    harness = await mountHook(
      {
        ...DEFAULT_PROPS,
        pubkey: "pk-ingest-reopen-failure",
        getTs: () => NOW_S,
      },
      makeRefs(),
    );
    await settle();
    failCommands.add("observed_unread_ingest");
    failCommands.add("observed_unread_open_scope");

    harness.api.syncMarkers(["channel-failed"]);
    await settle();
    await settle();

    assert.equal(
      harness.api.isNative(),
      false,
      "if the authoritative snapshot cannot be reopened, isNative must stop reporting the failed path as healthy",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

// ── D1: local mark-read must reach the native store ──────────────────────────

test("D1: local markChannelRead sends a read marker to the native store", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  try {
    const PUBKEY = "pk-d1";
    const CHANNEL = "channel-d1";
    harness = await mountUnreadChannels({
      pubkey: PUBKEY,
      relay: RELAY,
      channels: [{ id: CHANNEL, name: "d1", channelType: "stream" }],
      relayClient: makeStubRelayClient(),
    });
    await settle();

    const readAt = new Date(NOW_S * 1_000).toISOString();
    await act(async () => {
      harness.markChannelRead(CHANNEL, readAt);
    });
    await settle();

    const markers = rig.markerUpdates();
    assert.ok(
      markers.some((marker) => marker.contextId === CHANNEL),
      `local mark-read must reach observed_unread_ingest as a marker for ${CHANNEL}; saw ${JSON.stringify(markers)}`,
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("D1 control: the rig DOES record markers when syncMarkers is called directly", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  try {
    harness = await mountHook(
      {
        ...DEFAULT_PROPS,
        pubkey: "pk-d1-control",
        getTs: () => NOW_S,
      },
      makeRefs(),
    );
    await settle();

    harness.api.syncMarkers(["channel-control"]);
    await settle();

    assert.deepEqual(
      rig.markerUpdates(),
      [{ contextId: "channel-control", readAt: NOW_S }],
      "positive control: the marker path is observable through the rig, so a zero-marker result above means the code did not send one",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

// ── D2: maxTrigger must survive in native mode ───────────────────────────────

test("D2: a catch-up maxTrigger with no notifying event still advances native latest", async () => {
  installFreshStorage();
  const CHANNEL = "channel-d2";
  const MAX_TRIGGER = NOW_S - 10;
  let harness;
  const rig = installNativeRig({
    catchUpChannels: (request) =>
      request.channels.map((channel) => ({
        status: "success",
        channelId: channel.id,
        // The regression case: a trigger newer than the read marker that does
        // NOT survive the notify filter, so it produces no observed event.
        observedEvents: [],
        maxTrigger: MAX_TRIGGER,
        activityRows: [],
        discovered: { participated: [], authored: [], mentioned: [] },
      })),
  });
  try {
    harness = await mountUnreadChannels({
      pubkey: "pk-d2",
      relay: RELAY,
      channels: [{ id: CHANNEL, name: "d2", channelType: "stream" }],
      relayClient: makeStubRelayClient(),
    });
    await settle();
    await settle();

    assert.equal(
      rig.requests("unread_catch_up").length >= 1,
      true,
      "catch-up must have run for this assertion to mean anything",
    );
    assert.equal(
      rig
        .scope({ pubkey: "pk-d2", relayUrl: RELAY })
        .channelLatest.get(CHANNEL),
      MAX_TRIGGER,
      `maxTrigger ${MAX_TRIGGER} must survive as the channel latest anchor even when no observed row is returned`,
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

// ── D3: an empty seed must not wipe accumulated native membership ────────────

test("D3: reopening with an empty membership seed preserves discovered membership", async () => {
  installFreshStorage();
  let first;
  let second;
  const rig = installNativeRig();
  const scope = { pubkey: "pk-d3", relayUrl: RELAY };
  const emptySeed = {
    participatedRootIds: [],
    authoredRootIds: [],
    mentionedRootIds: [],
    followedRootIds: [],
    mutedRootIds: [],
    mutedChannelIds: [],
  };
  try {
    first = await mountHook(
      { ...DEFAULT_PROPS, pubkey: scope.pubkey, membershipSeed: emptySeed },
      makeRefs(),
    );
    await settle();

    // Catch-up discovery writes membership incrementally, as the commit
    // message's ownership story describes.
    first.api.updateMembership("participated", "root-discovered", true);
    await settle();
    assert.ok(
      rig.scope(scope).membership.has("participated\u0000root-discovered"),
      "precondition: discovery must have written membership natively",
    );
    await first.unmount();
    first = null;

    // Restart with an empty renderer seed (localStorage cleared / read failed).
    second = await mountHook(
      { ...DEFAULT_PROPS, pubkey: scope.pubkey, membershipSeed: emptySeed },
      makeRefs(),
    );
    await settle();

    assert.ok(
      rig.scope(scope).membership.has("participated\u0000root-discovered"),
      "an empty renderer seed must not delete membership the native store accumulated",
    );
  } finally {
    await first?.unmount();
    await second?.unmount();
    rig.restore();
  }
});

// ── Matrix rows that only became reachable once native mode was enterable ─────

test("matrix: a replayed sequence is a no-op, not a second mutation", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  const scope = { pubkey: "pk-replay", relayUrl: RELAY };
  try {
    harness = await mountHook(
      { ...DEFAULT_PROPS, pubkey: scope.pubkey },
      makeRefs(),
    );
    await settle();

    harness.api.updateMembership("followed", "root-1", true);
    await settle();
    const afterFirst = rig.scope(scope).revision;

    const replay = rig.requests("observed_unread_ingest").at(-1);
    const response = await globalThis.window.__TAURI_INTERNALS__.invoke(
      "observed_unread_ingest",
      { request: replay },
    );

    assert.equal(
      response.kind,
      "snapshot",
      "replay must return a snapshot, not a delta",
    );
    assert.equal(
      rig.scope(scope).revision,
      afterFirst,
      "replaying an acked sequence must not advance the revision",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("matrix: a sequence gap is rejected with snapshotRequired", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  const scope = { pubkey: "pk-gap", relayUrl: RELAY };
  try {
    harness = await mountHook(
      { ...DEFAULT_PROPS, pubkey: scope.pubkey },
      makeRefs(),
    );
    await settle();

    const current = rig.scope(scope);
    const response = await globalThis.window.__TAURI_INTERNALS__.invoke(
      "observed_unread_ingest",
      {
        request: {
          scope,
          sequence: current.lastSequence + 2,
          baseRevision: current.revision,
          events: [],
          markers: [],
          membership: [],
          clearChannels: [],
          clearAll: false,
        },
      },
    );

    assert.equal(response.kind, "snapshotRequired");
    assert.equal(
      rig.scope(scope).lastSequence,
      current.lastSequence,
      "a gap must not advance the ack",
    );
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("matrix: an ingested event reaches the projection the badge reads", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig();
  const scope = { pubkey: "pk-project", relayUrl: RELAY };
  try {
    const refs = makeRefs();
    harness = await mountHook({ ...DEFAULT_PROPS, pubkey: scope.pubkey }, refs);
    await settle();

    harness.api.schedule(
      harness.api.currentScope,
      "channel-p",
      makeObservedEvent({ id: "evt-p", createdAt: NOW_S }),
    );
    harness.flushNative?.();
    await act(async () => {
      globalThis.dispatchEvent(
        new (class extends Event {
          constructor() {
            super("pagehide");
          }
        })(),
      );
    });
    await settle();

    assert.equal(
      harness.api.projectionsRef.current.get("channel-p")?.count,
      1,
      "the native projection must carry the ingested event",
    );
    assert.equal(harness.api.latestForChannel("channel-p"), NOW_S);
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

test("matrix: a rebuilt store generation is reopened instead of wedging on the old revision", async () => {
  installFreshStorage();
  let harness;
  const rig = installNativeRig({
    newGeneration: (() => {
      let generation = 0;
      return () => `gen-${++generation}`;
    })(),
  });
  const scope = { pubkey: "pk-epoch", relayUrl: RELAY };
  try {
    harness = await mountHook(
      { ...DEFAULT_PROPS, pubkey: scope.pubkey },
      makeRefs(),
    );
    await settle();
    harness.api.updateMembership("followed", "before-rebuild", true);
    await settle();
    assert.equal(
      rig.scope(scope).revision,
      1,
      "precondition: renderer holds revision 1",
    );

    const rebuilt = rig.rebuildScope(scope);
    harness.api.updateMembership("followed", "after-rebuild", true);
    await settle();
    await settle();

    assert.equal(rebuilt.generation, "gen-2");
    assert.ok(
      rig.requests("observed_unread_open_scope").length >= 2,
      "generation mismatch must reopen for a replacement snapshot",
    );
    assert.equal(harness.api.isNative(), true);
  } finally {
    await harness?.unmount();
    rig.restore();
  }
});

// ── The badge lane below the projection ──────────────────────────────────────
//
// `matrix: an ingested event reaches the projection the badge reads` asserts
// projectionsRef — the map. It never reads the `rawUnread` memo that turns a
// projection into unreadChannelIds / unreadChannelCounts, so the whole native
// badge lane had no witness: forcing `nativeProjection?.count ?? 0` to a
// constant 0 left the full 4,919-test suite green. This closes that.

test("native: an ingested event reaches the hook's unread counts, not just the projection", async () => {
  installFreshStorage();
  const CHANNEL = "channel-badge";
  const scope = { pubkey: "pk-badge", relayUrl: RELAY };
  let first;
  let second;
  const rig = installNativeRig();
  try {
    // First mount creates the native scope.
    first = await mountUnreadChannels({
      pubkey: scope.pubkey,
      relay: RELAY,
      channels: [{ id: CHANNEL, channelType: "channel" }],
      relayClient: makeStubRelayClient(),
    });
    await settle();
    const store = rig.scope(scope);
    assert.ok(store, "precondition: native mode must be entered");
    await first.unmount();
    first = null;

    // A notifying event exists natively when the renderer reopens.
    store.events.set("evt-badge", {
      id: "evt-badge",
      channelId: CHANNEL,
      createdAt: NOW_S,
      rootId: null,
      highPriority: false,
      countsTowardBadge: true,
      countsTowardAppBadge: true,
    });
    assert.equal(
      store.projections().find((p) => p.channelId === CHANNEL)?.count,
      1,
      "precondition: the native projection itself must count the event",
    );

    // Reopen: the open snapshot carries the projection into the renderer.
    second = await mountUnreadChannels({
      pubkey: scope.pubkey,
      relay: RELAY,
      channels: [{ id: CHANNEL, channelType: "channel" }],
      relayClient: makeStubRelayClient(),
    });
    await settle();
    await settle();

    assert.equal(
      second.result.unreadChannelCounts.get(CHANNEL),
      1,
      "the native badgeCount must reach unreadChannelCounts; asserting projectionsRef alone leaves this lane untested",
    );
    assert.ok(
      second.result.unreadChannelIds.has(CHANNEL),
      "the channel must appear in unreadChannelIds",
    );
  } finally {
    await first?.unmount();
    await second?.unmount();
    rig.restore();
  }
});
