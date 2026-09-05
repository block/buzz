import assert from "node:assert/strict";
import test from "node:test";

// Multi-window durable-outbox safety. Outbox keys are per-window write-once
// (<prefix>:<pubkey>:<relay>:<nonce>:<seq>). Merge lanes fold all records;
// whole-blob lanes replay max-queuedAt. Supersession is STRICT (queuedAt <
// head.created_at) so same-second records are never dropped. Legacy v1 shared
// key is never deleted by v2. Each scenario is deterministic (mock Storage,
// no relay/timers). Manager/hook-level behavior is covered in their own suites.

function makeStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
    clear: () => map.clear(),
    get length() {
      return map.size;
    },
    key: (i) => [...map.keys()][i] ?? null,
    has: (k) => map.has(k),
  };
}

function withStorage(fn) {
  const ls = makeStorage(),
    ss = makeStorage();
  const priorWindow = globalThis.window;
  globalThis.window = {
    ...(priorWindow ?? {}),
    localStorage: ls,
    sessionStorage: ss,
  };
  try {
    return fn(ls);
  } finally {
    if (priorWindow !== undefined) globalThis.window = priorWindow;
    else delete globalThis.window;
  }
}

const { normalizeRelayUrl } = await import("@/shared/lib/normalizeRelayUrl");
const { outboxWindowNonce } = await import("./sidebarSyncWatermark.ts");
const stars = await import("./channelStarsStorage.ts");
const sort = await import("./channelSortPreference.ts");
const sections = await import("./channelSectionsStorage.ts");

const PK = "pk";
const RELAY = "wss://relay.example.com";
const SCOPE = `${PK}:${encodeURIComponent(normalizeRelayUrl(RELAY))}`;

const PREFIX = {
  stars: "buzz-channel-stars-outbox.v1",
  sort: "buzz-channel-sort-outbox.v1",
  sections: "buzz-channel-sections-outbox.v1",
};

const SEQ_WIDTH = 12;
const pad = (seq) => String(seq).padStart(SEQ_WIDTH, "0");
const foreignKey = (lane, nonce, seq) =>
  `${PREFIX[lane]}:${SCOPE}:${nonce}:${pad(seq)}`;
const legacyKey = (lane) => `${PREFIX[lane]}:${SCOPE}`;
const writeAt = (ls, key, store, queuedAt) =>
  ls.setItem(key, JSON.stringify({ store, queuedAt }));

const starStore = (channels) => ({ version: 1, channels });
const starEntry = (starred, updatedAt, rev) => ({ starred, updatedAt, rev });
const sortStore = (groups) => ({ version: 1, groups });
const sectionStore = (secs, assignments = {}) => ({
  version: 1,
  sections: secs,
  assignments,
});

test("(i+iii+ix) reclaim deletes proven-stale key and keeps coexisting fresh edit; same-second kept; non-subsumed kept", () => {
  withStorage((ls) => {
    const stale = foreignKey("stars", "peerB", 0),
      fresh = foreignKey("stars", "peerB", 1);
    writeAt(ls, stale, starStore({ a: starEntry(true, 100, 1) }), 100);
    writeAt(ls, fresh, starStore({ z: starEntry(true, 300, 9) }), 300);
    stars.reclaimSubsumedStarsOutbox(
      PK,
      RELAY,
      starStore({ a: starEntry(true, 100, 1) }),
    );
    assert.ok(!ls.has(stale), "proven-stale key reclaimed");
    assert.ok(ls.has(fresh), "fresh edit survives");
    assert.deepEqual(
      stars.readChannelStarsOutboxWithMeta(PK, RELAY).store.channels.z,
      starEntry(true, 300, 9),
    );
  });
  withStorage((ls) => {
    const stale = foreignKey("sort", "peerB", 0),
      fresh = foreignKey("sort", "peerB", 1);
    writeAt(ls, stale, sortStore({ dms: "alpha" }), 100);
    writeAt(ls, fresh, sortStore({ dms: "recent" }), 500);
    sort.reclaimSupersededSortOutbox(PK, RELAY, 200);
    assert.ok(!ls.has(stale), "proven-stale key reclaimed");
    assert.ok(ls.has(fresh), "fresh edit survives");
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY).store.groups.dms,
      "recent",
    );
  });
  withStorage((ls) => {
    writeAt(
      ls,
      foreignKey("sort", "same", 0),
      sortStore({ dms: "recent" }),
      100,
    );
    writeAt(ls, foreignKey("sort", "old", 0), sortStore({ dms: "alpha" }), 99);
    sort.reclaimSupersededSortOutbox(PK, RELAY, 100);
    assert.ok(
      ls.has(foreignKey("sort", "same", 0)),
      "same-second record (queuedAt == head) kept",
    );
    assert.ok(
      !ls.has(foreignKey("sort", "old", 0)),
      "strictly-earlier record reclaimed",
    );
  });
  withStorage((ls) => {
    const key = foreignKey("stars", "B", 0);
    writeAt(ls, key, starStore({ a: starEntry(true, 300, 2) }), 300);
    stars.reclaimSubsumedStarsOutbox(PK, RELAY, starStore({}));
    assert.ok(ls.has(key), "unsubsumed foreign edit is kept");
  });
});

test("(ii) merge-lane and whole-blob resume across window teardown/remount", () =>
  withStorage((ls) => {
    writeAt(
      ls,
      foreignKey("stars", "A", 0),
      starStore({ a: starEntry(true, 100, 1) }),
      100,
    );
    writeAt(
      ls,
      foreignKey("stars", "B", 0),
      starStore({ b: starEntry(true, 200, 1) }),
      200,
    );
    const resumed = stars.readChannelStarsOutboxWithMeta(PK, RELAY);
    assert.deepEqual(resumed.store.channels.a, starEntry(true, 100, 1));
    assert.deepEqual(resumed.store.channels.b, starEntry(true, 200, 1));
    writeAt(
      ls,
      foreignKey("sort", "A", 0),
      sortStore({ channels: "alpha" }),
      100,
    );
    writeAt(
      ls,
      foreignKey("sort", "B", 0),
      sortStore({ channels: "recent" }),
      200,
    );
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY).store.groups.channels,
      "recent",
    );
  }));

test("(iv) stars: legacy shared key resumes and is never reclaimed", () =>
  withStorage((ls) => {
    ls.setItem(
      legacyKey("stars"),
      JSON.stringify(starStore({ a: starEntry(true, 100, 1) })),
    );
    assert.deepEqual(
      stars.readChannelStarsOutboxWithMeta(PK, RELAY).store.channels.a,
      starEntry(true, 100, 1),
      "legacy entry resumes",
    );
    stars.reclaimSubsumedStarsOutbox(
      PK,
      RELAY,
      starStore({ a: starEntry(true, 100, 1) }),
    );
    assert.ok(
      ls.has(legacyKey("stars")),
      "legacy v1 key is never deleted by v2",
    );
  }));

test("(v) crash between write-new/delete-old: merge-lane coalesces, whole-blob resumes newer seq", () => {
  withStorage((ls) => {
    const base = `${PREFIX.stars}:${SCOPE}:${outboxWindowNonce()}`;
    writeAt(
      ls,
      `${base}:${pad(0)}`,
      starStore({ a: starEntry(true, 100, 1) }),
      100,
    );
    writeAt(
      ls,
      `${base}:${pad(1)}`,
      starStore({ b: starEntry(true, 200, 1) }),
      200,
    );
    const resumed = stars.readChannelStarsOutboxWithMeta(PK, RELAY);
    assert.deepEqual(resumed.store.channels.a, starEntry(true, 100, 1));
    assert.deepEqual(resumed.store.channels.b, starEntry(true, 200, 1));
  });
  withStorage((ls) => {
    const base = `${PREFIX.sort}:${SCOPE}:${outboxWindowNonce()}`;
    writeAt(ls, `${base}:${pad(9)}`, sortStore({ dms: "alpha" }), 100);
    writeAt(ls, `${base}:${pad(10)}`, sortStore({ dms: "recent" }), 100);
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY).store.groups.dms,
      "recent",
      "newer seq resumes",
    );
  });
});

test("(vi-viii) reload seq above surviving keys; whole-blob key tiebreak; merge-lane order-independent", () => {
  withStorage((ls) => {
    const base = `${PREFIX.stars}:${SCOPE}:${outboxWindowNonce()}`;
    writeAt(
      ls,
      `${base}:${pad(5)}`,
      starStore({ a: starEntry(true, 50, 1) }),
      50,
    );
    stars.writeChannelStarsOutbox(
      PK,
      starStore({ b: starEntry(true, 100, 1) }),
      RELAY,
    );
    const ownKeys = [];
    for (let i = 0; i < ls.length; i++) {
      const k = ls.key(i);
      if (k?.startsWith(`${base}:`)) ownKeys.push(k);
    }
    assert.equal(ownKeys.length, 1, "delete-old leaves exactly one own key");
    assert.ok(
      ownKeys[0] > `${base}:${pad(5)}`,
      "new key strictly above surviving",
    );
    assert.deepEqual(
      stars.readChannelStarsOutboxWithMeta(PK, RELAY).store.channels.b,
      starEntry(true, 100, 1),
    );
  });
  withStorage((ls) => {
    writeAt(
      ls,
      foreignKey("sort", "aaa", 0),
      sortStore({ forums: "alpha" }),
      100,
    );
    writeAt(
      ls,
      foreignKey("sort", "zzz", 0),
      sortStore({ forums: "recent" }),
      100,
    );
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY).store.groups.forums,
      "recent",
    );
  });
  withStorage((ls) => {
    writeAt(
      ls,
      foreignKey("stars", "aaa", 0),
      starStore({ c: starEntry(true, 100, 5) }),
      100,
    );
    writeAt(
      ls,
      foreignKey("stars", "zzz", 0),
      starStore({ c: starEntry(false, 100, 2) }),
      200,
    );
    assert.deepEqual(
      stars.readChannelStarsOutboxWithMeta(PK, RELAY).store.channels.c,
      starEntry(true, 100, 5),
      "higher rev wins",
    );
  });
});

test("(x) legacy outbox: replay-once, rewritten-blob replay, crash-recovery, and subsumption", () => {
  withStorage((ls) => {
    ls.setItem(legacyKey("sort"), JSON.stringify(sortStore({ dms: "recent" })));
    const boot1 = sort.readChannelSortOutbox(PK, RELAY);
    assert.equal(boot1.store.groups.dms, "recent", "legacy blob resumes");
    assert.equal(
      boot1.legacyRawToConsume,
      JSON.stringify(sortStore({ dms: "recent" })),
    );
    sort.markChannelSortLegacyConsumed(PK, RELAY, boot1.legacyRawToConsume);
    assert.ok(ls.has(legacyKey("sort")), "legacy key never deleted");
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY),
      null,
      "consumed blob not replayed again",
    );
    assert.equal(
      sort.readChannelSortOutbox(PK, RELAY),
      null,
      "still skipped on third boot",
    );
  });
  withStorage((ls) => {
    writeAt(
      ls,
      legacyKey("sections"),
      sectionStore([{ id: "s1", name: "One", order: 0 }]),
      0,
    );
    const boot1 = sections.readChannelSectionsOutbox(PK, RELAY);
    assert.deepEqual(boot1.store.sections, [
      { id: "s1", name: "One", order: 0 },
    ]);
    sections.markChannelSectionsLegacyConsumed(
      PK,
      RELAY,
      boot1.legacyRawToConsume,
    );
    assert.equal(
      sections.readChannelSectionsOutbox(PK, RELAY),
      null,
      "consumed blob skipped",
    );
    writeAt(
      ls,
      legacyKey("sections"),
      sectionStore([{ id: "s2", name: "Two", order: 0 }]),
      0,
    );
    const boot3 = sections.readChannelSectionsOutbox(PK, RELAY);
    assert.deepEqual(
      boot3.store.sections,
      [{ id: "s2", name: "Two", order: 0 }],
      "changed legacy value replayed",
    );
    assert.ok(boot3.legacyRawToConsume !== null);
  });
  withStorage((ls) => {
    ls.setItem(legacyKey("sort"), JSON.stringify(sortStore({ dms: "recent" })));
    const boot1 = sort.readChannelSortOutbox(PK, RELAY);
    sort.writeChannelSortOutbox(PK, boot1.store, RELAY);
    const boot2 = sort.readChannelSortOutbox(PK, RELAY);
    assert.equal(boot2.store.groups.dms, "recent", "intent survives in v2 key");
    assert.equal(
      boot2.legacyRawToConsume,
      null,
      "winner is the v2 key, not legacy",
    );
  });
  withStorage((ls) => {
    ls.setItem(
      legacyKey("stars"),
      JSON.stringify(starStore({ a: starEntry(true, 100, 1) })),
    );
    const outboxMeta = stars.readChannelStarsOutboxWithMeta(PK, RELAY);
    assert.ok(
      stars.isStarsStoreSubsumedBy(
        outboxMeta.store,
        starStore({ a: starEntry(true, 100, 1) }),
      ),
      "head-subsumed legacy fold is publish-free",
    );
    assert.ok(
      !stars.isStarsStoreSubsumedBy(outboxMeta.store, starStore({})),
      "unsubsumed legacy click still needs a publish",
    );
  });
});

test("own key: write resumes, clear removes only this window's own keys", () =>
  withStorage((ls) => {
    stars.writeChannelStarsOutbox(
      PK,
      starStore({ a: starEntry(true, 100, 1) }),
      RELAY,
    );
    const ownBase = `${PREFIX.stars}:${SCOPE}:${outboxWindowNonce()}`;
    const hasOwn = () => {
      for (let i = 0; i < ls.length; i++) {
        const k = ls.key(i);
        if (k?.startsWith(`${ownBase}:`)) return true;
      }
      return false;
    };
    assert.ok(hasOwn(), "own edit written");
    writeAt(
      ls,
      foreignKey("stars", "peer", 0),
      starStore({ z: starEntry(true, 9, 1) }),
      9,
    );
    stars.clearChannelStarsOutbox(PK, RELAY);
    assert.ok(!hasOwn(), "own keys cleared");
    assert.ok(ls.has(foreignKey("stars", "peer", 0)), "foreign key untouched");
  }));
