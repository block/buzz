// Authoritative merge-lane storage suite — runs directly against channelStarsStorage.ts.
// Covers: parsePayload contract, mergeStores algebra, boundStore, idsFromStore,
// storageKey, readStore/writeStore, and the full claimLegacy state machine.

import assert from "node:assert/strict";
import test from "node:test";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import {
  boundStarStore,
  DEFAULT_STORE,
  MAX_CHANNEL_STAR_ENTRIES,
  mergeStores,
  parseStarPayload,
  readChannelStarsStore,
  starredChannelIdsFromStore,
  storageKey,
  writeChannelStarsStore,
} from "./channelStarsStorage.ts";

if (typeof globalThis.window === "undefined") {
  const storage = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => storage.get(key) ?? null,
      setItem: (key, value) => storage.set(key, value),
      removeItem: (key) => storage.delete(key),
    },
  };
}

const storageKeyPrefix = "buzz-channel-stars.v1";
const MAX_ENTRIES = MAX_CHANNEL_STAR_ENTRIES;
const E = (v, updatedAt, rev) => ({ starred: v, updatedAt, rev });
const S = (entry) => ({ version: 1, channels: { c: entry } });
function makeStore(channels = {}) {
  return { version: 1, channels };
}

test("parseStarPayload: valid payload round-trips; rev normalization; invalid entries filtered", () => {
  const payload = {
    version: 1,
    channels: { "chan-1": E(true, 1000, 3), "chan-2": E(false, 2000, 0) },
  };
  assert.deepEqual(parseStarPayload(payload), payload);
  assert.deepEqual(
    parseStarPayload({
      version: 1,
      channels: { c: { starred: true, updatedAt: 1000 } },
    }).channels.c,
    E(true, 1000, 0),
    "missing rev normalizes to 0",
  );
  const badRevResult = parseStarPayload({
    version: 1,
    channels: {
      str: { starred: true, updatedAt: 1, rev: "5" },
      neg: { starred: true, updatedAt: 1, rev: -2 },
      frac: { starred: true, updatedAt: 1, rev: 1.5 },
      nan: { starred: true, updatedAt: 1, rev: NaN },
      huge: { starred: true, updatedAt: 1, rev: Number.MAX_SAFE_INTEGER + 1 },
    },
  });
  for (const id of ["str", "neg", "frac", "nan", "huge"])
    assert.equal(badRevResult.channels[id].rev, 0);
  assert.deepEqual(
    parseStarPayload({
      version: 1,
      channels: {
        "no-starred": { updatedAt: 1000 },
        "no-updated-at": { starred: true },
        valid: { starred: false, updatedAt: 500 },
        "value-wrong-type": { starred: "yes", updatedAt: 1000 },
        "updated-at-wrong-type": { starred: true, updatedAt: "now" },
        null: null,
      },
    }),
    makeStore({ valid: E(false, 500, 0) }),
  );
  assert.deepEqual(
    parseStarPayload({
      version: 1,
      channels: {
        nan: { starred: true, updatedAt: NaN },
        inf: { starred: true, updatedAt: Infinity },
        neg: { starred: true, updatedAt: -1 },
        unsafe: { starred: true, updatedAt: Number.MAX_SAFE_INTEGER + 1 },
        valid: { starred: true, updatedAt: 100, rev: 2 },
      },
    }),
    makeStore({ valid: E(true, 100, 2) }),
  );
  assert.deepEqual(parseStarPayload({ version: 1, channels: {} }), makeStore());
  assert.deepEqual(parseStarPayload({ version: 1 }), makeStore());
  for (const input of [
    { channels: { c: E(true, 1, 0) } },
    { version: 2, channels: {} },
    null,
    "string",
    42,
  ])
    assert.equal(
      parseStarPayload(input),
      null,
      "invalid payload must return null",
    );
});

test("mergeStores: union, winner selection, same-second old-build compat", () => {
  assert.deepEqual(
    mergeStores(
      makeStore({ a: E(true, 100, 1) }),
      makeStore({ b: E(false, 200, 1) }),
    ),
    makeStore({ a: E(true, 100, 1), b: E(false, 200, 1) }),
    "non-overlapping returns union",
  );
  assert.deepEqual(
    mergeStores(S(E(false, 200, 0)), S(E(true, 100, 7))).channels.c,
    E(false, 200, 0),
    "later updatedAt wins",
  );
  assert.deepEqual(
    mergeStores(S(E(false, 100, 5)), S(E(true, 100, 2))).channels.c,
    E(false, 100, 5),
    "higher rev tiebreaks",
  );
  assert.deepEqual(
    mergeStores(S(E(false, 100, 3)), S(E(true, 100, 3))).channels.c,
    E(true, 100, 3),
    "true wins equal tie",
  );
  assert.deepEqual(
    mergeStores(S(E(true, 100, 9)), S(E(false, 999, 1))).channels.c,
    E(false, 999, 1),
    "false beats true at later ts",
  );
  assert.deepEqual(
    mergeStores(makeStore(), S(E(true, 42, 1))).channels.c,
    E(true, 42, 1),
    "remote-only entry",
  );
  assert.deepEqual(
    mergeStores(S(E(false, 10, 2)), makeStore()).channels.c,
    E(false, 10, 2),
    "local-only entry",
  );
  assert.deepEqual(
    mergeStores(makeStore(), makeStore()),
    makeStore(),
    "both empty",
  );
  assert.deepEqual(
    mergeStores(S(E(true, 100, 2)), S(E(false, 100, 0))).channels.c,
    E(true, 100, 2),
    "earlier new-build rev-2 true wins same-second tie",
  );
  assert.deepEqual(
    mergeStores(S(E(true, 100, 2)), S(E(false, 101, 0))).channels.c,
    E(false, 101, 0),
    "strictly-later-second old-build false wins — residual is transient",
  );
});

test("mergeStores: boundary (MAX_SAFE_INTEGER) rev cannot wedge later same-second toggles", () => {
  const wedged = parseStarPayload({
    version: 1,
    channels: {
      c: { starred: true, updatedAt: 100, rev: Number.MAX_SAFE_INTEGER },
    },
  });
  assert.equal(wedged.channels.c.rev, 0, "boundary rev normalized to 0");
  const mint = (store, v) => {
    const rev = Math.max(store.channels.c?.rev ?? 0, 0) + 1;
    return { store: S(E(v, 100, rev)), rev };
  };
  let state = wedged;
  let prevRev = 0;
  for (const v of [false, true, false, true]) {
    const { store: click, rev } = mint(state, v);
    assert.ok(rev > prevRev, `mint rev ${rev} advances past ${prevRev}`);
    state = mergeStores(state, click);
    assert.deepEqual(
      state.channels.c,
      E(v, 100, rev),
      `toggle to ${v} (rev ${rev}) wins`,
    );
    prevRev = rev;
  }
});

function randEntry(rng) {
  return E(rng() > 0.5, Math.floor(rng() * 5), Math.floor(rng() * 5));
}
function randStore(rng, ids) {
  const channels = {};
  for (const id of ids) if (rng() > 0.3) channels[id] = randEntry(rng);
  return makeStore(channels);
}
function lcg(seed) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

for (const [title, check, seed] of [
  [
    "commutative — merge(a,b) === merge(b,a)",
    (a, b) => assert.deepEqual(mergeStores(a, b), mergeStores(b, a)),
    12345,
  ],
  [
    "associative — merge(merge(a,b),c) === merge(a,merge(b,c))",
    (a, b, c) =>
      assert.deepEqual(
        mergeStores(mergeStores(a, b), c),
        mergeStores(a, mergeStores(b, c)),
      ),
    67890,
  ],
  [
    "idempotent — merge(a, merge(a,b)) === merge(a,b)",
    (a, b) => {
      const ab = mergeStores(a, b);
      assert.deepEqual(mergeStores(a, ab), ab);
      assert.deepEqual(mergeStores(ab, ab), ab);
    },
    24680,
  ],
]) {
  test(`mergeStores: ${title}`, () => {
    const rng = lcg(seed);
    const ids = ["a", "b", "c", "d"];
    for (let i = 0; i < 200; i++)
      check(randStore(rng, ids), randStore(rng, ids), randStore(rng, ids));
  });
}

test("v1 compat: rev-carrying blob round-trips; old-build unstar (no rev, updatedAt+1) beats stale star", () => {
  const roundTripped = parseStarPayload(
    JSON.parse(JSON.stringify(makeStore({ c: E(true, 100, 7) }))),
  );
  assert.equal(roundTripped.channels.c.starred, true);
  assert.equal(roundTripped.channels.c.updatedAt, 100);
  assert.deepEqual(
    mergeStores(
      S(E(true, 100, 7)),
      parseStarPayload({
        version: 1,
        channels: { c: { starred: false, updatedAt: 101 } },
      }),
    ).channels.c,
    E(false, 101, 0),
  );
});

test("boundStarStore: newest retained; ID tiebreak; preserves pinned mutation key", () => {
  const ch1 = Object.fromEntries(
    Array.from({ length: MAX_ENTRIES }, (_, i) => [
      `active-${i}`,
      E(true, i + 1, 0),
    ]),
  );
  ch1["old-false"] = E(false, 0, 0);
  ch1["new-false"] = E(false, 9999, 0);
  const r1 = boundStarStore(makeStore(ch1));
  assert.equal(Object.keys(r1.channels).length, MAX_ENTRIES);
  assert.equal(r1.channels["old-false"], undefined);
  assert.deepEqual(r1.channels["new-false"], E(false, 9999, 0));
  const ch2 = Object.fromEntries(
    Array.from({ length: MAX_ENTRIES + 1 }, (_, i) => [
      `channel-${String(MAX_ENTRIES - i).padStart(3, "0")}`,
      E(true, 1, 0),
    ]),
  );
  const r2 = boundStarStore(makeStore(ch2));
  assert.equal(r2.channels["channel-000"], undefined);
  assert.deepEqual(r2.channels["channel-500"], E(true, 1, 0));
  const ch3 = Object.fromEntries(
    Array.from({ length: MAX_ENTRIES }, (_, i) => [
      `z-channel-${String(i).padStart(3, "0")}`,
      E(true, 1, 0),
    ]),
  );
  ch3["a-target"] = E(false, 1, 1);
  const r3 = boundStarStore(makeStore(ch3), "a-target");
  assert.equal(Object.keys(r3.channels).length, MAX_ENTRIES);
  assert.deepEqual(r3.channels["a-target"], E(false, 1, 1));
  assert.equal(r3.channels["z-channel-000"], undefined);
});

test("mergeStores + boundStarStore: at-capacity unstar wins, evicted ID re-enters, id-tiebreak eviction", () => {
  const channels = Object.fromEntries(
    Array.from({ length: MAX_ENTRIES }, (_, i) => [
      `active-${i}`,
      E(true, i + 1, 0),
    ]),
  );
  channels.toggled = E(false, 9999, 1);
  assert.deepEqual(
    mergeStores(
      boundStarStore(makeStore(channels)),
      makeStore({ toggled: E(true, 9998, 5) }),
    ).channels.toggled,
    E(false, 9999, 1),
  );
  const localChannels = Object.fromEntries(
    Array.from({ length: MAX_ENTRIES }, (_, i) => [
      `active-${i}`,
      E(true, i + 10, 0),
    ]),
  );
  const result = mergeStores(
    makeStore(localChannels),
    makeStore({
      "evicted-id": E(true, 9999, 0),
      "active-0": E(false, 9998, 0),
    }),
  );
  assert.equal(Object.keys(result.channels).length, MAX_ENTRIES);
  assert.deepEqual(result.channels["evicted-id"], E(true, 9999, 0));
  assert.deepEqual(result.channels["active-0"], E(false, 9998, 0));
  assert.equal(result.channels["active-1"], undefined);
  const NOW = 777,
    TARGET = "aaa-target";
  const ch3 = { [TARGET]: E(true, NOW, 7) };
  for (let i = 0; i < MAX_ENTRIES; i++)
    ch3[`z-${String(i).padStart(3, "0")}`] = E(true, NOW, 0);
  assert.equal(
    boundStarStore(makeStore(ch3)).channels[TARGET],
    undefined,
    "TARGET evicted by id tiebreak",
  );
});

test("starredChannelIdsFromStore: returns starred IDs; all-false / empty returns empty set", () => {
  assert.deepEqual(
    [
      ...starredChannelIdsFromStore({
        version: 1,
        channels: {
          a: E(true, 100, 0),
          b: E(true, 200, 0),
          c: E(false, 300, 0),
        },
      }),
    ].sort(),
    ["a", "b"],
  );
  assert.equal(
    starredChannelIdsFromStore({ version: 1, channels: { x: E(false, 1, 0) } })
      .size,
    0,
  );
  assert.equal(
    starredChannelIdsFromStore({ version: 1, channels: {} }).size,
    0,
  );
});

test("readChannelStarsStore + writeChannelStarsStore: storageKey format, roundtrip, isolation, migration, precedence", () => {
  assert.equal(
    storageKey("pk1", "wss://relay.example.com"),
    `${storageKeyPrefix}:pk1:${encodeURIComponent(normalizeRelayUrl("wss://relay.example.com"))}`,
  );
  assert.equal(storageKey("pk1"), `${storageKeyPrefix}:pk1`);
  assert.notEqual(
    storageKey("pk1", "wss://relay-a.example.com"),
    storageKey("pk1", "wss://relay-b.example.com"),
  );
  assert.equal(
    storageKey("pk1", "WSS://Relay.Example/"),
    storageKey("pk1", "wss://relay.example"),
    "normalizeRelayUrl applied",
  );
  const store = makeStore({ chan1: E(true, 1000, 1) });
  assert.ok(
    writeChannelStarsStore("pk-rw-rt", store, "wss://relay.example.com") !==
      null,
  );
  assert.deepEqual(
    readChannelStarsStore("pk-rw-rt", "wss://relay.example.com"),
    store,
  );
  writeChannelStarsStore(
    "pk-rw-iso",
    makeStore({ cha: E(true, 100, 1) }),
    "wss://relay-a.example.com",
  );
  assert.deepEqual(
    readChannelStarsStore("pk-rw-iso", "wss://relay-b.example.com"),
    DEFAULT_STORE,
  );
  const legacy = makeStore({ chl: E(true, 500, 2) });
  writeChannelStarsStore("pk-rw-mig", legacy);
  assert.deepEqual(
    readChannelStarsStore("pk-rw-mig", "wss://relay-mig.example.com"),
    legacy,
  );
  assert.equal(
    window.localStorage.getItem(storageKey("pk-rw-mig")),
    null,
    "legacy key deleted",
  );
  assert.deepEqual(
    readChannelStarsStore("pk-rw-mig", "wss://relay-mig.example.com"),
    legacy,
    "idempotent",
  );
  writeChannelStarsStore("pk-rw-once", makeStore({ chm: E(true, 1, 1) }));
  readChannelStarsStore("pk-rw-once", "wss://relay-a.example.com");
  assert.deepEqual(
    readChannelStarsStore("pk-rw-once", "wss://relay-b.example.com"),
    DEFAULT_STORE,
    "one-time",
  );
  writeChannelStarsStore("pk-rw-empty", DEFAULT_STORE);
  assert.deepEqual(
    readChannelStarsStore("pk-rw-empty", "wss://relay-empty.example.com"),
    DEFAULT_STORE,
  );
  assert.notEqual(
    window.localStorage.getItem(storageKey("pk-rw-empty")),
    null,
    "empty not migrated",
  );
  const relay = "wss://relay-prec.example.com";
  writeChannelStarsStore("pk-rw-prec", makeStore({ old: E(true, 1, 1) }));
  const scoped = makeStore({ new: E(true, 2, 1) });
  writeChannelStarsStore("pk-rw-prec", scoped, relay);
  assert.deepEqual(readChannelStarsStore("pk-rw-prec", relay), scoped);
});

for (const {
  title,
  pubkey,
  relayA,
  relayB,
  setupFailure,
  assertions,
  assertionsAfterRestore,
} of [
  {
    title: "scoped-write failure returns DEFAULT; relay B still claims legacy",
    pubkey: "pk-stars-migrate-writefail",
    relayA: "wss://relay-a-writefail-stars.example.com",
    relayB: "wss://relay-b-writefail-stars.example.com",
    setupFailure: (ls, scopedA) => {
      const orig = ls.setItem;
      ls.setItem = (k, v) => {
        if (k === scopedA) throw new Error("QuotaExceededError");
        return orig.call(ls, k, v);
      };
      return () => {
        ls.setItem = orig;
      };
    },
    assertions: (pubkey, relayA, relayB, legacy) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.equal(
        window.localStorage.getItem(storageKey(pubkey, relayA)),
        null,
      );
      assert.notEqual(
        window.localStorage.getItem(storageKey(pubkey)),
        null,
        "legacy not yet deleted",
      );
      assert.deepEqual(readChannelStarsStore(pubkey, relayB), legacy);
      assert.equal(
        window.localStorage.getItem(storageKey(pubkey)),
        null,
        "relay B claims+deletes legacy",
      );
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
    },
  },
  {
    title: "legacy delete no-op rolls back and returns DEFAULT",
    pubkey: "pk-stars-migrate-delfail",
    relayA: "wss://relay-delfail-stars.example.com",
    setupFailure: (ls, _scopedA, legacyKey) => {
      const orig = ls.removeItem;
      ls.removeItem = (k) => {
        if (k !== legacyKey) return orig.call(ls, k);
      };
      return () => {
        ls.removeItem = orig;
      };
    },
    assertions: (pubkey, relayA) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.equal(
        window.localStorage.getItem(storageKey(pubkey, relayA)),
        null,
      );
    },
    assertionsAfterRestore: (pubkey, relayA, _relayB, legacy) => {
      assert.deepEqual(
        readChannelStarsStore(pubkey, relayA),
        legacy,
        "legacy claimable after storage recovers",
      );
      assert.equal(
        window.localStorage.getItem(storageKey(pubkey)),
        null,
        "legacy deleted by healthy claim",
      );
    },
  },
  {
    title:
      "legacy delete + rollback both throw — DEFAULT until storage recovers",
    pubkey: "pk-stars-migrate-delrollbackthrow",
    relayA: "wss://relay-delrollbackthrow-stars.example.com",
    setupFailure: (ls) => {
      const orig = ls.removeItem;
      ls.removeItem = () => {
        throw new Error("SecurityError");
      };
      return () => {
        ls.removeItem = orig;
      };
    },
    assertions: (pubkey, relayA, _relayB, _legacy, _scopedA, legacyKey) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.notEqual(
        window.localStorage.getItem(legacyKey),
        null,
        "legacy still present",
      );
    },
    assertionsAfterRestore: (pubkey, relayA, _relayB, legacy) => {
      assert.deepEqual(
        readChannelStarsStore(pubkey, relayA),
        legacy,
        "legacy claimable after storage recovers",
      );
    },
  },
  {
    title:
      "legacy delete succeeds; confirmation read throws — scoped copy retained, no data loss",
    pubkey: "pk-stars-migrate-confirmthrow",
    relayA: "wss://relay-confirmthrow-stars.example.com",
    setupFailure: (ls, _scopedA, legacyKey) => {
      const origGet = ls.getItem;
      const origRemove = ls.removeItem;
      let legacyDeleted = false;
      let threwOnce = false;
      ls.removeItem = (k) => {
        if (k === legacyKey) legacyDeleted = true;
        return origRemove.call(ls, k);
      };
      ls.getItem = (k) => {
        if (k === legacyKey && legacyDeleted && !threwOnce) {
          threwOnce = true;
          throw new Error("SecurityError");
        }
        return origGet.call(ls, k);
      };
      return () => {
        ls.getItem = origGet;
        ls.removeItem = origRemove;
      };
    },
    assertions: (pubkey, relayA, _relayB, legacy, scopedA) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.notEqual(window.localStorage.getItem(scopedA), null);
      assert.equal(window.localStorage.getItem(storageKey(pubkey)), null);
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), legacy);
    },
  },
  {
    title:
      "legacy delete throws while probe stays healthy — rollback, DEFAULT, single future claimant",
    pubkey: "pk-stars-migrate-delthrow-probeok",
    relayA: "wss://relay-delthrow-probeok-stars.example.com",
    setupFailure: (ls, _scopedA, legacyKey) => {
      const orig = ls.removeItem;
      let thrown = false;
      ls.removeItem = (k) => {
        if (k === legacyKey && !thrown) {
          thrown = true;
          throw new Error("SecurityError");
        }
        return orig.call(ls, k);
      };
      return () => {
        ls.removeItem = orig;
      };
    },
    assertions: (pubkey, relayA, _relayB, legacy, scopedA, legacyKey) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.equal(window.localStorage.getItem(scopedA), null);
      assert.notEqual(window.localStorage.getItem(legacyKey), null);
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), legacy);
      assert.equal(window.localStorage.getItem(legacyKey), null);
    },
  },
  {
    title:
      "delete and probe both throw — scoped kept but hidden while legacy remains",
    pubkey: "pk-stars-migrate-delthrow-probethrow",
    relayA: "wss://relay-delthrow-probethrow-stars.example.com",
    setupFailure: (ls, _scopedA, legacyKey) => {
      const origGet = ls.getItem;
      const origRemove = ls.removeItem;
      let removeAttempted = false;
      ls.removeItem = (k) => {
        if (k === legacyKey) {
          removeAttempted = true;
          throw new Error("SecurityError");
        }
        return origRemove.call(ls, k);
      };
      ls.getItem = (k) => {
        if (k === legacyKey && removeAttempted)
          throw new Error("SecurityError");
        return origGet.call(ls, k);
      };
      return () => {
        ls.getItem = origGet;
        ls.removeItem = origRemove;
      };
    },
    assertions: (pubkey, relayA, _relayB, _legacy, scopedA) => {
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
      assert.notEqual(window.localStorage.getItem(scopedA), null);
      assert.deepEqual(readChannelStarsStore(pubkey, relayA), DEFAULT_STORE);
    },
    assertionsAfterRestore: (
      pubkey,
      relayA,
      _relayB,
      legacy,
      _scopedA,
      legacyKey,
    ) => {
      assert.deepEqual(
        readChannelStarsStore(pubkey, relayA),
        legacy,
        "legacy claimable after storage recovers",
      );
      assert.equal(window.localStorage.getItem(legacyKey), null);
    },
  },
]) {
  test(`claimLegacy: ${title}`, () => {
    const legacy = makeStore({ ch: E(true, 500, 2) });
    writeChannelStarsStore(pubkey, legacy);
    const scopedA = storageKey(pubkey, relayA);
    const legacyKey = storageKey(pubkey);
    const restore = setupFailure(window.localStorage, scopedA, legacyKey);
    try {
      assertions(pubkey, relayA, relayB, legacy, scopedA, legacyKey);
    } finally {
      restore();
    }
    assertionsAfterRestore?.(
      pubkey,
      relayA,
      relayB,
      legacy,
      scopedA,
      legacyKey,
    );
  });
}
