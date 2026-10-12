import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools";
import { BridgeCommandHandler, makeSeenSet } from "./commandHandler.ts";
import { BridgeCommander } from "./commander.ts";
import { BridgePublisher } from "./publisher.ts";
import { buildAgentsSnapshot, snapshotSignature } from "./snapshot.ts";
import { runTermHandoff, isHandoffReply, withEnvExports, TermHandoffError } from "./termHandoff.ts";
import {
  attachHandoffToTerminal,
  handoffAgentsFor,
  handoffHarnesses,
  parseHandoffResult,
  peekPendingTermHandoff,
  planAttach,
  setPendingTermHandoff,
  takePendingTermHandoff,
} from "./webTermHandoff.ts";
import { bridgeTags, buildEnvelope, cmdD, encryptEnvelope, generateSessionKey, ALL_COMMANDS, DESTRUCTIVE_COMMANDS } from "./protocol.ts";

const sk = new Uint8Array(32).fill(9);
const pk = getPublicKey(sk);
const AG = "ab".repeat(32);
const TH = "cd".repeat(32);
const K = generateSessionKey();

test("snapshot advertises channel membership for running agents only, capped and signed", () => {
  const body = buildAgentsSnapshot(
    [
      { pubkey: AG, name: "A", status: "running", channelIds: ["c2", "c1", "c1"] },
      { pubkey: "ee".repeat(32), name: "B", status: "stopped", channelIds: ["c1"] },
    ],
    [],
    () => undefined,
    1,
  );
  assert.deepEqual(body.agents.find((a) => a.pubkey === AG).channelIds, ["c1", "c2"]);
  assert.equal(body.agents.find((a) => a.name === "B").channelIds, undefined);
  const other = buildAgentsSnapshot([{ pubkey: AG, name: "A", status: "running", channelIds: ["c1"] }], [], () => undefined, 1);
  assert.notEqual(snapshotSignature(body), snapshotSignature(other));
});

test("heartbeat carries installed harnesses and the term.handoff cap", async () => {
  const published = [];
  const p = new BridgePublisher(
    {
      encryptToSelf: async (s) => s,
      signEvent: async (e) => e,
      publish: async (e) => void published.push(e),
    },
    { deviceId: "d", caps: ["term.handoff"], harnesses: () => [{ id: "claude", available: true }, { id: "codex", available: false }] },
  );
  await p.sendHeartbeat();
  const hb = published.find((e) => e.tags.some((t) => t[1]?.endsWith("/hb")));
  assert.ok(hb);
  assert.ok(ALL_COMMANDS.includes("term.handoff") && DESTRUCTIVE_COMMANDS.includes("term.handoff"));
});

test("web picker: agents in this channel and installed harnesses come from the desktop data", () => {
  const snap = {
    generatedAt: 1,
    agents: [
      { pubkey: AG, name: "A", status: "running", activeTurns: [], channelIds: ["c1"] },
      { pubkey: "ee".repeat(32), name: "B", status: "running", activeTurns: [], channelIds: ["c2"] },
      { pubkey: "ff".repeat(32), name: "C", status: "stopped", activeTurns: [], channelIds: ["c1"] },
    ],
  };
  assert.deepEqual(handoffAgentsFor(snap, "c1").map((a) => a.name), ["A"]);
  assert.deepEqual(handoffAgentsFor(null, "c1"), []);
  assert.equal(handoffHarnesses({}), null);
  const hs = handoffHarnesses({ harnesses: [{ id: "claude", available: true }, { id: "codex", available: false }] });
  assert.deepEqual(hs.map((h) => [h.id, h.available]), [["claude", true], ["codex", false]]);
  assert.match(hs[1].hint, /Codex/);
});

function mkHandler({ allowed = true, termHandoff, status = "running", channelIds = ["c1"] } = {}) {
  const results = [];
  const calls = [];
  const h = new BridgeCommandHandler({
    memberPubkey: pk,
    sessionKey: () => K,
    agents: () => new Map([[AG, { status, channelIds }]]),
    seen: makeSeenSet(() => [], () => {}),
    executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
    publishResult: async (id, b) => void results.push(b),
    agentManagementAllowed: () => allowed,
    termHandoff: termHandoff === null ? undefined : async (i) => (calls.push(i), { sid: "s1", launchCommand: "claude x" }),
    now: () => 1_000_000,
    verify: () => true,
  });
  return { h, results, calls };
}
const ev = (body) => {
  const env = buildEnvelope({ type: "term.handoff", kid: K.kid, dev: "web-x", seq: 1, ttlS: 60, body, id: crypto.randomUUID(), now: 1_000_000 });
  return finalizeEvent({ kind: 30078, content: encryptEnvelope(env, K.key), created_at: 1_000_000, tags: bridgeTags(cmdD(env.id), K.kid, env.exp) }, sk);
};
const good = { agentPubkey: AG, channelId: "c1", threadId: TH, harness: "claude" };

test("term.handoff runs only with Allow remote agent management on, then acks accepted → ok with the result", async () => {
  const on = mkHandler();
  assert.equal(await on.h.handle(ev(good)), "executed");
  assert.deepEqual(on.results.map((r) => r.status), ["accepted", "ok"]);
  assert.equal(on.results[1].result.sid, "s1");
  assert.deepEqual(on.calls, [good]);
  const off = mkHandler({ allowed: false });
  assert.equal(await off.h.handle(ev(good)), "dropped");
  assert.equal(off.results[0].code, "remote_disabled");
  assert.equal(off.calls.length, 0);
});

test("term.handoff validates agent state, membership, thread id and harness", async () => {
  for (const [opts, body, code] of [
    [{ status: "stopped" }, good, "agent_not_running"],
    [{ channelIds: ["other"] }, good, "not_member"],
    [{ channelIds: null }, good, "not_member"],
    [{}, { ...good, threadId: "nope" }, "bad_request"],
    [{}, { ...good, harness: "bash" }, "bad_request"],
    [{ termHandoff: null }, good, "unsupported"],
    [{}, { ...good, agentPubkey: "11".repeat(32) }, "unknown_agent"],
  ]) {
    const m = mkHandler(opts);
    await m.h.handle(ev(body));
    assert.equal(m.results.at(-1).code, code, JSON.stringify([opts, body]));
    assert.equal(m.calls.length, 0);
  }
});

test("executor error codes reach the web", async () => {
  const m = mkHandler({});
  const h = new BridgeCommandHandler({
    memberPubkey: pk,
    sessionKey: () => K,
    agents: () => new Map([[AG, { status: "running", channelIds: ["c1"] }]]),
    seen: makeSeenSet(() => [], () => {}),
    executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
    publishResult: async (id, b) => void m.results.push(b),
    termHandoff: async () => {
      throw new TermHandoffError("no_card", "slow agent");
    },
    now: () => 1_000_000,
    verify: () => true,
  });
  await h.handle(ev(good));
  assert.equal(m.results.at(-1).code, "no_card");
});

const card = { hula: "term-session", v: 1, name: "Fix bug", tool: "claude", sid: "sid-1", prompt: "p" };
function deps(over = {}) {
  const log = [];
  return {
    log,
    d: {
      agentName: () => "Stitch",
      channelName: () => "general",
      harnessAvailable: () => true,
      sendInstruction: async (i, c) => void log.push(["send", i.agentPubkey, c]),
      waitForCard: async () => card,
      openCard: async (cd, ctx) => (log.push(["open", cd.sid, ctx.channelName]), { launchCommand: "claude \"$(cat '/p')\"", extraEnv: { CLAUDE_CONFIG_DIR: "/cfg dir" } }),
      waitMs: 10,
      ...over,
    },
  };
}
test("desktop flow: instruction to the agent, card back, desktop Buzz Term opened, launch line returned", async () => {
  const { d, log } = deps();
  const r = await runTermHandoff(d, { agentPubkey: AG, channelId: "c1", threadId: TH, harness: "claude" });
  assert.equal(log[0][0], "send");
  assert.match(log[0][2], /@Stitch Summarize this thread for a Buzz Term handoff/);
  assert.match(log[0][2], /term_session_card/);
  assert.deepEqual(log[1], ["open", "sid-1", "general"]);
  assert.equal(r.sid, "sid-1");
  assert.match(r.launchCommand, /^export CLAUDE_CONFIG_DIR='\/cfg dir' && claude/);
});
test("desktop flow fails clearly: missing harness, no card, open failure, send failure", async () => {
  const i = { agentPubkey: AG, channelId: "c1", threadId: TH, harness: "codex" };
  await assert.rejects(runTermHandoff(deps({ harnessAvailable: () => false }).d, i), (e) => e.code === "harness_unavailable");
  await assert.rejects(runTermHandoff(deps({ waitForCard: async () => null }).d, i), (e) => e.code === "no_card");
  await assert.rejects(runTermHandoff(deps({ openCard: async () => null }).d, i), (e) => e.code === "open_failed");
  await assert.rejects(runTermHandoff(deps({ sendInstruction: async () => { throw new Error("x"); } }).d, i), (e) => e.code === "send_failed");
});
test("reply matching and env export helper", () => {
  const i = { agentPubkey: AG, threadId: TH };
  const e = (o = {}) => ({ pubkey: AG, created_at: 100, tags: [["e", TH, "", "reply"]], ...o });
  assert.ok(isHandoffReply(e(), i, 100));
  assert.ok(!isHandoffReply(e({ pubkey: "11".repeat(32) }), i, 100));
  assert.ok(!isHandoffReply(e({ created_at: 50 }), i, 100));
  assert.ok(!isHandoffReply(e({ tags: [["e", "other"]] }), i, 100));
  assert.equal(withEnvExports("claude x", {}), "claude x");
  assert.equal(withEnvExports("export CLAUDE_CONFIG_DIR='a' && claude", { CLAUDE_CONFIG_DIR: "a" }), "export CLAUDE_CONFIG_DIR='a' && claude");
  assert.equal(withEnvExports("c", { "bad key": "x" }), "c");
});

test("after ack: attach when remote terminal is usable, otherwise a clear message", () => {
  const host = (o = {}) => ({ id: "d", name: "Mac", url: "wss://x", ageS: 1, stale: false, supports: { agents: true, terminal: true }, hasDirect: true, sigTtlS: 3600, ...o });
  assert.deepEqual(planAttach(host(), true, "reachable"), { attach: true });
  assert.deepEqual(planAttach(host(), true, "checking"), { attach: true });
  const off = planAttach(host({ supports: { agents: true, terminal: false } }), true, "reachable");
  assert.equal(off.attach, false);
  assert.match(off.message, /started on your desktop/);
  assert.match(off.message, /Allow remote terminal is off/);
  const tail = planAttach(host(), true, "unreachable");
  assert.match(tail.message, /Tailscale/);
  assert.equal(planAttach(undefined, false, "unknown").attach, false);
});

test("hand-over store: one-shot, expires, result parsing, attach types the launch line", () => {
  const h = { sid: "s", name: "n", tool: "claude", launchCommand: "claude x" };
  setPendingTermHandoff(h, 1000);
  assert.deepEqual(peekPendingTermHandoff(2000), h);
  assert.deepEqual(takePendingTermHandoff(2000), h);
  assert.equal(takePendingTermHandoff(2000), null);
  setPendingTermHandoff(h, 1000);
  assert.equal(takePendingTermHandoff(1000 + 61_000), null);
  assert.equal(parseHandoffResult({ sid: "s", tool: "claude" }), null);
  assert.equal(parseHandoffResult({ sid: "s", tool: "bash", launchCommand: "x" }), null);
  assert.deepEqual(parseHandoffResult({ sid: "s", name: "n", tool: "codex", launchCommand: "x" }).tool, "codex");
  const writes = [];
  attachHandoffToTerminal({ write: (s) => writes.push(s) }, h, (fn) => fn());
  assert.deepEqual(writes, ["claude x\r"]);
});

test("web commander requires confirmation and desktop support for term.handoff", async () => {
  const client = { state: { heartbeat: { caps: ["term.handoff"] }, presence: "online", results: new Map() }, currentKey: () => K, expectStatus() {} };
  const sent = [];
  const c = new BridgeCommander(client, { signEvent: async (e) => e, publish: async (e) => void sent.push(e) }, "t");
  await assert.rejects(c.send("term.handoff", good), (e) => e.code === "confirmation_required");
  await c.send("term.handoff", good, { confirmed: true });
  assert.equal(sent.length, 1);
  client.state.heartbeat = { caps: [] };
  await assert.rejects(c.send("term.handoff", good, { confirmed: true }), (e) => e.code === "unsupported");
});
