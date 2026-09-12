import assert from "node:assert/strict";
import test from "node:test";

async function loadSubject() {
  try {
    return await import("./userLabelStorage.ts");
  } catch {
    return {};
  }
}

function installLocalStorage() {
  const values = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
      removeItem: (key) => values.delete(key),
      key: (index) => [...values.keys()][index] ?? null,
      get length() {
        return values.size;
      },
    },
  };
  globalThis.localStorage = globalThis.window.localStorage;
  return values;
}

test("reads cached labels as safe stale profile summaries", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.readCachedUserLabels, "function");
  installLocalStorage();
  window.localStorage.setItem(
    "buzz-user-labels.v1:wss://relay.example",
    JSON.stringify({
      version: 1,
      updatedAt: 100,
      profiles: {
        abcdef: {
          displayName: "Alice",
          name: "alice",
          nip05Handle: "alice@example.com",
          updatedAt: 100,
        },
      },
    }),
  );

  assert.deepEqual(
    subject.readCachedUserLabels("WSS://Relay.Example/", ["ABCDEF", "missing"]),
    {
      profiles: {
        abcdef: {
          displayName: "Alice",
          name: "alice",
          avatarUrl: null,
          nip05Handle: "alice@example.com",
          ownerPubkey: null,
        },
      },
      missing: [],
    },
  );
});

test("keeps previous full profiles ahead of persisted label placeholders", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.resolveUserLabelPlaceholderData, "function");
  installLocalStorage();
  window.localStorage.setItem(
    "buzz-user-labels.v1:wss://relay.example",
    JSON.stringify({
      version: 1,
      profiles: {
        abcdef: {
          displayName: "Cached Alice",
          name: "alice",
          nip05Handle: null,
          updatedAt: 100,
        },
      },
    }),
  );
  const previous = {
    profiles: {
      abcdef: {
        displayName: "Fresh Alice",
        name: "alice",
        avatarUrl: "https://relay.example/alice.png",
        nip05Handle: null,
        ownerPubkey: "owner",
      },
    },
    missing: [],
  };

  assert.equal(
    subject.resolveUserLabelPlaceholderData(previous, "wss://relay.example", [
      "abcdef",
    ]),
    previous,
  );
});

test("writes merge with existing labels and remain bounded", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.writeCachedUserLabels, "function");
  installLocalStorage();

  subject.writeCachedUserLabels("wss://relay.example", {
    existing: {
      displayName: "Existing",
      name: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });
  subject.writeCachedUserLabels(
    "wss://relay.example",
    Object.fromEntries(
      Array.from({ length: 1_005 }, (_, index) => [
        `pubkey-${index}`,
        {
          displayName: `Person ${index}`,
          name: null,
          avatarUrl: null,
          nip05Handle: null,
          ownerPubkey: null,
        },
      ]),
    ),
  );

  const stored = JSON.parse(
    window.localStorage.getItem(
      subject.userLabelCacheKey("wss://relay.example"),
    ),
  );
  assert.equal(Object.keys(stored.profiles).length, 1_000);
  assert.equal(stored.version, 1);
  assert.equal(stored.updatedAt, undefined);
});

test("removes a stale label when the fresh profile clears all names", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.writeCachedUserLabels, "function");
  installLocalStorage();

  subject.writeCachedUserLabels("wss://relay.example", {
    abcdef: {
      displayName: "Alice",
      name: "alice",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });
  subject.writeCachedUserLabels("wss://relay.example", {
    abcdef: {
      displayName: null,
      name: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });

  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", ["abcdef"]),
    undefined,
  );
});

test("removes stale labels for profiles the relay reports missing", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.writeCachedUserLabels, "function");
  installLocalStorage();

  subject.writeCachedUserLabels("wss://relay.example", {
    abcdef: {
      displayName: "Alice",
      name: "alice",
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });
  subject.writeCachedUserLabels("wss://relay.example", {}, ["ABCDEF"]);

  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", ["abcdef"]),
    undefined,
  );
});

test("removes only the selected relay cache", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.removeUserLabelCacheForRelay, "function");
  installLocalStorage();
  const first = subject.userLabelCacheKey("wss://one.example");
  const second = subject.userLabelCacheKey("wss://two.example");
  window.localStorage.setItem(first, "{}");
  window.localStorage.setItem(second, "{}");

  subject.removeUserLabelCacheForRelay("wss://one.example");

  assert.equal(window.localStorage.getItem(first), null);
  assert.equal(window.localStorage.getItem(second), "{}");
});

test("ignores malformed cache payloads", async () => {
  const subject = await loadSubject();
  assert.equal(typeof subject.readCachedUserLabels, "function");
  installLocalStorage();
  window.localStorage.setItem(
    "buzz-user-labels.v1:wss://relay.example",
    JSON.stringify({ version: 1, profiles: { abc: { displayName: 42 } } }),
  );

  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", ["abc"]),
    undefined,
  );
});

// ── Parse cost ───────────────────────────────────────────────────────────────
//
// `readCache` is reached once per React render, per query observer, through
// `resolveUserLabelPlaceholderData`. These pin the two costs that made an idle
// window burn CPU: re-parsing bytes that have not changed, and parsing at all
// for a caller that asked about no pubkeys.

function seedCache(relayUrl, count) {
  window.localStorage.setItem(
    `buzz-user-labels.v1:${relayUrl}`,
    JSON.stringify({
      version: 1,
      profiles: Object.fromEntries(
        Array.from({ length: count }, (_, index) => [
          `pubkey-${index}`,
          {
            displayName: `Person ${index}`,
            name: null,
            nip05Handle: null,
            updatedAt: index,
          },
        ]),
      ),
    }),
  );
}

function countingParse() {
  const original = JSON.parse;
  let calls = 0;
  JSON.parse = (...args) => {
    calls += 1;
    return original(...args);
  };
  return {
    calls: () => calls,
    restore: () => {
      JSON.parse = original;
    },
  };
}

test("unchanged cache bytes are parsed once, not once per read", async () => {
  const subject = await loadSubject();
  installLocalStorage();
  subject.resetUserLabelCacheMemo();
  seedCache("wss://relay.example", 200);

  const parse = countingParse();
  try {
    for (let i = 0; i < 25; i += 1) {
      subject.readCachedUserLabels("wss://relay.example", ["pubkey-1"]);
    }
    assert.equal(parse.calls(), 1);
  } finally {
    parse.restore();
  }

  assert.deepEqual(
    subject.readCachedUserLabels("wss://relay.example", ["pubkey-1"]).profiles[
      "pubkey-1"
    ].displayName,
    "Person 1",
  );
});

test("an empty pubkey list never touches storage", async () => {
  const subject = await loadSubject();
  const values = installLocalStorage();
  subject.resetUserLabelCacheMemo();
  seedCache("wss://relay.example", 200);

  let reads = 0;
  const getItem = window.localStorage.getItem;
  window.localStorage.getItem = (key) => {
    reads += 1;
    return getItem(key);
  };

  // A closed UserProfilePopover passes `[]` and mounts per message row, per
  // avatar and per member-list entry, so this is the common call.
  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", []),
    undefined,
  );
  assert.equal(
    subject.resolveUserLabelPlaceholderData(
      undefined,
      "wss://relay.example",
      [],
    ),
    undefined,
  );
  assert.equal(reads, 0);
  assert.ok(values.size > 0);
});

test("a write is seen by the next read", async () => {
  const subject = await loadSubject();
  installLocalStorage();
  subject.resetUserLabelCacheMemo();

  subject.writeCachedUserLabels("wss://relay.example", {
    abcdef: {
      displayName: "Alice",
      name: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });
  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", ["abcdef"]).profiles
      .abcdef.displayName,
    "Alice",
  );

  subject.writeCachedUserLabels("wss://relay.example", {
    abcdef: {
      displayName: "Alice Renamed",
      name: null,
      avatarUrl: null,
      nip05Handle: null,
      ownerPubkey: null,
    },
  });
  assert.equal(
    subject.readCachedUserLabels("wss://relay.example", ["abcdef"]).profiles
      .abcdef.displayName,
    "Alice Renamed",
  );
});

test("one relay's labels are never served for another", async () => {
  const subject = await loadSubject();
  installLocalStorage();
  subject.resetUserLabelCacheMemo();
  window.localStorage.setItem(
    "buzz-user-labels.v1:wss://a.example",
    JSON.stringify({
      version: 1,
      profiles: {
        abcdef: {
          displayName: "From A",
          name: null,
          nip05Handle: null,
          updatedAt: 1,
        },
      },
    }),
  );
  window.localStorage.setItem(
    "buzz-user-labels.v1:wss://b.example",
    JSON.stringify({
      version: 1,
      profiles: {
        abcdef: {
          displayName: "From B",
          name: null,
          nip05Handle: null,
          updatedAt: 1,
        },
      },
    }),
  );

  for (const [relay, expected] of [
    ["wss://a.example", "From A"],
    ["wss://b.example", "From B"],
    ["wss://a.example", "From A"],
  ]) {
    assert.equal(
      subject.readCachedUserLabels(relay, ["abcdef"]).profiles.abcdef
        .displayName,
      expected,
    );
  }
});

test("resetUserLabelCacheMemo forces the next read to re-parse", async () => {
  const subject = await loadSubject();
  installLocalStorage();
  subject.resetUserLabelCacheMemo();
  seedCache("wss://relay.example", 50);

  const parse = countingParse();
  try {
    subject.readCachedUserLabels("wss://relay.example", ["pubkey-1"]);
    subject.readCachedUserLabels("wss://relay.example", ["pubkey-1"]);
    assert.equal(parse.calls(), 1);
    subject.resetUserLabelCacheMemo();
    subject.readCachedUserLabels("wss://relay.example", ["pubkey-1"]);
    assert.equal(parse.calls(), 2);
  } finally {
    parse.restore();
  }
});
