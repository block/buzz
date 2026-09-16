import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_SELF_PROFILE_CACHES,
  MAX_SELF_PROFILE_CACHES_PER_RELAY,
  _resetProfileKeyCountForTest,
  parseSelfProfileCache,
  resolveAvatarDataUrl,
  shouldFetchAvatar,
  storageKey,
  writeSelfProfileCache,
} from "./selfProfileStorage.ts";

test("storageKey: includes pubkey in result", () => {
  const key = storageKey("https://relay.example.com", "deadbeef");
  assert.ok(
    key.includes("deadbeef"),
    `expected key to contain pubkey, got: ${key}`,
  );
});

test("storageKey: strips trailing slash from relay URL", () => {
  const withSlash = storageKey("https://relay.example.com/", "abc");
  const withoutSlash = storageKey("https://relay.example.com", "abc");
  assert.equal(withSlash, withoutSlash);
});

test("storageKey: strips multiple trailing slashes", () => {
  const key = storageKey("https://relay.example.com///", "abc");
  assert.ok(
    !key.includes("///"),
    `expected trailing slashes stripped, got: ${key}`,
  );
});

test("storageKey: lowercases relay URL", () => {
  const upper = storageKey("HTTPS://Relay.Example.COM", "abc");
  const lower = storageKey("https://relay.example.com", "abc");
  assert.equal(upper, lower);
});

test("storageKey: trims whitespace from relay URL", () => {
  const padded = storageKey("  https://relay.example.com  ", "abc");
  const clean = storageKey("https://relay.example.com", "abc");
  assert.equal(padded, clean);
});

test("storageKey: different pubkeys produce different keys", () => {
  const a = storageKey("https://relay.example.com", "pubkey-a");
  const b = storageKey("https://relay.example.com", "pubkey-b");
  assert.notEqual(a, b);
});

function installStorage(onGetItem = () => {}) {
  const values = new Map();
  globalThis.window = {
    dispatchEvent: () => true,
    localStorage: {
      get length() {
        return values.size;
      },
      getItem: (key) => {
        onGetItem(key);
        return values.get(key) ?? null;
      },
      key: (index) => [...values.keys()][index] ?? null,
      removeItem: (key) => values.delete(key),
      setItem: (key, value) => values.set(key, String(value)),
    },
  };
  globalThis.CustomEvent ??= class CustomEvent {};
  return values;
}

test("writeSelfProfileCache caps each relay and the global cache by updatedAt", () => {
  _resetProfileKeyCountForTest();
  const values = installStorage();
  const relayA = "wss://relay-a.example";
  for (let index = 0; index < MAX_SELF_PROFILE_CACHES_PER_RELAY + 1; index++) {
    assert.equal(
      writeSelfProfileCache(
        relayA,
        `pubkey-${index}`,
        makeCache({ updatedAt: index }),
      ),
      true,
    );
  }
  assert.equal(values.has(storageKey(relayA, "pubkey-0")), false);
  assert.equal(values.has(storageKey(relayA, "pubkey-8")), true);

  for (let index = 0; index < MAX_SELF_PROFILE_CACHES + 1; index++) {
    writeSelfProfileCache(
      `wss://relay-${index}.example`,
      `global-${index}`,
      makeCache({ updatedAt: index + 100 }),
    );
  }
  const profileKeys = [...values.keys()].filter((key) =>
    key.startsWith("buzz-self-profile.v1:"),
  );
  assert.equal(profileKeys.length, MAX_SELF_PROFILE_CACHES);
  assert.equal(values.has(storageKey(relayA, "pubkey-1")), false);
});

test("writeSelfProfileCache does not read existing payloads below both caps", () => {
  _resetProfileKeyCountForTest();
  const readKeys = [];
  const values = installStorage((key) => readKeys.push(key));
  const relay = "wss://relay.example";
  const existingKey = storageKey(relay, "existing");
  const writtenKey = storageKey(relay, "written");
  values.set(existingKey, JSON.stringify(makeCache({ updatedAt: 1 })));

  assert.equal(
    writeSelfProfileCache(relay, "written", makeCache({ updatedAt: 2 })),
    true,
  );

  assert.deepEqual(readKeys, [writtenKey]);
  assert.equal(values.has(existingKey), true);
  assert.equal(values.has(writtenKey), true);
});

test("parseSelfProfileCache: valid v1 payload round-trips", () => {
  const payload = {
    version: 1,
    displayName: "Alice",
    avatarUrl: "https://relay.example.com/media/abc.jpg",
    about: "Building better communities",
    avatarDataUrl: "data:image/jpeg;base64,/9j/4A==",
    updatedAt: 1700000000000,
  };
  const result = parseSelfProfileCache(payload);
  assert.deepEqual(result, payload);
});

test("parseSelfProfileCache: null fields are preserved", () => {
  const payload = {
    version: 1,
    displayName: null,
    avatarUrl: null,
    about: null,
    avatarDataUrl: null,
    updatedAt: 0,
  };
  const result = parseSelfProfileCache(payload);
  assert.deepEqual(result, payload);
});

test("parseSelfProfileCache: wrong version returns null", () => {
  assert.equal(
    parseSelfProfileCache({
      version: 2,
      displayName: "Bob",
      avatarUrl: null,
      avatarDataUrl: null,
      updatedAt: 0,
    }),
    null,
  );
});

test("parseSelfProfileCache: missing version returns null", () => {
  assert.equal(
    parseSelfProfileCache({
      displayName: "Bob",
      avatarUrl: null,
      avatarDataUrl: null,
      updatedAt: 0,
    }),
    null,
  );
});

test("parseSelfProfileCache: null input returns null", () => {
  assert.equal(parseSelfProfileCache(null), null);
});

test("parseSelfProfileCache: string input returns null", () => {
  assert.equal(parseSelfProfileCache("garbage"), null);
});

test("parseSelfProfileCache: array input returns null", () => {
  assert.equal(parseSelfProfileCache([1, 2, 3]), null);
});

test("parseSelfProfileCache: number input returns null", () => {
  assert.equal(parseSelfProfileCache(42), null);
});

test("parseSelfProfileCache: non-string displayName is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: 123,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 0,
  });
  assert.notEqual(result, null);
  assert.equal(result?.displayName, null);
});

test("parseSelfProfileCache: non-finite updatedAt is coerced to 0", () => {
  const resultNaN = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: NaN,
  });
  assert.equal(resultNaN?.updatedAt, 0);

  const resultInf = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: Infinity,
  });
  assert.equal(resultInf?.updatedAt, 0);
});

test("parseSelfProfileCache: non-number updatedAt is coerced to 0", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: "yesterday",
  });
  assert.equal(result?.updatedAt, 0);
});

test("parseSelfProfileCache: valid data:image/ avatarDataUrl is preserved", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "data:image/png;base64,iVBORw0KGgo=",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, "data:image/png;base64,iVBORw0KGgo=");
});

test("parseSelfProfileCache: javascript: avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "javascript:alert(1)",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

test("parseSelfProfileCache: bare base64 avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "iVBORw0KGgoAAAANSUhEUgAAAAUA",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

test("parseSelfProfileCache: data:text/html avatarDataUrl is coerced to null", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: "data:text/html,<h1>xss</h1>",
    updatedAt: 0,
  });
  assert.equal(result?.avatarDataUrl, null);
});

/** Minimal SelfProfileCache fixture for policy helper tests. */
function makeCache(overrides = {}) {
  return {
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 0,
    ...overrides,
  };
}

test("parseSelfProfileCache: hasProfileEvent true is preserved", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 1700000000000,
    hasProfileEvent: true,
  });
  assert.equal(result?.hasProfileEvent, true);
});

test("parseSelfProfileCache: hasProfileEvent false is omitted (conservative default)", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 1700000000000,
    hasProfileEvent: false,
  });
  assert.equal(result?.hasProfileEvent, undefined);
});

test("parseSelfProfileCache: absent hasProfileEvent field is omitted (legacy v1 migration)", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: "Alice",
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 1700000000000,
  });
  assert.equal(result?.hasProfileEvent, undefined);
});

test("parseSelfProfileCache: non-boolean hasProfileEvent is coerced to absent", () => {
  const result = parseSelfProfileCache({
    version: 1,
    displayName: null,
    avatarUrl: null,
    avatarDataUrl: null,
    updatedAt: 1700000000000,
    hasProfileEvent: "yes",
  });
  assert.equal(result?.hasProfileEvent, undefined);
});

test("shouldFetchAvatar: URL changed → fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/new.jpg", existing),
    true,
  );
});

test("shouldFetchAvatar: URL unchanged but avatarDataUrl null → fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: null,
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/same.jpg", existing),
    true,
  );
});

test("shouldFetchAvatar: URL unchanged and avatarDataUrl present → no fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/existing",
  });
  assert.equal(
    shouldFetchAvatar("https://relay.example.com/same.jpg", existing),
    false,
  );
});

test("shouldFetchAvatar: nextAvatarUrl null → no fetch", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(shouldFetchAvatar(null, existing), false);
});

test("resolveAvatarDataUrl: nextAvatarUrl null → null", () => {
  const existing = makeCache({ avatarDataUrl: "data:image/jpeg;base64,/old" });
  assert.equal(resolveAvatarDataUrl(null, null, existing), null);
});

test("resolveAvatarDataUrl: fetch succeeded → use fetched value", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    resolveAvatarDataUrl(
      "https://relay.example.com/new.jpg",
      "data:image/jpeg;base64,/new",
      existing,
    ),
    "data:image/jpeg;base64,/new",
  );
});

test("resolveAvatarDataUrl: fetch failed, URL unchanged → preserve existing", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/same.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/existing",
  });
  assert.equal(
    resolveAvatarDataUrl("https://relay.example.com/same.jpg", null, existing),
    "data:image/jpeg;base64,/existing",
  );
});

test("resolveAvatarDataUrl: fetch failed, URL changed → null", () => {
  const existing = makeCache({
    avatarUrl: "https://relay.example.com/old.jpg",
    avatarDataUrl: "data:image/jpeg;base64,/old",
  });
  assert.equal(
    resolveAvatarDataUrl("https://relay.example.com/new.jpg", null, existing),
    null,
  );
});

// ---------------------------------------------------------------------------
// Memoized key-count tests
// ---------------------------------------------------------------------------

function installStorageTracked() {
  const values = new Map();
  let keyCallCount = 0;
  const mock = {
    values,
    get keyCallCount() {
      return keyCallCount;
    },
    get length() {
      return values.size;
    },
    key(index) {
      keyCallCount++;
      return [...values.keys()][index] ?? null;
    },
    getItem: (key) => values.get(key) ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, String(value)),
  };
  globalThis.window = {
    dispatchEvent: () => true,
    localStorage: mock,
  };
  globalThis.CustomEvent ??= class CustomEvent {};
  return mock;
}

test("trim does not call key() when memo is initialized and total is under per-relay cap", () => {
  _resetProfileKeyCountForTest();
  const storage = installStorageTracked();
  const relay = "wss://relay.example";

  // First write: memo uninitialized → ensureProfileKeyCount scans (calls key()).
  writeSelfProfileCache(relay, "pk-0", makeCache({ updatedAt: 1 }));
  const keyCallsAfterFirst = storage.keyCallCount;
  assert.ok(
    keyCallsAfterFirst > 0,
    "first write triggers key scan to init memo",
  );

  // Second write with total still ≤ MAX_SELF_PROFILE_CACHES_PER_RELAY (8).
  // Memo already initialized → trim returns in O(1), no key() calls.
  const before = storage.keyCallCount;
  writeSelfProfileCache(relay, "pk-1", makeCache({ updatedAt: 2 }));
  const after = storage.keyCallCount;
  assert.equal(
    after - before,
    0,
    "no key() calls in trim when memo is hot and count is under cap",
  );
});

test("memoized count resyncs when a full scan finds external deletions", () => {
  _resetProfileKeyCountForTest();
  const storage = installStorageTracked();
  const relay = "wss://relay.example";

  // Fill to just above per-relay cap so trim does a key-only scan.
  for (let i = 0; i < MAX_SELF_PROFILE_CACHES_PER_RELAY + 1; i++) {
    writeSelfProfileCache(relay, `pk-${i}`, makeCache({ updatedAt: i + 1 }));
  }

  // Externally delete some keys to make the memo stale.
  const keysToDelete = [...storage.values.keys()]
    .filter((k) => k.startsWith("buzz-self-profile.v1:"))
    .slice(0, 3);
  for (const k of keysToDelete) storage.values.delete(k);

  // Next write: trim does key-only scan, detects mismatch, resyncs memo.
  // Should not throw and should cap correctly.
  assert.doesNotThrow(() => {
    writeSelfProfileCache(relay, "pk-new", makeCache({ updatedAt: 999 }));
  });
  // After resync the memo reflects actual storage (no stale inflation).
  const finalKeys = [...storage.values.keys()].filter((k) =>
    k.startsWith("buzz-self-profile.v1:"),
  );
  assert.ok(
    finalKeys.length <= MAX_SELF_PROFILE_CACHES_PER_RELAY,
    `expected ≤${MAX_SELF_PROFILE_CACHES_PER_RELAY} keys after trim, got ${finalKeys.length}`,
  );
});

test("over-cap write still trims correctly after memo init", () => {
  _resetProfileKeyCountForTest();
  const values = installStorage();
  const relay = "wss://relay-a.example";

  // Fill to cap + 1 (triggers the full parse+evict path).
  for (let i = 0; i <= MAX_SELF_PROFILE_CACHES_PER_RELAY; i++) {
    writeSelfProfileCache(relay, `pk-${i}`, makeCache({ updatedAt: i }));
  }

  // Oldest (updatedAt=0) should have been evicted; newest preserved.
  assert.equal(values.has(storageKey(relay, "pk-0")), false, "oldest evicted");
  assert.equal(
    values.has(storageKey(relay, `pk-${MAX_SELF_PROFILE_CACHES_PER_RELAY}`)),
    true,
    "newest preserved",
  );
});

test("scan failure in ensureProfileKeyCount: write succeeds, memo stays null, next write rescans and enforces cap", () => {
  _resetProfileKeyCountForTest();

  const values = new Map();
  let throwOnScan = false;
  globalThis.window = {
    dispatchEvent: () => true,
    localStorage: {
      get length() {
        if (throwOnScan) throw new Error("Storage unavailable");
        return values.size;
      },
      getItem: (k) => values.get(k) ?? null,
      key: (i) => [...values.keys()][i] ?? null,
      removeItem: (k) => values.delete(k),
      setItem: (k, v) => values.set(k, String(v)),
    },
  };
  globalThis.CustomEvent ??= class CustomEvent {};

  const relay = "wss://relay.example";

  // Write with scan disabled — ensureProfileKeyCount should catch the throw,
  // leave memo null, and let the write succeed.
  throwOnScan = true;
  const result = writeSelfProfileCache(
    relay,
    "pk-0",
    makeCache({ updatedAt: 1 }),
  );
  throwOnScan = false;

  assert.equal(result, true, "write returns true despite scan failure");
  assert.equal(
    values.has(storageKey(relay, "pk-0")),
    true,
    "entry was written",
  );

  // Pre-seed enough entries (directly into the map) to push total above cap.
  for (let i = 1; i <= MAX_SELF_PROFILE_CACHES_PER_RELAY; i++) {
    values.set(
      storageKey(relay, `pk-${i}`),
      JSON.stringify(makeCache({ updatedAt: i + 1 })),
    );
  }

  // Replace key() with a tracking version to confirm memo was null (i.e. a
  // fresh scan is triggered on the next write).
  let keyCalls = 0;
  globalThis.window.localStorage.key = (i) => {
    keyCalls++;
    return [...values.keys()][i] ?? null;
  };

  writeSelfProfileCache(relay, "pk-new", makeCache({ updatedAt: 999 }));

  assert.ok(
    keyCalls > 0,
    "key() was called, confirming memo was null after scan failure",
  );

  const finalKeys = [...values.keys()].filter((k) =>
    k.startsWith("buzz-self-profile.v1:"),
  );
  assert.ok(
    finalKeys.length <= MAX_SELF_PROFILE_CACHES_PER_RELAY,
    `cap enforced after rescan: got ${finalKeys.length}, expected ≤${MAX_SELF_PROFILE_CACHES_PER_RELAY}`,
  );
});
