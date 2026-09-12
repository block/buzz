import assert from "node:assert/strict";
import test from "node:test";

const agentA = "a".repeat(64);
const agentB = "b".repeat(64);
const agentC = "c".repeat(64);
const ownerA = "1".repeat(64);
const ownerB = "2".repeat(64);

let loadSequence = 0;

async function loadStore(offset = 0) {
  loadSequence += 1;
  return import(
    `./persistentAgentAudience.ts?test=${Date.now()}-${offset}-${loadSequence}`
  );
}

function currentAudiences(store) {
  return store.getPersistentAgentAudienceSnapshot().audiences;
}

test("audience scopes isolate identities and channels", async () => {
  const store = await loadStore();
  const scopes = [
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-a",
    }),
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-b",
    }),
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerB,
      channelId: "channel-a",
    }),
  ];

  for (const scope of scopes) {
    assert.ok(scope);
    store.setPersistentAgentAudience(scope, [agentA]);
  }

  assert.equal(new Set(Object.keys(currentAudiences(store))).size, 3);
});

test("address locks can be added independently", async () => {
  const store = await loadStore(1);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
  });
  store.addPersistentAgentAudienceMember(scope, agentA);
  store.addPersistentAgentAudienceMember(scope, agentB);

  assert.deepEqual(currentAudiences(store), { [scope]: [agentA, agentB] });
});

test("adding an existing address lock preserves order and dedupes", async () => {
  const store = await loadStore(2);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA, agentB]);
  store.addPersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(currentAudiences(store), { [scope]: [agentA, agentB] });
});

test("address locks can be removed independently", async () => {
  const store = await loadStore(3);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA, agentB]);

  store.removePersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(currentAudiences(store), { [scope]: [agentB] });
});

test("removing final chip preserves an explicit empty scope", async () => {
  const store = await loadStore(4);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);
  store.removePersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(currentAudiences(store), { [scope]: [] });
});

test("invalid, duplicate, and differently-cased pubkeys normalize", async () => {
  const store = await loadStore(5);
  const scope = `${ownerA}:channel-a:timeline`;
  store.setPersistentAgentAudience(scope, [
    agentA.toUpperCase(),
    agentA,
    "bad",
  ]);

  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});

test("in-memory audiences retain only the 200 most recently touched scopes", async () => {
  const store = await loadStore(6);
  for (
    let index = 0;
    index < store.MAX_IN_MEMORY_AGENT_AUDIENCES + 2;
    index++
  ) {
    store.setPersistentAgentAudience(`scope-${index}`, [agentA]);
  }

  const bounded = currentAudiences(store);
  assert.equal(
    Object.keys(bounded).length,
    store.MAX_IN_MEMORY_AGENT_AUDIENCES,
  );
  assert.equal(bounded["scope-0"], undefined);
  assert.equal(bounded["scope-1"], undefined);
  assert.deepEqual(bounded["scope-201"], [agentA]);

  store.setPersistentAgentAudience("scope-2", [agentB]);
  store.setPersistentAgentAudience("scope-new", [agentC]);
  const retouched = currentAudiences(store);
  assert.equal(retouched["scope-3"], undefined);
  assert.deepEqual(retouched["scope-2"], [agentB]);
  assert.deepEqual(retouched["scope-new"], [agentC]);
});

test("reset clears every audience for refresh and community boundaries", async () => {
  const store = await loadStore(7);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);

  store.resetPersistentAgentAudienceStore();

  assert.deepEqual(currentAudiences(store), {});
});

test("channel and thread composers have independent audience scopes", async () => {
  const store = await loadStore(8);
  const channelScope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    composerKey: "channel-a",
  });
  const threadScope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    composerKey: "thread:root",
  });
  const otherThreadScope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    composerKey: "thread:other-root",
  });

  assert.equal(channelScope, `${ownerA}:channel-a:channel`);
  assert.equal(threadScope, `${ownerA}:channel-a:thread:root`);
  assert.equal(otherThreadScope, `${ownerA}:channel-a:thread:other-root`);
  assert.notEqual(threadScope, channelScope);
  assert.notEqual(otherThreadScope, threadScope);
});

test("delayed promotion cannot overwrite a newer audience choice", async () => {
  const store = await loadStore(9);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, []);
  const sendRevision = store.getPersistentAgentAudienceRevision(scope);

  store.addPersistentAgentAudienceMember(scope, agentA);
  store.removePersistentAgentAudienceMember(scope, agentA);
  const result = store.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: sendRevision,
    pubkeys: [agentA],
    scope,
  });

  assert.equal(result, null);
  assert.deepEqual(currentAudiences(store), { [scope]: [] });
});

test("stale auto-pin Undo cannot remove a newer explicit choice", async () => {
  const store = await loadStore(10);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, []);
  const appliedRevision = store.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    pubkeys: [agentA],
    scope,
  });
  assert.notEqual(appliedRevision, null);

  store.removePersistentAgentAudienceMember(scope, agentA);
  store.addPersistentAgentAudienceMember(scope, agentA);
  const removed = store.removePersistentAgentAudienceMembersIfUnchanged({
    expectedRevision: appliedRevision.revision,
    pubkeys: [agentA],
    scope,
  });

  assert.equal(removed, false);
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});

test("explicitly excluded agents are not auto-promoted again", async () => {
  const store = await loadStore(12);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);
  store.excludePersistentAgentAudienceMember(scope, agentA);

  const promotion = store.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    pubkeys: [agentA],
    scope,
  });

  assert.equal(promotion, null);
  assert.deepEqual(currentAudiences(store), { [scope]: [] });

  store.addPersistentAgentAudienceMember(scope, agentA);
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});

test("initialization preserves exclusions across thread remounts", async () => {
  const store = await loadStore(14);
  const scope = `${ownerA}:channel-a:thread-a`;

  store.initializePersistentAgentAudience(scope, [agentA]);
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });

  store.excludePersistentAgentAudienceMember(scope, agentA);
  store.initializePersistentAgentAudience(scope, [agentA]);
  assert.deepEqual(currentAudiences(store), { [scope]: [] });

  store.addPersistentAgentAudienceMember(scope, agentA);
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});

test("explicit re-selection reinstates an excluded agent", async () => {
  const store = await loadStore(13);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);
  store.excludePersistentAgentAudienceMember(scope, agentA);

  const promotion = store.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    reinstateExcluded: true,
    pubkeys: [agentA],
    scope,
  });

  assert.deepEqual(promotion?.promotedPubkeys, [agentA]);
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});

test("promotion reports only newly added agents for transactional Undo", async () => {
  const store = await loadStore(11);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);
  const promotion = store.promotePersistentAgentAudienceIfUnchanged({
    expectedRevision: store.getPersistentAgentAudienceRevision(scope),
    pubkeys: [agentA, agentB],
    scope,
  });

  assert.deepEqual(promotion.promotedPubkeys, [agentB]);
  assert.equal(
    store.removePersistentAgentAudienceMembersIfUnchanged({
      expectedRevision: promotion.revision,
      pubkeys: promotion.promotedPubkeys,
      scope,
    }),
    true,
  );
  assert.deepEqual(currentAudiences(store), { [scope]: [agentA] });
});
