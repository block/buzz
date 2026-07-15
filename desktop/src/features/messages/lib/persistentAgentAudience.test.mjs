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

test("adding an explicit agent merges with the active audience", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now() + 1}`
  );
  const agentA = "a".repeat(64);
  const agentB = "b".repeat(64);

  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience("channel:one", [agentA]);
  store.addPersistentAgentAudienceMembers("channel:one", [agentB]);

  const saved = JSON.parse(
    window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
  );
  assert.deepEqual(saved, { "channel:one": [agentA, agentB] });
});

test("draft completion updates its captured scope after navigation", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now() + 2}`
  );
  const originatingAgent = "a".repeat(64);
  const viewedAgent = "b".repeat(64);

  store.setPersistentAgentAudienceEnabled(true);
  store.addPersistentAgentAudienceMembersForDraft({
    capturedChannelId: "originating-channel",
    explicitAgentPubkeys: [originatingAgent],
    sentDraftKey: "thread:originating-root",
  });
  store.setPersistentAgentAudience(
    store.getPersistentAgentAudienceScope(
      "newly-viewed-channel",
      "thread:newly-viewed-root",
    ),
    [viewedAgent],
  );

  const saved = JSON.parse(
    window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
  );
  assert.deepEqual(saved, {
    "originating-channel:thread:originating-root": [originatingAgent],
    "newly-viewed-channel:thread:newly-viewed-root": [viewedAgent],
  });
});

test("completion after disabling cannot repopulate a cleared audience", async () => {
  globalThis.window = { localStorage: createStorage() };
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now() + 3}`
  );
  const agent = "a".repeat(64);

  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience("channel:one", [agent]);
  store.setPersistentAgentAudienceEnabled(false);
  store.addPersistentAgentAudienceMembersForDraft({
    capturedChannelId: "channel",
    explicitAgentPubkeys: [agent],
    sentDraftKey: "one",
  });
  store.setPersistentAgentAudienceEnabled(true);

  assert.deepEqual(
    JSON.parse(
      window.localStorage.getItem("buzz:persistent-agent-audiences:v1"),
    ),
    {},
  );
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
