import assert from "node:assert/strict";
import test from "node:test";

const values = new Map();
globalThis.localStorage = {
  getItem: (key) => values.get(key) ?? null,
  setItem: (key, value) => values.set(key, String(value)),
};

const grants = await import("./directAgentCreationGrant.ts");

test("parses only normalized unique pubkeys", () => {
  const upper = "A".repeat(64);
  assert.deepEqual(
    grants.parseDirectAgentCreationGrants(
      JSON.stringify([upper, upper.toLowerCase(), "invalid", 7]),
    ),
    [upper.toLowerCase()],
  );
  assert.deepEqual(grants.parseDirectAgentCreationGrants("not-json"), []);
});

test("persists per-agent grants and supports revocation", () => {
  const owner = "a".repeat(64);
  const pubkey = "b".repeat(64);
  grants.setDirectAgentCreationGrant(owner, pubkey, true);
  assert.equal(grants.hasDirectAgentCreationGrant(owner, pubkey), true);
  assert.equal(
    values.get(`${grants.DIRECT_AGENT_CREATION_GRANTS_STORAGE_KEY}.${owner}`),
    JSON.stringify([pubkey]),
  );

  grants.setDirectAgentCreationGrant(owner, pubkey, false);
  assert.equal(grants.hasDirectAgentCreationGrant(owner, pubkey), false);
});

test("grants do not cross owner identities", () => {
  const ownerA = "c".repeat(64);
  const ownerB = "d".repeat(64);
  const agent = "e".repeat(64);
  grants.setDirectAgentCreationGrant(ownerA, agent, true);

  assert.equal(grants.hasDirectAgentCreationGrant(ownerA, agent), true);
  assert.equal(grants.hasDirectAgentCreationGrant(ownerB, agent), false);
});

test("a failed permission write leaves the prior grant state in force", () => {
  const owner = "1".repeat(64);
  const agent = "2".repeat(64);
  const originalSetItem = globalThis.localStorage.setItem;
  globalThis.localStorage.setItem = () => {
    throw new Error("quota denied");
  };
  try {
    grants.setDirectAgentCreationGrant(owner, agent, true);
  } finally {
    globalThis.localStorage.setItem = originalSetItem;
  }

  assert.equal(grants.hasDirectAgentCreationGrant(owner, agent), false);
});
