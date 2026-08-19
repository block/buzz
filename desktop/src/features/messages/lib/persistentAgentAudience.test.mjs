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
const storageKey = (communityId = "community-a") =>
  `buzz:persistent-agent-audiences:v3:${communityId}`;

let loadSequence = 0;

async function loadStore(offset = 0) {
  globalThis.window = { localStorage: createStorage() };
  loadSequence += 1;
  const store = await import(
    `./persistentAgentAudience.ts?test=${Date.now()}-${offset}-${loadSequence}`
  );
  store.initPersistentAgentAudienceStore("community-a");
  return store;
}

function savedAudiences(communityId = "community-a") {
  return JSON.parse(window.localStorage.getItem(storageKey(communityId)));
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

  assert.equal(new Set(Object.keys(savedAudiences())).size, 3);
});

test("address locks can be added independently", async () => {
  const store = await loadStore(1);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
  });
  store.addPersistentAgentAudienceMember(scope, agentA);
  store.addPersistentAgentAudienceMember(scope, agentB);

  assert.deepEqual(savedAudiences(), { [scope]: [agentA, agentB] });
});

test("adding an existing address lock preserves order and dedupes", async () => {
  const store = await loadStore(2);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA, agentB]);
  store.addPersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(savedAudiences(), { [scope]: [agentA, agentB] });
});

test("address locks can be removed independently", async () => {
  const store = await loadStore(3);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA, agentB]);

  store.removePersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(savedAudiences(), { [scope]: [agentB] });
});

test("removing final chip preserves an explicit empty scope", async () => {
  const store = await loadStore(4);
  const scope = `${ownerA}:channel-a:channel`;
  store.setPersistentAgentAudience(scope, [agentA]);
  store.removePersistentAgentAudienceMember(scope, agentA);

  assert.deepEqual(savedAudiences(), { [scope]: [] });
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
  store.initPersistentAgentAudienceStore("community-a");
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
  const renderCountBeforeTouch = renderCount;
  writes.length = 0;

  await React.act(async () => {
    store.setPersistentAgentAudience(touchedScope, [agentA]);
  });

  assert.equal(writes.length, 1);
  assert.equal(writes[0][0], storageKey());
  assert.deepEqual(JSON.parse(writes[0][1])[touchedScope], [agentA]);
  assert.equal(Object.keys(JSON.parse(writes[0][1])).at(-1), touchedScope);
  assert.equal(renderCount, renderCountBeforeTouch);

  writes.length = 0;
  await React.act(async () => {
    store.setPersistentAgentAudience(touchedScope, [agentA]);
  });
  assert.equal(writes.length, 0);
  assert.equal(renderCount, renderCountBeforeTouch);

  await React.act(async () => {
    store.setPersistentAgentAudience("scope-new", [agentB]);
  });
  const saved = savedAudiences();
  assert.deepEqual(saved[touchedScope], [agentA]);
  assert.equal(saved["scope-1"], undefined);
  assert.deepEqual(saved["scope-new"], [agentB]);
  await React.act(async () => root.unmount());
  dom.window.close();
});

test("community switches isolate and restore address locks", async () => {
  const store = await loadStore(12);
  const scope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "shared-channel-id",
  });

  store.setPersistentAgentAudience(scope, [agentA]);
  store.resetPersistentAgentAudienceStore();
  store.initPersistentAgentAudienceStore("community-b");
  assert.deepEqual(savedAudiences("community-a"), { [scope]: [agentA] });
  assert.deepEqual(store.getPersistentAgentAudienceSnapshot().audiences, {});

  store.setPersistentAgentAudience(scope, [agentB]);
  store.resetPersistentAgentAudienceStore();
  store.initPersistentAgentAudienceStore("community-a");
  assert.deepEqual(store.getPersistentAgentAudienceSnapshot().audiences, {
    [scope]: [agentA],
  });
  assert.deepEqual(savedAudiences("community-b"), { [scope]: [agentB] });
});

test("channel and thread composers share the channel audience scope", async () => {
  const store = await loadStore(7);
  const channelScope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
  });
  const threadScope = store.getPersistentAgentAudienceScope({
    ownerPubkey: ownerA,
    channelId: "channel-a",
    threadRootId: "root",
  });

  assert.equal(channelScope, `${ownerA}:channel-a:channel`);
  assert.equal(threadScope, channelScope);
});
