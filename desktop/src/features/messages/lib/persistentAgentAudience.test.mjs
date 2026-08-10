import assert from "node:assert/strict";
import test from "node:test";

function createStorage(onSetItem = () => {}) {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      onSetItem(key, value);
      values.set(key, String(value));
    },
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
  const scopes = [
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-a",
      threadRootId: "root-1",
    }),
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-a",
      threadRootId: "root-2",
    }),
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-b",
      threadRootId: "root-1",
    }),
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerB,
      channelId: "channel-a",
      threadRootId: "root-1",
    }),
  ];

  for (const scope of scopes) {
    assert.ok(scope);
    store.setPersistentAgentAudience(scope, [agentA]);
  }

  assert.equal(new Set(Object.keys(savedAudiences())).size, 4);
});

test("successful fast send promotes without a persisted draft key", async () => {
  const store = await loadStore(1);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    threadRootId: "root",
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

test("persistent audiences retain only the 200 most recently touched scopes", async () => {
  const store = await loadStore(11);
  for (
    let index = 0;
    index < store.MAX_PERSISTENT_AGENT_AUDIENCES + 2;
    index++
  ) {
    store.setPersistentAgentAudience(`scope-${index}`, [agentA]);
  }

  const saved = savedAudiences();
  assert.equal(Object.keys(saved).length, store.MAX_PERSISTENT_AGENT_AUDIENCES);
  assert.equal(saved["scope-0"], undefined);
  assert.equal(saved["scope-1"], undefined);
  assert.deepEqual(saved["scope-201"], [agentA]);

  store.setPersistentAgentAudience("scope-2", [agentB]);
  store.setPersistentAgentAudience("scope-new", [agentC]);
  const retouched = savedAudiences();
  assert.equal(retouched["scope-3"], undefined);
  assert.deepEqual(retouched["scope-2"], [agentB]);
  assert.deepEqual(retouched["scope-new"], [agentC]);
});

test("an unchanged touch refreshes LRU without revision or emit", async () => {
  const { JSDOM } = await import("jsdom");
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div></body></html>",
    {
      url: "http://localhost",
    },
  );
  const writes = [];
  Object.defineProperty(dom.window, "localStorage", {
    configurable: true,
    value: createStorage((key, value) => writes.push([key, String(value)])),
  });
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  loadSequence += 1;
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now()}-touch-${loadSequence}`
  );
  const touchedScope = "scope-0";
  store.setPersistentAgentAudience(touchedScope, [agentA]);
  for (let index = 1; index < store.MAX_PERSISTENT_AGENT_AUDIENCES; index++) {
    store.setPersistentAgentAudience(`scope-${index}`, [agentA]);
  }

  const React = await import("react");
  const { createRoot } = await import("react-dom/client");
  const root = createRoot(document.getElementById("root"));
  let renderCount = 0;
  function Probe() {
    store.usePersistentAgentAudience(touchedScope);
    renderCount += 1;
    return null;
  }
  await React.act(async () => root.render(React.createElement(Probe)));
  const revision = store.getPersistentAgentAudienceRevision(touchedScope);
  const renderCountBeforeTouch = renderCount;
  writes.length = 0;

  await React.act(async () => {
    store.setPersistentAgentAudience(touchedScope, [agentA]);
  });

  assert.equal(writes.length, 1);
  assert.equal(writes[0][0], storageKey);
  assert.deepEqual(JSON.parse(writes[0][1])[touchedScope], [agentA]);
  assert.equal(Object.keys(JSON.parse(writes[0][1])).at(-1), touchedScope);
  assert.equal(
    store.getPersistentAgentAudienceRevision(touchedScope),
    revision,
  );
  assert.equal(renderCount, renderCountBeforeTouch);

  writes.length = 0;
  await React.act(async () => {
    store.setPersistentAgentAudience(touchedScope, [agentA]);
  });
  assert.equal(writes.length, 0);
  assert.equal(
    store.getPersistentAgentAudienceRevision(touchedScope),
    revision,
  );
  assert.equal(renderCount, renderCountBeforeTouch);

  await React.act(async () => {
    store.setPersistentAgentAudience("scope-new", [agentB]);
  });
  const saved = savedAudiences();
  assert.deepEqual(saved[touchedScope], [agentA]);
  assert.equal(saved["scope-1"], undefined);
  assert.deepEqual(saved["scope-new"], [agentB]);
  assert.equal(
    store.getPersistentAgentAudienceRevision(touchedScope),
    revision,
  );

  await React.act(async () => root.unmount());
  dom.window.close();
});

test("timeline scope is intentionally unsupported", async () => {
  const store = await loadStore(7);
  assert.equal(
    store.getPersistentAgentAudienceScope({
      ownerPubkey: ownerA,
      channelId: "channel-a",
    }),
    null,
  );
});

test("thread root audience initializes once and explicit clear wins on reopen", async () => {
  const store = await loadStore(10);
  const scope = `${ownerA}:channel-a:thread:root`;
  store.setPersistentAgentAudienceEnabled(true);

  store.initializePersistentAgentAudience(scope, [agentB, agentA]);
  assert.deepEqual(savedAudiences(), { [scope]: [agentB, agentA] });

  store.setPersistentAgentAudience(scope, []);
  store.initializePersistentAgentAudience(scope, [agentA]);
  assert.deepEqual(savedAudiences(), { [scope]: [] });
});
