import assert from "node:assert/strict";
import test from "node:test";

function createStorage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };
}

const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const agentC = "c".repeat(64);
const ownerA = "1".repeat(64);
const ownerB = "2".repeat(64);
const storageKey = "buzz:persistent-agent-audiences:v2";

let loadSequence = 0;

async function loadStore(offset = 0) {
  globalThis.window = { localStorage: createStorage() };
  loadSequence += 1;
  return import(
    `./persistentAgentAudience.ts?test=${Date.now()}-${offset}-${loadSequence}`
  );
}

function savedAudiences() {
  return JSON.parse(window.localStorage.getItem(storageKey));
}

test("conversation scopes isolate identities, channels, and threads", async () => {
  const store = await loadStore();
  const channelA = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
  });
  const channelB = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-b",
  });
  const threadA1 = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    threadRootId: "root-1",
  });
  const threadA2 = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    threadRootId: "root-2",
  });
  const otherIdentity = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerB,
    channelId: "channel-a",
  });

  for (const scope of [channelA, channelB, threadA1, threadA2, otherIdentity]) {
    assert.ok(scope);
    store.setPersistentAgentAudience(scope, [agentA]);
  }

  assert.equal(new Set(Object.keys(savedAudiences())).size, 5);
});

test("successful fast send promotes without a persisted draft key", async () => {
  const store = await loadStore(1);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
  });
  store.setPersistentAgentAudienceEnabled(true);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    explicitAgentPubkeys: [agentA],
  });

  assert.deepEqual(savedAudiences(), { [scope]: [agentA] });
});

test("explicit recipients merge and dedupe after successful send", async () => {
  const store = await loadStore(2);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience(scope, [agentA]);
  const revision = store.getPersistentAgentAudienceRevision(scope);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: revision,
    explicitAgentPubkeys: [agentA, agentB],
  });

  assert.deepEqual(savedAudiences(), { [scope]: [agentA, agentB] });
});

test("successful send makes authored mention order authoritative", async () => {
  const store = await loadStore(100);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience(scope, [agentA, agentB]);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    explicitAgentPubkeys: [agentB, agentA, agentC],
  });

  assert.deepEqual(savedAudiences(), {
    [scope]: [agentB, agentA, agentC],
  });
});

test("successful send retains saved targets absent from the draft", async () => {
  const store = await loadStore(101);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience(scope, [agentA, agentC]);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    explicitAgentPubkeys: [agentB, agentA],
  });

  assert.deepEqual(savedAudiences(), {
    [scope]: [agentB, agentA, agentC],
  });
});

test("removal while send awaits wins over late success", async () => {
  const store = await loadStore(3);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience(scope, [agentA]);
  const revisionAtSubmit = store.getPersistentAgentAudienceRevision(scope);

  store.removePersistentAgentAudienceMember(scope, agentA);
  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: revisionAtSubmit,
    explicitAgentPubkeys: [agentA],
  });

  assert.deepEqual(savedAudiences(), { [scope]: [] });
});

test("removing final chip preserves an explicit empty scope", async () => {
  const store = await loadStore(4);
  const scope = `${ownerA}:channel-a:thread:root`;
  store.setPersistentAgentAudience(scope, [agentA]);
  store.removePersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(savedAudiences(), { [scope]: [] });
});

test("completion after disabling cannot repopulate audiences", async () => {
  const store = await loadStore(5);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);
  store.setPersistentAgentAudience(scope, [agentA]);
  const revisionAtSubmit = store.getPersistentAgentAudienceRevision(scope);
  store.setPersistentAgentAudienceEnabled(false);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: revisionAtSubmit,
    explicitAgentPubkeys: [agentB],
  });

  assert.deepEqual(savedAudiences(), {});
});

test("invalid, duplicate, and differently-cased pubkeys normalize", async () => {
  const store = await loadStore(6);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudience(scope, [
    agentA.toUpperCase(),
    agentA,
    "bad",
  ]);

  assert.deepEqual(savedAudiences(), { [scope]: [agentA] });
});

test("new recipients retain explicit mention order", async () => {
  const store = await loadStore(9);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudienceEnabled(true);

  store.promotePersistentAgentAudience({
    expectedGeneration: store.getPersistentAgentAudienceGeneration(),
    scope,
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    explicitAgentPubkeys: [agentB, agentA],
  });

  assert.deepEqual(savedAudiences(), { [scope]: [agentB, agentA] });
});

test("first new-message send resolves its destination after capturing generation", async () => {
  const store = await loadStore(7);
  const capturedGeneration = store.getPersistentAgentAudienceGeneration();
  store.setPersistentAgentAudienceEnabled(true);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "resolved-dm",
  });

  store.promotePersistentAgentAudience({
    expectedGeneration: capturedGeneration,
    expectedRevision: null,
    scope,
    explicitAgentPubkeys: [agentA],
  });

  assert.deepEqual(savedAudiences(), { [scope]: [agentA] });
});

test("disable during new-message destination preparation invalidates promotion", async () => {
  const store = await loadStore(8);
  store.setPersistentAgentAudienceEnabled(true);
  const capturedGeneration = store.getPersistentAgentAudienceGeneration();
  store.setPersistentAgentAudienceEnabled(false);
  store.setPersistentAgentAudienceEnabled(true);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "resolved-dm",
  });

  store.promotePersistentAgentAudience({
    expectedGeneration: capturedGeneration,
    expectedRevision: null,
    scope,
    explicitAgentPubkeys: [agentA],
  });

  assert.deepEqual(savedAudiences(), {});
});
