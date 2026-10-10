import assert from "node:assert/strict";
import test from "node:test";
import { nip44, getPublicKey, finalizeEvent } from "nostr-tools";
import { BridgePublisher } from "./publisher.ts";
import { BridgeWebClient } from "./webClient.ts";
import { buildAgentsSnapshot } from "./snapshot.ts";
import {
  decryptEnvelope,
  generateSessionKey,
  buildEnvelope,
  encryptEnvelope,
  presenceOf,
  nowS,
  OFFLINE_AFTER_S,
  tagValue,
} from "./protocol.ts";

const sk = new Uint8Array(32).fill(7);
const pk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, pk);
const agent = (o = {}) => ({
  pubkey: "aa".repeat(32),
  name: "Opus",
  status: "running",
  activeTurns: [],
  ...o,
});

function harness() {
  const published = [];
  const timers = [];
  let t = 1_000_000;
  const io = {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) =>
      finalizeEvent(
        {
          kind: e.kind,
          content: e.content,
          created_at: e.createdAt,
          tags: e.tags,
        },
        sk,
      ),
    publish: async (s) => {
      published.push(s);
    },
    now: () => t,
    setTimeout: (fn) => {
      timers.push(fn);
      return timers.length;
    },
    clearTimeout: () => {},
  };
  const pub = new BridgePublisher(io, { deviceId: "dev1" });
  return {
    pub,
    published,
    timers,
    advance: (ms) => {
      t += ms;
    },
    runTimers: async () => {
      const ts = timers.splice(0);
      for (const f of ts) f();
      await new Promise((r) => setTimeout(r, 20));
    },
  };
}
function client() {
  let signerCalls = 0;
  const cache = new Map();
  const c = new BridgeWebClient(pk, {
    decryptFromSelf: async (ct) => {
      signerCalls++;
      return nip44.v2.decrypt(ct, selfKey);
    },
    cacheGet: (id) => cache.get(id) ?? null,
    cachePut: (id, j) => cache.set(id, j),
  });
  return { c, calls: () => signerCalls, cache };
}

test("snapshot projection: no secrets, running first, current task from turns", () => {
  const s = buildAgentsSnapshot(
    [
      agent({ name: "B", status: "stopped" }),
      agent({
        pubkey: "bb".repeat(32),
        name: "A",
        envVars: { SECRET: "x" },
        systemPrompt: "p",
      }),
    ],
    [{ channelId: "c1", anchorAt: 5000, agentPubkeys: ["BB".repeat(32)] }],
    (id) => (id === "c1" ? "general" : undefined),
    1,
  );
  assert.equal(s.agents[0].name, "A");
  assert.equal(s.agents[0].currentTask, "Working in general");
  assert.ok(
    !JSON.stringify(s).includes("SECRET") &&
      !JSON.stringify(s).includes("systemPrompt"),
  );
});

test("end to end: key wrap + snapshot + heartbeat decrypt with ONE signer call, no plaintext on the wire", async () => {
  const h = harness();
  h.pub.start();
  h.pub.update({ agents: [agent()], generatedAt: 1 });
  await h.runTimers();
  await new Promise((r) => setTimeout(r, 20));
  assert.ok(h.published.length >= 3, `published ${h.published.length}`);
  for (const e of h.published)
    assert.ok(
      !e.content.includes("Opus") &&
        e.tags.some((t) => t[0] === "t" && t[1] === "buzz-bridge"),
    );
  const { c, calls } = client();
  // deliver payloads BEFORE the key: must park, not call the signer
  for (const e of [...h.published].reverse()) await c.ingest(e);
  assert.equal(c.state.snapshot?.agents[0].name, "Opus");
  assert.equal(c.state.presence, "online");
  assert.equal(calls(), 1);
  // many more payloads: still one signer call
  for (let i = 0; i < 5; i++) {
    h.advance(6000);
    h.pub.update({ agents: [agent({ name: `Opus${i}` })], generatedAt: i });
    await h.runTimers();
  }
  for (const e of h.published) await c.ingest(e);
  assert.equal(calls(), 1);
  assert.equal(c.state.snapshot.agents[0].name, "Opus4");
});

test("unchanged state is not republished; change is rate limited/coalesced", async () => {
  const h = harness();
  h.pub.update({ agents: [agent()], generatedAt: 1 });
  await h.runTimers();
  const n = h.pub.stats.snapshots;
  h.pub.update({ agents: [agent()], generatedAt: 2 });
  assert.equal(h.timers.length, 0);
  h.pub.update({ agents: [agent({ status: "stopped" })], generatedAt: 3 });
  h.pub.update({
    agents: [agent({ status: "stopped", name: "Z" })],
    generatedAt: 4,
  });
  assert.equal(h.timers.length, 1);
  await h.runTimers();
  assert.equal(h.pub.stats.snapshots, n + 1);
});

test("security: foreign author, tampered/expired/replayed/unknown-key payloads are ignored", async () => {
  const h = harness();
  h.pub.update({ agents: [agent()], generatedAt: 1 });
  await h.runTimers();
  const { c } = client();
  const other = new Uint8Array(32).fill(9);
  const forged = finalizeEvent(
    {
      kind: 30078,
      content: h.published[1].content,
      created_at: nowS(),
      tags: h.published[1].tags,
    },
    other,
  );
  await c.ingest(forged);
  assert.equal(c.state.snapshot, null);
  const key = h.published.find((e) => tagValue(e, "d").endsWith("/key"));
  await c.ingest(key);
  const snap = h.published.find((e) => tagValue(e, "d").endsWith("/agents"));
  await c.ingest(snap);
  assert.equal(c.state.snapshot.agents.length, 1);
  // replay of an older seq (fresh event id, same envelope seq) is dropped
  const before = c.state;
  await c.ingest(
    finalizeEvent(
      {
        kind: 30078,
        content: snap.content,
        created_at: nowS() + 1,
        tags: snap.tags,
      },
      sk,
    ),
  );
  assert.equal(c.state, before);
  // wrong key => undecryptable => dropped
  const bad = generateSessionKey();
  const env = buildEnvelope({
    type: "agents.snapshot",
    kid: bad.kid,
    dev: "x",
    seq: 1,
    ttlS: 60,
    body: { agents: [], generatedAt: 1 },
  });
  assert.equal(
    decryptEnvelope(encryptEnvelope(env, bad.key), generateSessionKey().key),
    null,
  );
  // expired envelope
  const old = buildEnvelope({
    type: "agents.snapshot",
    kid: "k",
    dev: "x",
    seq: 1,
    ttlS: 10,
    body: {},
    now: nowS() - 100,
  });
  const { c: c2 } = client();
  await c2.ingest(key);
  const oldEv = finalizeEvent(
    {
      kind: 30078,
      content: encryptEnvelope(
        { ...old, kid: h.pub.kid },
        c2.keys?.get?.(h.pub.kid) ?? generateSessionKey().key,
      ),
      created_at: nowS(),
      tags: snap.tags,
    },
    sk,
  );
  await c2.ingest(oldEv);
  assert.equal(c2.state.snapshot, null);
});

test("presence: offline after the heartbeat goes stale, unknown before any", () => {
  const n = nowS();
  assert.equal(presenceOf(null, n), "unknown");
  assert.equal(presenceOf(n - 10, n), "online");
  assert.equal(presenceOf(n - OFFLINE_AFTER_S - 1, n), "offline");
});

test("desktop restart (new key, seq restarts) is accepted; cached key avoids a signer call on reload", async () => {
  const h1 = harness();
  h1.pub.start();
  await new Promise((r) => setTimeout(r, 20));
  const { c, calls, cache } = client();
  for (const e of h1.published) await c.ingest(e);
  assert.equal(calls(), 1);
  const h2 = harness();
  h2.pub.start();
  await new Promise((r) => setTimeout(r, 20));
  for (const e of h2.published) await c.ingest(e);
  assert.equal(calls(), 2);
  assert.equal(c.state.presence, "online");
  // simulated page reload with sessionStorage cache: no signer call
  let reloadCalls = 0;
  const c3 = new BridgeWebClient(pk, {
    decryptFromSelf: async () => {
      reloadCalls++;
      return "";
    },
    cacheGet: (id) => cache.get(id) ?? null,
    cachePut() {},
  });
  for (const e of h2.published) await c3.ingest(e);
  assert.equal(reloadCalls, 0);
  assert.equal(c3.state.presence, "online");
});
