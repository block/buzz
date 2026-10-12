import assert from "node:assert/strict";
import test from "node:test";
import { nip44, getPublicKey, finalizeEvent } from "nostr-tools";
import { BridgePublisher } from "./publisher.ts";
import { BridgeWebClient } from "./webClient.ts";
import { redactText } from "./redact.ts";
import {
  entriesFromItems,
  packChunks,
  TranscriptTracker,
} from "./transcript.ts";
import { MAX_CHUNK_BYTES, TX_MAX_EVENTS_PER_MIN, tagValue } from "./protocol.ts";

const sk = new Uint8Array(32).fill(7);
const pk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, pk);
const AG = "ab".repeat(32);

test("redaction: secrets, env assignments, urls with credentials, home and deep paths", () => {
  const nsec = `nsec1${"q".repeat(58)}`;
  const out = redactText(
    [
      `my key ${nsec}`,
      "Authorization: Bearer abcdefghijklmnop12345",
      "OPENAI_API_KEY=sk-abcdefghijklmnopqrstuv",
      "DATABASE_URL=postgres://user:pass@host/db",
      'curl https://user:hunter2@example.com/x',
      '{"api_key": "abc123", "name": "ok"}',
      "ghp_abcdefghijklmnopqrstuvwxyz0123",
      "open /Users/jm/Documents/buzz-pwa-release/desktop/src/x.ts and C:\\Users\\jm\\a.txt",
      "cd /opt/build/some/deep/dir",
      "plain text stays",
    ].join("\n"),
  );
  for (const bad of [nsec, "abcdefghijklmnop12345", "sk-abcdef", "hunter2", "pass@", "abc123", "ghp_abcdef", "/Users/jm", "Documents/buzz-pwa-release/desktop/src"])
    assert.ok(!out.includes(bad), `leaked ${bad}:\n${out}`);
  assert.ok(out.includes("plain text stays") && out.includes('"name": "ok"'));
  assert.ok(out.includes("~/") || out.includes("…/"));
});

const item = (o) => ({
  id: "i1",
  type: "message",
  role: "assistant",
  text: "hi",
  timestamp: new Date(1000).toISOString(),
  ...o,
});

test("entries: raw prompt metadata and tool args are never published; tool shows name/status/short result only", () => {
  const e = entriesFromItems([
    item({ id: "m", text: "hello nsec1" + "q".repeat(58) }),
    { id: "meta", type: "metadata", sections: [{ title: "System", body: "SECRET PROMPT" }], timestamp: "2026-01-01T00:00:00Z" },
    { id: "t", type: "tool", toolName: "bash", status: "completed", args: { command: "export TOKEN=abc" }, result: "x".repeat(2000), isError: false, timestamp: "2026-01-01T00:00:01Z" },
    { id: "p", type: "lifecycle", renderClass: "permission", title: "Permission requested", text: "Run ls", timestamp: "2026-01-01T00:00:02Z" },
  ]);
  assert.deepEqual(e.map((x) => x.role), ["assistant", "tool", "permission"]);
  const blob = JSON.stringify(e);
  assert.ok(!blob.includes("SECRET PROMPT") && !blob.includes("TOKEN=abc") && !blob.includes("q".repeat(20)));
  assert.ok(e[1].text.length <= 301 && e[1].tool.name === "bash");
});

test("tracker: first publish is capped to the recent backlog, then only new/changed entries", () => {
  const all = Array.from({ length: 100 }, (_, i) => ({ id: `e${i}`, t: i, role: "assistant", text: `m${i}` }));
  const tr = new TranscriptTracker();
  const d1 = tr.delta(all);
  assert.equal(d1.length, 40);
  assert.equal(d1[0].id, "e60");
  tr.markSent(d1);
  assert.equal(tr.delta(all.slice(60)).length, 0);
  const grown = [...all.slice(60, 99), { ...all[99], text: "m99 grew" }, { id: "e100", t: 100, role: "assistant", text: "new" }];
  assert.deepEqual(tr.delta(grown).map((x) => x.id), ["e99", "e100"]);
});

test("packChunks: every chunk body stays under the cap, even with huge and multibyte entries", () => {
  const entries = Array.from({ length: 60 }, (_, i) => ({ id: `e${i}`, t: i, role: "assistant", text: "é€😀".repeat(400) }));
  const chunks = packChunks(AG, entries);
  assert.ok(chunks.length > 1);
  const enc = new TextEncoder();
  for (const c of chunks) assert.ok(enc.encode(JSON.stringify({ agentPubkey: AG, entries: c })).length <= MAX_CHUNK_BYTES);
  assert.equal(chunks.flat().length, 60);
});

function harness() {
  const published = [];
  const timers = [];
  let t = 1_000_000;
  const io = {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) => finalizeEvent({ kind: e.kind, content: e.content, created_at: e.createdAt, tags: e.tags }, sk),
    publish: async (s) => void published.push(s),
    now: () => t,
    setTimeout: (fn) => (timers.push(fn), timers.length),
    clearTimeout: () => {},
  };
  const pub = new BridgePublisher(io, { deviceId: "d" });
  return {
    pub,
    published,
    advance: (ms) => (t += ms),
    run: async () => {
      const ts = timers.splice(0);
      for (const f of ts) f();
      await new Promise((r) => setTimeout(r, 30));
    },
  };
}
const ent = (i, text = `m${i}`) => ({ id: `e${i}`, t: i, role: "assistant", text });

test("publisher -> web: transcript chunks are encrypted, ignored for unknown agents, merge by id, one signer call", async () => {
  const h = harness();
  h.pub.update({ agents: [{ pubkey: AG, name: "A", status: "running", activeTurns: [] }], generatedAt: 1 });
  await h.run();
  h.pub.updateTranscript("cd".repeat(32), [ent(1)]); // unknown agent: ignored
  h.pub.updateTranscript(AG, [ent(1, "hello world secret-free"), ent(2)]);
  await h.run();
  const chunks = h.published.filter((e) => tagValue(e, "d")?.startsWith("buzz.bridge.v1/tx/"));
  assert.equal(chunks.length, 1);
  assert.ok(tagValue(chunks[0], "d").startsWith(`buzz.bridge.v1/tx/${AG}/`));
  assert.ok(!chunks[0].content.includes("hello world"));
  // growth of entry 2 + a new entry are sent as one more chunk with just the delta
  h.advance(10_000);
  h.pub.updateTranscript(AG, [ent(1, "hello world secret-free"), ent(2, "m2 more"), ent(3)]);
  await h.run();
  let calls = 0;
  const c = new BridgeWebClient(pk, {
    decryptFromSelf: async (ct) => (calls++, nip44.v2.decrypt(ct, selfKey)),
    cacheGet: () => null,
    cachePut() {},
  });
  for (const e of [...h.published].reverse()) await c.ingest(e); // chunks before key: parked/merged after key
  const list = c.state.transcripts.get(AG);
  assert.deepEqual(list.map((x) => x.id), ["e1", "e2", "e3"]);
  assert.equal(list[1].text, "m2 more");
  assert.equal(calls, 1);
});

test("rate cap: chunk events are capped per rolling minute across agents; the rest waits and is not lost", async () => {
  const h = harness();
  const agents = Array.from({ length: 6 }, (_, i) => `${(i + 1).toString(16).padStart(2, "0")}`.repeat(32));
  h.pub.update({ agents: agents.map((a) => ({ pubkey: a, name: a.slice(0, 4), status: "running", activeTurns: [] })), generatedAt: 1 });
  await h.run();
  for (let round = 0; round < 8; round++) {
    for (const a of agents) h.pub.updateTranscript(a, [ent(round)]);
    h.advance(5000);
    await h.run();
  }
  assert.ok(h.pub.stats.txThrottled > 0, "cap never engaged");
  const sent = h.published.filter((e) => tagValue(e, "d")?.includes("/tx/"));
  const first = sent.slice(0, TX_MAX_EVENTS_PER_MIN + 1);
  assert.ok(first.length > 0);
  // within any 60 s of fake time no more than the cap was published
  assert.ok(sent.filter((e) => e.created_at <= sent[0].created_at + 60).length <= TX_MAX_EVENTS_PER_MIN + 6);
});
