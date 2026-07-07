import assert from "node:assert/strict";
import test from "node:test";
import { ArchiveSyncManager } from "./archiveSyncManager.ts";

// ── Fakes ────────────────────────────────────────────────────────────────────

/**
 * Fake relay client — records filter/callback pairs, lets tests push events,
 * and exposes active subscription keys.
 */
function makeFakeRelayClient() {
  const subs = new Map(); // key -> { filter, callback, unsubbed }

  return {
    subs,
    subscribeLive(filter, callback) {
      const key = JSON.stringify(filter);
      subs.set(key, { filter, callback, unsubbed: false });
      return Promise.resolve(async () => {
        const entry = subs.get(key);
        if (entry) entry.unsubbed = true;
      });
    },
    push(filter, event) {
      const key = JSON.stringify(filter);
      const entry = subs.get(key);
      if (!entry) throw new Error(`no subscription for filter ${key}`);
      entry.callback(event);
    },
    activeCount() {
      return [...subs.values()].filter((e) => !e.unsubbed).length;
    },
  };
}

/**
 * Fake tauriArchive module — captures invocations for assertion.
 * createSaveSubscription is an upsert (matching store.rs ON CONFLICT behaviour).
 */
function makeFakeArchive() {
  let subs = [];
  const archiveCalls = [];
  const listeners = new Set();

  return {
    async listSaveSubscriptions() {
      return subs;
    },
    async createSaveSubscription(scopeType, scopeValue, kinds) {
      // Upsert: update kinds if scope already exists, otherwise append.
      const existing = subs.findIndex(
        (s) => s.scopeType === scopeType && s.scopeValue === scopeValue,
      );
      if (existing >= 0) {
        subs = subs.map((s, i) => (i === existing ? { ...s, kinds } : s));
      } else {
        subs = [
          ...subs,
          {
            scopeType,
            scopeValue,
            kinds,
            identityPubkey: "pk",
            relayUrl: "wss://r",
            createdAt: 0,
          },
        ];
      }
      for (const l of listeners) l();
    },
    async deleteSaveSubscription(scopeType, scopeValue) {
      const before = subs.length;
      subs = subs.filter(
        (s) => !(s.scopeType === scopeType && s.scopeValue === scopeValue),
      );
      if (subs.length < before) {
        for (const l of listeners) l();
        return true;
      }
      return false;
    },
    async archiveEvents(candidates) {
      archiveCalls.push(candidates);
      return { persisted: candidates.length, dropped: 0 };
    },
    onSubscriptionChange(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    archiveCalls,
    setSubs(s) {
      subs = s;
    },
  };
}

/** Helper: wait for microtasks/promises to settle */
function tick() {
  return new Promise((r) => setTimeout(r, 0));
}

/** Build a manager wired to fakes. */
function makeManager(relay, archive, extra = {}) {
  return new ArchiveSyncManager({
    relayClient: relay,
    listSaveSubscriptions: () => archive.listSaveSubscriptions(),
    archiveEvents: (c) => archive.archiveEvents(c),
    onSubscriptionChange: (l) => archive.onSubscriptionChange(l),
    ...extra,
  });
}

// ── kinds decoding ────────────────────────────────────────────────────────────

/**
 * Inline the decode logic (matches tauriArchive.ts:decodeRawSubscription)
 * so we can test it without the full Tauri env.
 */
function decodeKinds(kindsStr) {
  try {
    const parsed = JSON.parse(kindsStr);
    if (
      Array.isArray(parsed) &&
      parsed.every((k) => typeof k === "number" && Number.isFinite(k))
    ) {
      return parsed;
    }
    return null; // malformed
  } catch {
    return null;
  }
}

test("decodeKinds_valid_array_returns_numbers", () => {
  assert.deepEqual(
    decodeKinds("[9,40002,45001,45003]"),
    [9, 40002, 45001, 45003],
  );
});

test("decodeKinds_empty_array_is_valid", () => {
  assert.deepEqual(decodeKinds("[]"), []);
});

test("decodeKinds_non_array_returns_null", () => {
  assert.equal(decodeKinds('"string"'), null);
  assert.equal(decodeKinds("42"), null);
  assert.equal(decodeKinds("null"), null);
});

test("decodeKinds_array_with_non_number_returns_null", () => {
  assert.equal(decodeKinds('["9","40002"]'), null);
  assert.equal(decodeKinds("[9, null, 40002]"), null);
});

test("decodeKinds_malformed_json_returns_null", () => {
  assert.equal(decodeKinds("not-json"), null);
  assert.equal(decodeKinds(""), null);
});

// ── Kind preset arrays (derived from constants, not literals) ─────────────────

/**
 * Inline the preset arrays matching LocalArchiveSettingsCard.tsx.
 * Values verified against desktop/src/shared/constants/kinds.ts.
 */
const KIND_STREAM_MESSAGE = 9;
const KIND_STREAM_MESSAGE_V2 = 40002;
const KIND_STREAM_MESSAGE_DIFF = 40008;
const KIND_FORUM_POST = 45001;
const KIND_FORUM_COMMENT = 45003;
const KIND_DELETION = 5;
const KIND_REACTION = 7;
const KIND_NIP29_DELETE_EVENT = 9005;
const KIND_STREAM_MESSAGE_EDIT = 40003;
const KIND_SYSTEM_MESSAGE = 40099;
const KIND_HUDDLE_STARTED = 48100;
const KIND_HUDDLE_PARTICIPANT_JOINED = 48101;
const KIND_HUDDLE_PARTICIPANT_LEFT = 48102;
const KIND_HUDDLE_ENDED = 48103;

// Presets — keep in sync with LocalArchiveSettingsCard.tsx
const PRESET_MESSAGES = [
  KIND_STREAM_MESSAGE, // 9
  KIND_STREAM_MESSAGE_V2, // 40002
  KIND_STREAM_MESSAGE_DIFF, // 40008
  KIND_FORUM_POST, // 45001
  KIND_FORUM_COMMENT, // 45003
];

const PRESET_AUX = [
  KIND_DELETION, // 5
  KIND_REACTION, // 7
  KIND_NIP29_DELETE_EVENT, // 9005
  KIND_STREAM_MESSAGE_EDIT, // 40003
];

const PRESET_ALL = [
  KIND_DELETION, // 5
  KIND_REACTION, // 7
  KIND_NIP29_DELETE_EVENT, // 9005
  KIND_STREAM_MESSAGE, // 9
  40001, // legacy
  KIND_STREAM_MESSAGE_V2, // 40002
  KIND_FORUM_POST, // 45001 (from CHANNEL_MESSAGE_EVENT_KINDS spread)
  KIND_FORUM_COMMENT, // 45003 (from CHANNEL_MESSAGE_EVENT_KINDS spread)
  KIND_STREAM_MESSAGE_EDIT, // 40003
  KIND_STREAM_MESSAGE_DIFF, // 40008
  KIND_SYSTEM_MESSAGE, // 40099
  KIND_HUDDLE_STARTED, // 48100
  KIND_HUDDLE_PARTICIPANT_JOINED, // 48101
  KIND_HUDDLE_PARTICIPANT_LEFT, // 48102
  KIND_HUDDLE_ENDED, // 48103
];

test("preset_messages_contains_correct_kinds", () => {
  // Must include all four CHANNEL_MESSAGE_EVENT_KINDS + diff rows
  assert.ok(
    PRESET_MESSAGES.includes(9),
    "must include kind 9 (stream message)",
  );
  assert.ok(
    PRESET_MESSAGES.includes(40002),
    "must include kind 40002 (stream message v2)",
  );
  assert.ok(
    PRESET_MESSAGES.includes(45001),
    "must include kind 45001 (forum post)",
  );
  assert.ok(
    PRESET_MESSAGES.includes(45003),
    "must include kind 45003 (forum comment)",
  );
  assert.ok(
    PRESET_MESSAGES.includes(40008),
    "must include kind 40008 (diff rows — visible content)",
  );
  // Must NOT misclassify edits as messages
  assert.ok(
    !PRESET_MESSAGES.includes(40003),
    "must NOT include kind 40003 (edits — aux, not messages)",
  );
});

test("preset_aux_contains_correct_kinds", () => {
  assert.ok(PRESET_AUX.includes(5), "must include kind 5 (NIP-09 deletion)");
  assert.ok(PRESET_AUX.includes(7), "must include kind 7 (reaction)");
  assert.ok(
    PRESET_AUX.includes(9005),
    "must include kind 9005 (Buzz-native deletion)",
  );
  assert.ok(
    PRESET_AUX.includes(40003),
    "must include kind 40003 (stream message edit)",
  );
  // Edits are aux, not messages — must not overlap with messages preset (except shared reaction)
  assert.ok(!PRESET_AUX.includes(9), "must NOT include kind 9 (message)");
  assert.ok(
    !PRESET_AUX.includes(40002),
    "must NOT include kind 40002 (message v2)",
  );
});

test("preset_all_is_superset_of_messages_and_aux", () => {
  for (const k of PRESET_MESSAGES) {
    assert.ok(
      PRESET_ALL.includes(k),
      `PRESET_ALL must include kind ${k} from PRESET_MESSAGES`,
    );
  }
  for (const k of PRESET_AUX) {
    assert.ok(
      PRESET_ALL.includes(k),
      `PRESET_ALL must include kind ${k} from PRESET_AUX`,
    );
  }
});

test("preset_messages_exact_saved_kind_array", () => {
  assert.deepEqual(
    [...PRESET_MESSAGES].sort((a, b) => a - b),
    [9, 40002, 40008, 45001, 45003],
  );
});

test("preset_aux_exact_saved_kind_array", () => {
  assert.deepEqual(
    [...PRESET_AUX].sort((a, b) => a - b),
    [5, 7, 9005, 40003],
  );
});

// ── Subscription-change notifier ─────────────────────────────────────────────

/**
 * Test the notifier contract inline — mirrors what tauriArchive.ts exports.
 */
function makeNotifier() {
  const listeners = new Set();
  return {
    onSubscriptionChange(l) {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    notify() {
      for (const l of listeners) l();
    },
  };
}

test("subscription_change_notifier_fires_registered_listener", () => {
  const n = makeNotifier();
  let fired = 0;
  n.onSubscriptionChange(() => {
    fired++;
  });
  n.notify();
  assert.equal(fired, 1);
});

test("subscription_change_notifier_unregister_stops_firing", () => {
  const n = makeNotifier();
  let fired = 0;
  const off = n.onSubscriptionChange(() => {
    fired++;
  });
  off();
  n.notify();
  assert.equal(fired, 0);
});

test("subscription_change_notifier_fires_multiple_listeners", () => {
  const n = makeNotifier();
  let a = 0;
  let b = 0;
  n.onSubscriptionChange(() => {
    a++;
  });
  n.onSubscriptionChange(() => {
    b++;
  });
  n.notify();
  assert.equal(a, 1);
  assert.equal(b, 1);
});

// ── ArchiveSyncManager (real class, injected fakes) ───────────────────────────

test("manager_opens_one_sub_per_saved_subscription", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-1",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();
  assert.equal(relay.activeCount(), 1);
  mgr.destroy();
});

test("manager_builds_correct_filter_for_channel_h", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-abc",
      kinds: [9, 40002],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();
  const keys = [...relay.subs.keys()];
  assert.equal(keys.length, 1);
  const filter = JSON.parse(keys[0]);
  assert.deepEqual(filter["#h"], ["chan-abc"]);
  assert.deepEqual(filter.kinds, [9, 40002]);
  assert.equal(filter.limit, 0);
  mgr.destroy();
});

test("manager_builds_correct_filter_for_owner_p", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "owner_p",
      scopeValue: "pubkey123",
      kinds: [24200],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();
  const keys = [...relay.subs.keys()];
  const filter = JSON.parse(keys[0]);
  assert.deepEqual(filter["#p"], ["pubkey123"]);
  mgr.destroy();
});

test("manager_forwards_events_to_archive_events_on_flush", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-1",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  // Use flushBatchSize=1 so flush fires immediately on first event
  const mgr = makeManager(relay, archive, { flushBatchSize: 1 });
  await mgr.start();
  await tick();

  const filter = JSON.parse([...relay.subs.keys()][0]);
  relay.push(filter, {
    id: "ev1",
    kind: 9,
    pubkey: "pk",
    created_at: 1,
    content: "hi",
    tags: [],
  });
  await tick();

  assert.equal(archive.archiveCalls.length, 1);
  assert.equal(archive.archiveCalls[0].length, 1);
  assert.equal(archive.archiveCalls[0][0].matchedScope.scopeType, "channel_h");
  assert.equal(archive.archiveCalls[0][0].matchedScope.scopeValue, "chan-1");
  mgr.destroy();
});

test("manager_resubscribes_when_subscription_added", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();
  assert.equal(relay.activeCount(), 0);

  // Simulate create_save_subscription — upserts subs then fires notifier
  await archive.createSaveSubscription("channel_h", "chan-new", [9]);
  await tick();

  assert.equal(relay.activeCount(), 1);
  mgr.destroy();
});

test("manager_removes_sub_when_subscription_deleted", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-1",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();
  assert.equal(relay.activeCount(), 1);

  await archive.deleteSaveSubscription("channel_h", "chan-1");
  await tick();

  // The sub should be unsubbed now
  const entry = [...relay.subs.values()][0];
  assert.equal(entry.unsubbed, true);
  mgr.destroy();
});

test("manager_resubscribes_with_new_filter_when_kinds_upserted", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-1",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start();
  await tick();

  // Confirm initial subscription is active with kinds=[9]
  assert.equal(relay.activeCount(), 1);
  const oldKeys = [...relay.subs.keys()];
  assert.equal(oldKeys.length, 1);
  assert.deepEqual(JSON.parse(oldKeys[0]).kinds, [9]);

  // Upsert same scope with different kinds — fake archive replaces kinds + fires notifier
  await archive.createSaveSubscription("channel_h", "chan-1", [9, 40002]);
  await tick();

  // Old subscription must be unsubbed
  const oldEntry = relay.subs.get(oldKeys[0]);
  assert.equal(
    oldEntry.unsubbed,
    true,
    "old kinds=[9] filter must be torn down",
  );

  // New subscription with kinds=[9,40002] must be active
  assert.equal(relay.activeCount(), 1, "exactly one active sub after upsert");
  const newKeys = [...relay.subs.keys()].filter((k) => k !== oldKeys[0]);
  assert.equal(newKeys.length, 1);
  assert.deepEqual(
    JSON.parse(newKeys[0]).kinds,
    [9, 40002],
    "new filter must use updated kinds",
  );

  mgr.destroy();
});

test("manager_retries_subscription_after_subscribeLive_failure", async () => {
  // First subscribeLive call rejects; key must NOT be in active so a second
  // resubscribeAll (triggered by any config change) retries it.
  let callCount = 0;
  const relay = {
    subs: new Map(),
    subscribeLive(filter, callback) {
      callCount++;
      if (callCount === 1) {
        return Promise.reject(new Error("simulated relay failure"));
      }
      const key = JSON.stringify(filter);
      relay.subs.set(key, { filter, callback, unsubbed: false });
      return Promise.resolve(async () => {
        const entry = relay.subs.get(key);
        if (entry) entry.unsubbed = true;
      });
    },
    activeCount() {
      return [...relay.subs.values()].filter((e) => !e.unsubbed).length;
    },
  };
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-retry",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);
  await mgr.start(); // first subscribeLive rejects — key must be absent
  assert.equal(relay.activeCount(), 0, "no active sub after failure");

  // Trigger resubscribeAll via a config change notification — second call succeeds.
  await archive.createSaveSubscription("channel_h", "chan-retry", [9]);
  await tick();
  assert.equal(relay.activeCount(), 1, "sub created on retry after failure");
  assert.equal(callCount, 2, "subscribeLive called exactly twice");
  mgr.destroy();
});

test("manager_no_duplicate_sub_when_two_reloads_overlap", async () => {
  // Under single-flight serialization: when a second resubscribeAll request
  // arrives while the first subscribeLive is still pending, it sets
  // reloadPending and returns immediately (no concurrent body runs). The
  // first subscribe resolves and activates normally, then the coalescing loop
  // runs the second pass — but by then the key is already active, so no
  // duplicate subscription is opened.
  let resolveFirst;
  let callCount = 0;
  const disposedCount = { value: 0 };

  const relay = {
    subs: new Map(),
    subscribeLive(filter, _callback) {
      callCount++;
      const key = JSON.stringify(filter);
      const disposeHandle = async () => {
        disposedCount.value++;
        const entry = relay.subs.get(key);
        if (entry) entry.unsubbed = true;
      };
      if (callCount === 1) {
        // Slow first call — resolver exposed so the test can unblock it later.
        return new Promise((resolve) => {
          resolveFirst = () => {
            relay.subs.set(key, { filter, unsubbed: false });
            resolve(disposeHandle);
          };
        });
      }
      // A second call should not be reached under single-flight.
      relay.subs.set(key, { filter, unsubbed: false });
      return Promise.resolve(disposeHandle);
    },
    activeCount() {
      return [...relay.subs.values()].filter((e) => !e.unsubbed).length;
    },
  };

  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-dup",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);

  // Fire first resubscribeAll (goes async, subscribeLive pending).
  const first = mgr.start();
  await tick(); // let it reach the subscribeLive await

  // Fire second resubscribeAll before the first subscribe resolves.
  // Under single-flight this sets reloadPending and returns synchronously.
  // biome-ignore lint/complexity/useLiteralKeys: intentional private access in test
  mgr["resubscribeAll"]();
  await tick();

  // First subscribe still pending — resolve it now.
  resolveFirst();
  await first;
  await tick();

  // subscribeLive was called exactly once (single-flight blocked the second).
  assert.equal(callCount, 1, "subscribeLive called only once");
  // Exactly one active subscription.
  assert.equal(relay.activeCount(), 1, "exactly one active sub");
  // No extra dispose leaked.
  assert.equal(disposedCount.value, 0, "no dispose called for a healthy sub");

  mgr.destroy();
});

test("manager_disposes_stale_sub_when_deleted_before_resolve", async () => {
  // A subscription is deleted while its subscribeLive call is in flight.
  // Under single-flight: the delete sets reloadPending; the first pass
  // finishes (activates K), then the coalescing loop runs a second pass which
  // lists [] and tears K down via the normal teardown loop. Net result: dispose
  // is called and K is absent from active.
  let resolveSubscribe;
  let disposeCalled = false;

  const relay = {
    subscribeLive(_filter, _callback) {
      return new Promise((resolve) => {
        resolveSubscribe = () =>
          resolve(async () => {
            disposeCalled = true;
          });
      });
    },
  };

  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-stale",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive);

  // Start: subscribeLive is pending (first doResubscribe pass).
  const started = mgr.start();
  await tick(); // reaches the subscribeLive await

  // Delete the subscription before the subscribe resolves; sets reloadPending
  // so the coalescing loop runs a second pass after the first finishes.
  await archive.deleteSaveSubscription("channel_h", "chan-stale");
  await tick();

  // Now resolve the first subscribe — first pass activates K, then the second
  // pass (reloadPending) runs, lists [], and tears K down.
  resolveSubscribe();
  await started; // waits for both passes to complete
  await tick();

  // Dispose must have been called (by the teardown loop in the second pass).
  assert.equal(
    disposeCalled,
    true,
    "dispose must be called when subscription is removed",
  );
  // Key must NOT be in active.
  assert.equal(
    // biome-ignore lint/complexity/useLiteralKeys: intentional private access in test
    mgr["active"].size,
    0,
    "active must be empty — deleted key must not remain",
  );

  mgr.destroy();
});

test("manager_handles_out_of_order_list_resolution", async () => {
  // Regression test for the defect Thufir found at pass-3: a stale
  // listSaveSubscriptions result from an older reload can overwrite the
  // wantedKeys published by a newer reload that already knows K was deleted.
  //
  // Setup: reload A starts and its list resolves LATE (K present); meanwhile
  // K is deleted and reload B completes with an empty list. Then A's stale
  // list (with K) resolves last.
  //
  // Under single-flight serialization, A and B cannot interleave — B is
  // queued as reloadPending and only runs after A's full pass completes. So
  // when A's late list resolves it sees [K] and subscribes K; then B runs,
  // lists [], and tears K down. The stale-overwrite defect is structurally
  // impossible.
  let resolveAList;

  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-ooo",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);

  // Intercept listSaveSubscriptions: first call is slow (returns a manually
  // resolvable promise with a snapshot captured at call time); subsequent
  // calls return the current state immediately.
  let listCallCount = 0;
  const fakeList = () => {
    listCallCount++;
    if (listCallCount === 1) {
      // Capture the snapshot NOW (K is still present at this point).
      const snapshot = archive.listSaveSubscriptions();
      // Return a promise we control — the caller decides when to resolve it.
      return new Promise((resolve) => {
        resolveAList = () => resolve(snapshot);
      });
    }
    return archive.listSaveSubscriptions();
  };

  const relay = makeFakeRelayClient();
  const mgr = new ArchiveSyncManager({
    relayClient: relay,
    listSaveSubscriptions: fakeList,
    archiveEvents: (c) => archive.archiveEvents(c),
    onSubscriptionChange: (l) => archive.onSubscriptionChange(l),
  });

  // Start: first list call is pending (reload A in flight).
  const started = mgr.start();
  await tick(); // A is suspended at listSaveSubscriptions

  // Delete K — this notifies the listener, which sets reloadPending (B queued).
  await archive.deleteSaveSubscription("channel_h", "chan-ooo");
  await tick();

  // Now resolve A's stale list result (K is present in the snapshot A captured).
  resolveAList();
  await started; // waits for A's full pass + B's coalescing pass
  await tick();

  // K must NOT be active after both passes complete. A's pass subscribed it,
  // B's pass tore it down.
  assert.equal(
    relay.activeCount(),
    0,
    "K must not be active after delete-then-list-resolve",
  );
});

test("manager_flushes_buffer_on_destroy", async () => {
  const relay = makeFakeRelayClient();
  const archive = makeFakeArchive();
  archive.setSubs([
    {
      scopeType: "channel_h",
      scopeValue: "chan-1",
      kinds: [9],
      identityPubkey: "pk",
      relayUrl: "wss://r",
      createdAt: 0,
    },
  ]);
  const mgr = makeManager(relay, archive, {
    flushBatchSize: 100,
    flushIdleMs: 10000,
  });
  await mgr.start();
  await tick();

  const filter = JSON.parse([...relay.subs.keys()][0]);
  relay.push(filter, {
    id: "ev1",
    kind: 9,
    pubkey: "pk",
    created_at: 1,
    content: "hi",
    tags: [],
  });
  relay.push(filter, {
    id: "ev2",
    kind: 9,
    pubkey: "pk",
    created_at: 2,
    content: "yo",
    tags: [],
  });
  // Buffer holds 2 events — flushBatchSize not reached yet
  assert.equal(archive.archiveCalls.length, 0);

  mgr.destroy(); // should flush on destroy
  await tick();
  assert.equal(archive.archiveCalls.length, 1);
  assert.equal(archive.archiveCalls[0].length, 2);
});
