import assert from "node:assert/strict";
import test from "node:test";
import { getPublicKey, finalizeEvent, nip44 } from "nostr-tools";
import { BridgeCommandHandler, makeSeenSet } from "./commandHandler.ts";
import { BridgeCommander } from "./commander.ts";
import { BridgePublisher } from "./publisher.ts";
import { BridgeWebClient } from "./webClient.ts";
import { bridgeTags, buildEnvelope, cmdD, encryptEnvelope, generateSessionKey, nowS } from "./protocol.ts";

const sk = new Uint8Array(32).fill(7);
const pk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, pk);
const AG = "ab".repeat(32);
const K = generateSessionKey();

function setup(over = {}) {
  const calls = [];
  const results = [];
  const agents = new Map([[AG, { status: over.status ?? "running" }]]);
  const h = new BridgeCommandHandler({
    memberPubkey: pk,
    sessionKey: () => K,
    agents: () => agents,
    seen: makeSeenSet(() => [], () => {}),
    executors: {
      start: async (a) => void calls.push(["start", a]),
      stop: async (a) => void calls.push(["stop", a]),
      message: async (a, c, t) => void calls.push(["message", a, c, t]),
    },
    publishResult: async (id, b) => void results.push([id, b.status, b.code]),
    now: () => 1_000_000,
    ...over.deps,
  });
  return { h, calls, results, agents };
}

function cmdEvent(type, body, o = {}) {
  const now = o.now ?? 1_000_000;
  const env = buildEnvelope({ type, kid: o.kid ?? K.kid, dev: "web-x", seq: 1, ttlS: o.ttl ?? 60, body, id: o.id ?? crypto.randomUUID(), now: o.envTs ?? now });
  const key = o.key ?? K.key;
  return finalizeEvent(
    {
      kind: 30078,
      content: encryptEnvelope(env, key),
      created_at: o.createdAt ?? now,
      tags: o.tags ?? bridgeTags(cmdD(o.dTag ?? env.id), o.tagKid ?? K.kid, env.exp),
    },
    o.sk ?? sk,
  );
}

test("valid commands run, ack first then final; start is idempotent for a running agent", async () => {
  const { h, calls, results } = setup();
  assert.equal(await h.handle(cmdEvent("agent.stop", { agentPubkey: AG })), "executed");
  assert.deepEqual(calls, [["stop", AG]]);
  assert.deepEqual(results.map((r) => r[1]), ["accepted", "ok"]);
  await h.handle(cmdEvent("agent.message", { agentPubkey: AG, channelId: "c1", text: "hi" }));
  assert.deepEqual(calls[1], ["message", AG, "c1", "hi"]);
  const s = setup({ status: "running" });
  await s.h.handle(cmdEvent("agent.start", { agentPubkey: AG }));
  assert.equal(s.calls.length, 0); // already running: no re-spawn
  const t = setup({ status: "stopped" });
  await t.h.handle(cmdEvent("agent.start", { agentPubkey: AG }));
  assert.deepEqual(t.calls, [["start", AG]]);
});

test("replay: same command id never re-executes; the stored result is republished", async () => {
  const { h, calls, results } = setup();
  const ev = cmdEvent("agent.stop", { agentPubkey: AG });
  await h.handle(ev);
  const replay = finalizeEvent({ kind: 30078, content: ev.content, created_at: 1_000_000, tags: [...ev.tags, ["x", "1"]] }, sk);
  assert.equal(await h.handle(replay), "replayed");
  assert.equal(calls.length, 1);
  assert.equal(results.at(-1)[1], "ok");
  // previous-run replay (id persisted, result unknown): dropped silently
  const seen = makeSeenSet(() => [], () => {});
  const ev2 = cmdEvent("agent.stop", { agentPubkey: AG });
  seen.add(JSON.parse(JSON.stringify(ev2.tags.find((t) => t[0] === "d")[1].split("/").pop())));
  const s2 = setup({ deps: { seen } });
  assert.equal(await s2.h.handle(ev2), "dropped");
  assert.equal(s2.calls.length, 0);
});

test("rejected: foreign author, forged/tampered signature, wrong kid, wrong key, expired, future, too-long ttl, id mismatch, unknown type", async () => {
  const { h, calls, results } = setup();
  const other = new Uint8Array(32).fill(9);
  const body = { agentPubkey: AG };
  const bad = [];
  bad.push(cmdEvent("agent.stop", body, { sk: other })); // foreign author
  const tampered = cmdEvent("agent.stop", body);
  bad.push({ ...JSON.parse(JSON.stringify(tampered)), sig: tampered.sig.replace(/^../, tampered.sig.startsWith("00") ? "11" : "00") }); // bad signature
  bad.push(cmdEvent("agent.stop", body, { tagKid: "deadbeefdeadbeef" })); // unknown kid
  bad.push(cmdEvent("agent.stop", body, { key: generateSessionKey().key })); // wrong key
  bad.push(cmdEvent("agent.stop", body, { envTs: 1_000_000 - 500 })); // expired envelope (ttl 60)
  bad.push(cmdEvent("agent.stop", body, { createdAt: 1_000_000 - 10_000 })); // stale event
  bad.push(cmdEvent("agent.stop", body, { envTs: 1_000_000 + 3600, ttl: 60 })); // from the future
  bad.push(cmdEvent("agent.stop", body, { ttl: 3600 })); // ttl > 120
  bad.push(cmdEvent("agent.stop", body, { dTag: "some-other-id-1234" })); // d-tag/id mismatch
  bad.push(cmdEvent("agent.nuke", body)); // unknown type
  for (const [i, ev] of bad.entries()) assert.equal(await h.handle(ev), "dropped", `case ${i}`);
  assert.equal(calls.length, 0);
  assert.equal(results.length, 0); // nothing answered to unauthenticated senders
});

test("known-agent rule and body validation: unknown agent errors without executing; bad message rejected", async () => {
  const { h, calls, results } = setup();
  await h.handle(cmdEvent("agent.stop", { agentPubkey: "cd".repeat(32) }));
  assert.equal(calls.length, 0);
  assert.equal(results.at(-1)[2], "unknown_agent");
  await h.handle(cmdEvent("agent.message", { agentPubkey: AG, channelId: "", text: "x" }));
  await h.handle(cmdEvent("agent.message", { agentPubkey: AG, channelId: "c", text: "   " }));
  await h.handle(cmdEvent("agent.message", { agentPubkey: AG, channelId: "c", text: "x".repeat(5000) }));
  assert.equal(calls.length, 0);
  const stopped = setup({ status: "stopped" });
  await stopped.h.handle(cmdEvent("agent.message", { agentPubkey: AG, channelId: "c", text: "hi" }));
  assert.equal(stopped.calls.length, 0);
  assert.equal(stopped.results.at(-1)[2], "agent_not_running");
});

test("prompt.approve/deny are acknowledged as unsupported and never execute anything", async () => {
  const { h, calls, results } = setup();
  await h.handle(cmdEvent("prompt.approve", { agentPubkey: AG, promptId: "p1" }));
  await h.handle(cmdEvent("prompt.deny", { agentPubkey: AG, promptId: "p1" }));
  assert.equal(calls.length, 0);
  assert.deepEqual(results.filter((r) => r[1] === "error").map((r) => r[2]), ["unsupported", "unsupported"]);
});

test("executor failure becomes an error result, not a crash", async () => {
  const { h, results } = setup({ deps: { executors: { start: async () => {}, stop: async () => { throw new Error("boom"); }, message: async () => {} } } });
  await h.handle(cmdEvent("agent.stop", { agentPubkey: AG }));
  assert.deepEqual(results.at(-1).slice(1), ["error", "failed"]);
});

// ---- web commander + full loop with the publisher/web client
function loop() {
  const published = [];
  const io = {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) => finalizeEvent({ kind: e.kind, content: e.content, created_at: e.createdAt, tags: e.tags }, sk),
    publish: async (s) => void published.push(s),
  };
  const timers = [];
  io.setTimeout = (fn) => (timers.push(fn), timers.length);
  io.clearTimeout = () => {};
  const pub = new BridgePublisher(io, { deviceId: "d", caps: ["agents.snapshot", "agent.start", "agent.stop", "agent.message"] });
  const client = new BridgeWebClient(pk, { decryptFromSelf: async (c) => nip44.v2.decrypt(c, selfKey), cacheGet: () => null, cachePut() {} });
  const wire = [];
  const commander = new BridgeCommander(client, { signEvent: io.signEvent, publish: async (s) => void wire.push(s) }, "tab1");
  return { pub, client, commander, published, wire, run: async () => { for (const f of timers.splice(0)) f(); await new Promise((r) => setTimeout(r, 30)); } };
}

test("commander: destructive commands require explicit confirmation; offline/unsupported refused; full round trip to the desktop handler", async () => {
  const L = loop();
  L.pub.start();
  L.pub.update({ agents: [{ pubkey: AG, name: "A", status: "running", activeTurns: [] }], generatedAt: 1 });
  await L.run();
  for (const e of L.published) await L.client.ingest(e);
  const body = { agentPubkey: AG };
  await assert.rejects(L.commander.send("agent.stop", body), /confirmation/i);
  await assert.rejects(L.commander.send("agent.message", { ...body, channelId: "c", text: "x" }, { confirmed: false }), /confirmation/i);
  await assert.rejects(L.commander.send("prompt.approve", { ...body, promptId: "p" }, { confirmed: true }), /doesn't support/);
  assert.equal(L.wire.length, 0);
  // run it end to end: web -> desktop handler -> publisher result -> web client
  const calls = [];
  const handler = new BridgeCommandHandler({
    memberPubkey: pk,
    sessionKey: () => ({ kid: L.pub.kid, key: L.pub.sessionKey }),
    agents: () => L.pub.agents(),
    seen: makeSeenSet(() => [], () => {}),
    executors: { start: async () => {}, stop: async (a) => void calls.push(a), message: async () => {} },
    publishResult: (id, b) => L.pub.publishResult(id, b),
  });
  const id = await L.commander.send("agent.stop", body, { confirmed: true, label: "Stop" });
  assert.equal(L.commander.statusOf(id), "pending");
  assert.equal(await handler.handle(L.wire[0]), "executed");
  assert.deepEqual(calls, [AG]);
  for (const e of L.published) await L.client.ingest(e);
  assert.equal(L.commander.statusOf(id), "ok");
  assert.ok(!L.wire[0].content.includes(AG)); // command payload is encrypted
  // timeout when nothing answers
  const id2 = await L.commander.send("agent.stop", body, { confirmed: true });
  assert.equal(L.commander.statusOf(id2, nowS() + 500), "timeout");
});

test("remote agent management switch off: agent.* commands are refused with remote_disabled and nothing runs", async () => {
  let allowed = false;
  const { h, calls, results } = setup({ deps: { agentManagementAllowed: () => allowed } });
  await h.handle(cmdEvent("agent.stop", { agentPubkey: AG }));
  assert.deepEqual(calls, []);
  assert.deepEqual(results.map((r) => [r[1], r[2]]), [["error", "remote_disabled"]]);
  allowed = true;
  await h.handle(cmdEvent("agent.stop", { agentPubkey: AG }));
  assert.deepEqual(calls, [["stop", AG]]);
});
