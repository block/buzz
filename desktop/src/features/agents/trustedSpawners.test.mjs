import assert from "node:assert/strict";
import test from "node:test";

// The module reads localStorage at import time, so stub it first.
const store = new Map();
globalThis.window = {
  localStorage: {
    getItem: (key) => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, value),
  },
};

const { isSpawnerTrusted, resetTrustedSpawners, revokeSpawner, trustSpawner } =
  await import("./trustedSpawners.ts");

const SPAWNER = "5c".repeat(32);
const OTHER = "7a".repeat(32);

test("remembersAnApprovedSpawner", () => {
  resetTrustedSpawners();
  assert.equal(isSpawnerTrusted(SPAWNER), false);
  trustSpawner(SPAWNER);
  assert.equal(isSpawnerTrusted(SPAWNER), true);
  // Approving one spawner must not vouch for any other.
  assert.equal(isSpawnerTrusted(OTHER), false);
});

test("ignoresValuesThatAreNotPubkeys", () => {
  resetTrustedSpawners();
  trustSpawner("not-a-pubkey");
  assert.equal(isSpawnerTrusted("not-a-pubkey"), false);
});

test("matchesRegardlessOfCase", () => {
  resetTrustedSpawners();
  trustSpawner(SPAWNER.toUpperCase());
  assert.equal(isSpawnerTrusted(SPAWNER), true);
});

test("revokeStopsAutoApproval", () => {
  resetTrustedSpawners();
  trustSpawner(SPAWNER);
  revokeSpawner(SPAWNER);
  assert.equal(isSpawnerTrusted(SPAWNER), false);
});

test("resetClearsEveryDecisionAndThePersistedCopy", () => {
  // The community-switch path calls this. A spawner trusted under the old
  // identity must not auto-sign under the new one — and it must not come back
  // from localStorage on the next read either.
  resetTrustedSpawners();
  trustSpawner(SPAWNER);
  trustSpawner(OTHER);

  resetTrustedSpawners();

  assert.equal(isSpawnerTrusted(SPAWNER), false);
  assert.equal(isSpawnerTrusted(OTHER), false);
  assert.deepEqual(JSON.parse(store.get("buzz:trusted-spawners")), []);
});
