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

// ── readWatermark ────────────────────────────────────────────────────────────

test("readWatermark: returns 0 when no key exists", () => {
  withFreshStorage(() => {
    assert.equal(readWatermark("pk", "sections"), 0);
  });
});

test("readWatermark: returns 0 when stored value is 0", () => {
  withFreshStorage((ls) => {
    ls.setItem("buzz-sync-watermark.v1:sections:pk", "0");
    assert.equal(readWatermark("pk", "sections"), 0);
  });
});

test("readWatermark: returns stored positive integer", () => {
  withFreshStorage((ls) => {
    ls.setItem("buzz-sync-watermark.v1:sections:pk", "1700000000");
    assert.equal(readWatermark("pk", "sections"), 1700000000);
  });
});

test("readWatermark: scopes by blobType", () => {
  withFreshStorage((ls) => {
    ls.setItem("buzz-sync-watermark.v1:sections:pk", "100");
    ls.setItem("buzz-sync-watermark.v1:sort:pk", "200");
    assert.equal(readWatermark("pk", "sections"), 100);
    assert.equal(readWatermark("pk", "sort"), 200);
  });
});

test("readWatermark: scopes by relayUrl", () => {
  withFreshStorage((ls) => {
    const encoded = encodeURIComponent("wss://relay.example.com");
    ls.setItem(`buzz-sync-watermark.v1:sections:pk:${encoded}`, "999");
    assert.equal(readWatermark("pk", "sections"), 0); // no relay scope
    assert.equal(
      readWatermark("pk", "sections", "wss://relay.example.com"),
      999,
    );
  });
});

// ── advanceWatermark ─────────────────────────────────────────────────────────

test("advanceWatermark: writes when no prior value exists", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 1700000000);
    assert.equal(readWatermark("pk", "sections"), 1700000000);
  });
});

test("advanceWatermark: advances when next > current", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 100);
    advanceWatermark("pk", "sections", 200);
    assert.equal(readWatermark("pk", "sections"), 200);
  });
});

test("advanceWatermark: does not regress when next <= current", () => {
  withFreshStorage(() => {
    advanceWatermark("pk", "sections", 500);
    advanceWatermark("pk", "sections", 400); // older — must not overwrite
    advanceWatermark("pk", "sections", 500); // equal — must not overwrite
    assert.equal(readWatermark("pk", "sections"), 500);
  });
});

test("advanceWatermark: round-trips across separate reads (simulated restart)", () => {
  withFreshStorage(() => {
    // Session A writes watermark.
    advanceWatermark("pk", "sections", 1700000042, "wss://relay.example.com");
    // Session B reads it back.
    assert.equal(
      readWatermark("pk", "sections", "wss://relay.example.com"),
      1700000042,
    );
  });
});
