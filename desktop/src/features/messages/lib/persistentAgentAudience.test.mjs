import assert from "node:assert/strict";
import test from "node:test";

function createStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
}

test("audiences remain independently scoped and removable", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(`./persistentAgentAudience.ts?test=${Date.now()}`);
  const agentA = "a".repeat(64);
  const agentB = "b".repeat(64);

  store.setPersistentAgentAudience("channel:one", [agentA]);
  store.setPersistentAgentAudience("thread:root", [agentB]);
  store.removePersistentAgentAudienceMember("channel:one", agentA);

  const saved = JSON.parse(
    window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
  );
  assert.deepEqual(saved, { "thread:root": [agentB] });
});

test("invalid, duplicate, and differently-cased pubkeys are normalized", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now() + 1}`
  );
  const agent = "A".repeat(64);

  store.setPersistentAgentAudience("channel:one", [
    agent,
    agent.toLowerCase(),
    "bad",
  ]);

  const saved = JSON.parse(
    window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
  );
  assert.deepEqual(saved, { "channel:one": [agent.toLowerCase()] });
});

test("preference persists and disabling clears stale audiences", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now() + 2}`
  );

  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience("channel:one", ["a".repeat(64)]);
  store.setPersistentAgentAudienceEnabled(false);

  assert.equal(
    window.localStorage.getItem("buzz:keep-addressed-agents-active"),
    "0",
  );
  assert.deepEqual(
    JSON.parse(
      window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
    ),
    {},
  );
});
