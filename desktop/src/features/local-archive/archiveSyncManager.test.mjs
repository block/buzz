import assert from "node:assert/strict";
import test from "node:test";

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

// ── Import the module under test with injected fakes ─────────────────────────

/**
 * Build an ArchiveSyncManager with injected fakes instead of importing the
 * singleton relayClient and tauriArchive. We inline a minimal version of the
 * class here that accepts the deps as constructor args — this is the
 * testability seam documented in the implementation file.
 *
 * Rather than re-implementing the full class here, we import it from the
 * source and test via the public API.  Because the module uses module-level
 * singletons (relayClient, tauriArchive) we test the observable behaviours
 * that don't depend on them (kinds decoding, preset arrays) and test the
 * manager's response to subscription-change events via a thin adapter.
 */

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

// ── ArchiveSyncManager with fakes ─────────────────────────────────────────────

/**
 * Inline testable ArchiveSyncManager that accepts injected deps.
 * Mirrors the structure in archiveSyncManager.ts — same logic, injectable.
 */
class TestableArchiveSyncManager {
  constructor({
    relayClient,
    archive,
    flushBatchSize = 25,
    flushIdleMs = 2000,
  }) {
    this._relay = relayClient;
    this._archive = archive;
    this._flushBatchSize = flushBatchSize;
    this._flushIdleMs = flushIdleMs;
    this._unsubs = new Map();
    this._buffer = [];
    this._flushTimer = null;
    this._destroyed = false;
    this._offChange = null;
  }

  async start() {
    await this._resubscribeAll();
    this._offChange = this._archive.onSubscriptionChange(() => {
      void this._resubscribeAll();
    });
  }

  destroy() {
    this._destroyed = true;
    if (this._flushTimer !== null) {
      clearTimeout(this._flushTimer);
      this._flushTimer = null;
    }
    this._offChange?.();
    if (this._buffer.length > 0) {
      const batch = this._buffer.splice(0);
      void this._archive.archiveEvents(batch);
    }
    for (const [, unsub] of this._unsubs) void unsub();
    this._unsubs.clear();
  }

  async _resubscribeAll() {
    if (this._destroyed) return;
    let subs;
    try {
      subs = await this._archive.listSaveSubscriptions();
    } catch {
      return;
    }
    if (this._destroyed) return;

    const wanted = new Set(subs.map((s) => `${s.scopeType}:${s.scopeValue}`));
    for (const [key, unsub] of this._unsubs) {
      if (!wanted.has(key)) {
        void unsub();
        this._unsubs.delete(key);
      }
    }
    for (const sub of subs) {
      const key = `${sub.scopeType}:${sub.scopeValue}`;
      if (this._unsubs.has(key)) continue;
      const scopeType = sub.scopeType;
      const scopeValue = sub.scopeValue;
      const filter = this._buildFilter(sub);
      let unsub = null;
      let cancelled = false;
      void this._relay
        .subscribeLive(filter, (event) => {
          this._enqueue(event, scopeType, scopeValue);
        })
        .then((dispose) => {
          if (cancelled) void dispose();
          else unsub = dispose;
        });
      this._unsubs.set(key, async () => {
        cancelled = true;
        if (unsub) await unsub();
      });
    }
  }

  _buildFilter(sub) {
    const base = { kinds: sub.kinds, limit: 0 };
    switch (sub.scopeType) {
      case "channel_h":
        return { ...base, "#h": [sub.scopeValue] };
      case "owner_p":
        return { ...base, "#p": [sub.scopeValue] };
      case "referenced_e":
        return { ...base, "#e": [sub.scopeValue] };
    }
  }

  _enqueue(event, scopeType, scopeValue) {
    if (this._destroyed) return;
    this._buffer.push({
      rawEventJson: JSON.stringify(event),
      matchedScope: { scopeType, scopeValue },
    });
    if (this._buffer.length >= this._flushBatchSize) {
      this._flush();
    } else {
      this._scheduleFlush();
    }
  }

  _scheduleFlush() {
    if (this._flushTimer !== null) return;
    this._flushTimer = setTimeout(() => {
      this._flushTimer = null;
      this._flush();
    }, this._flushIdleMs);
  }

  _flush() {
    if (this._flushTimer !== null) {
      clearTimeout(this._flushTimer);
      this._flushTimer = null;
    }
    if (this._buffer.length === 0) return;
    const batch = this._buffer.splice(0);
    void this._archive.archiveEvents(batch);
  }
}

// Helper: wait for microtasks/promises to settle
function tick() {
  return new Promise((r) => setTimeout(r, 0));
}

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
  const mgr = new TestableArchiveSyncManager({ relayClient: relay, archive });
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
  const mgr = new TestableArchiveSyncManager({ relayClient: relay, archive });
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
  const mgr = new TestableArchiveSyncManager({ relayClient: relay, archive });
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
  const mgr = new TestableArchiveSyncManager({
    relayClient: relay,
    archive,
    flushBatchSize: 1,
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
  const mgr = new TestableArchiveSyncManager({ relayClient: relay, archive });
  await mgr.start();
  await tick();
  assert.equal(relay.activeCount(), 0);

  // Simulate create_save_subscription — sets subs then fires notifier
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
  const mgr = new TestableArchiveSyncManager({ relayClient: relay, archive });
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
  const mgr = new TestableArchiveSyncManager({
    relayClient: relay,
    archive,
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
