import assert from "node:assert/strict";
import test from "node:test";

// We need a minimal localStorage stub since we're running in Node.
function makeLocalStorage() {
  const store = new Map();
  return {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
    removeItem: (key) => store.delete(key),
    clear: () => store.clear(),
  };
}

// Inject a fresh localStorage before each test by re-requiring the module.
// node:test doesn't reload modules between tests, so we manipulate the global
// directly and clear between tests.

function withFreshStorage(fn) {
  const fake = makeLocalStorage();
  const orig = globalThis.window?.localStorage;
  if (typeof globalThis.window === "undefined") globalThis.window = {};
  globalThis.window.localStorage = fake;
  try {
    fn(fake);
  } finally {
    if (orig !== undefined) {
      globalThis.window.localStorage = orig;
    } else {
      delete globalThis.window.localStorage;
    }
  }
}

const { readWatermark, advanceWatermark } = await import(
  "./sidebarSyncWatermark.ts"
);

// Relay URLs are normalised (trimmed, lowercase, trailing slash stripped)
// so the same relay written two ways produces the same key.
const RELAY = "wss://relay.example.com";
const RELAY_ENCODED = encodeURIComponent("wss://relay.example.com");

// ── readWatermark ────────────────────────────────────────────────────────────

test("readWatermark: returns 0 when no key exists", () => {
  withFreshStorage(() => {
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
  });
});

test("readWatermark: returns 0 when stored value is 0", () => {
  withFreshStorage((ls) => {
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`, "0");
    assert.equal(readWatermark("pk", "sections", RELAY), 0);
  });
});

test("readWatermark: returns stored positive integer", () => {
  withFreshStorage((ls) => {
    ls.setItem(
      `buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`,
      "1700000000",
    );
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
  });
});

test("readWatermark: scopes by blobType", () => {
  withFreshStorage((ls) => {
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${RELAY_ENCODED}`, "100");
    ls.setItem(`buzz-sync-watermark.v1:sort:pk:${RELAY_ENCODED}`, "200");
    assert.equal(readWatermark("pk", "sections", RELAY), 100);
    assert.equal(readWatermark("pk", "sort", RELAY), 200);
  });
});

test("readWatermark: normalises relay URL (trailing slash, case)", () => {
  withFreshStorage(() => {
    // Write with one form, read with another — must produce the same value.
    advanceWatermark("pk", "sections", 999, "WSS://Relay.Example.Com/");
    assert.equal(
      readWatermark("pk", "sections", "wss://relay.example.com"),
      999,
    );
    assert.equal(
      readWatermark("pk", "sections", "WSS://Relay.Example.Com/"),
      999,
    );
  });
});

// ── advanceWatermark ─────────────────────────────────────────────────────────

test("advanceWatermark: writes when no prior value exists", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 1700000000, RELAY);
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000000);
  });
});

test("advanceWatermark: advances when next > current", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 100, RELAY);
    advanceWatermark("pk", "sections", 200, RELAY);
    assert.equal(readWatermark("pk", "sections", RELAY), 200);
  });
});

test("advanceWatermark: does not regress when next <= current (monotonic)", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 500, RELAY);
    advanceWatermark("pk", "sections", 400, RELAY); // older — must not overwrite
    advanceWatermark("pk", "sections", 500, RELAY); // equal — must not overwrite
    assert.equal(readWatermark("pk", "sections", RELAY), 500);
  });
});

test("advanceWatermark: round-trips across separate reads (simulated restart)", () => {
  withFreshStorage(() => {
    // Session A writes watermark.
    advanceWatermark("pk", "sections", 1700000042, RELAY);
    // Session B reads it back.
    assert.equal(readWatermark("pk", "sections", RELAY), 1700000042);
  });
});

// ── Relay-A / Relay-B isolation ──────────────────────────────────────────────

test("relay-A watermark does not suppress first-sync on relay-B", () => {
  withFreshStorage(() => {
    const relayA = "wss://a.relay.test";
    const relayB = "wss://b.relay.test";
    // Session on relay A has seen a blob.
    advanceWatermark("pk", "sections", 1700000100, relayA);
    // Relay B watermark must still be 0.
    assert.equal(
      readWatermark("pk", "sections", relayB),
      0,
      "relay B watermark must be independent of relay A",
    );
  });
});

test("relay-A watermark is preserved after relay-B session", () => {
  withFreshStorage(() => {
    const relayA = "wss://a.relay.test";
    const relayB = "wss://b.relay.test";
    advanceWatermark("pk", "sections", 1700000100, relayA);
    advanceWatermark("pk", "sections", 1700000200, relayB);
    assert.equal(
      readWatermark("pk", "sections", relayA),
      1700000100,
      "relay A head must not be clobbered by relay B activity",
    );
  });
});
