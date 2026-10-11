import assert from "node:assert/strict";
import test from "node:test";
import { handleMgmt, MgmtError, mgmtErrorString } from "./directMgmt.ts";

const PK = "a".repeat(64);
const agent = (o = {}) => ({ pubkey: PK, name: "Ann", status: "stopped", runtime: null, model: null, provider: null, personaId: null, ...o });
function deps(over = {}) {
  const calls = [];
  const d = {
    calls,
    allowed: () => true,
    listAgents: async () => [agent()],
    startAgent: async (pk) => calls.push(["start", pk]),
    stopAgent: async (pk) => calls.push(["stop", pk]),
    sendMessage: async (...a) => calls.push(["message", ...a]),
    createAgent: async (i) => (calls.push(["create", i]), { agent: agent({ name: i.name }), spawnError: null }),
    updateAgent: async (i) => (calls.push(["update", i]), { agent: agent({ name: i.name ?? "Ann" }) }),
    runtimes: async () => [
      { id: "goose", label: "Goose", availability: "available", command: "goose-acp" },
      { id: "gone", label: "Gone", availability: "not_installed", command: null },
    ],
    personas: async () => [{ id: "p1", displayName: "Helper" }],
    ...over,
  };
  return d;
}
const code = (p) => p.then(() => null, (e) => e.code);

test("nothing runs while Allow remote agent management is off (even if the listener forwarded it)", async () => {
  const d = deps({ allowed: () => false });
  assert.equal(await code(handleMgmt("agents.list", {}, d)), "agents_disabled");
  assert.equal(await code(handleMgmt("agent.create", { name: "x" }, d)), "agents_disabled");
  assert.deepEqual(d.calls, []);
});

test("list shows every agent including stopped ones, with provider and model", async () => {
  const r = await handleMgmt("agents.list", {}, deps({ listAgents: async () => [agent({ provider: "anthropic", model: "m1" }), agent({ pubkey: "b".repeat(64), status: "running" })] }));
  assert.equal(r.agents.length, 2);
  assert.equal(r.agents[0].provider, "anthropic");
});

test("start/stop/message validate the agent and fields", async () => {
  const d = deps();
  await handleMgmt("agent.start", { pubkey: PK.toUpperCase() }, d);
  await handleMgmt("agent.stop", { pubkey: PK }, d);
  await handleMgmt("agent.message", { pubkey: PK, channelId: "c1", text: "hi" }, d);
  assert.deepEqual(d.calls, [["start", PK], ["stop", PK], ["message", PK, "c1", "hi"]]);
  assert.equal(await code(handleMgmt("agent.start", { pubkey: "zz" }, d)), "bad_request");
  assert.equal(await code(handleMgmt("agent.start", { pubkey: "c".repeat(64) }, d)), "unknown_agent");
  assert.equal(await code(handleMgmt("agent.message", { pubkey: PK, channelId: "c", text: "x".repeat(5000) }, d)), "bad_request");
  assert.equal(await code(handleMgmt("agent.message", { pubkey: PK, channelId: "", text: "x" }, d)), "bad_request");
});

test("create passes only allow-listed fields; runtime ids map to the discovered command; commands/env/prompts are never forwarded", async () => {
  const d = deps();
  await handleMgmt(
    "agent.create",
    { name: "  New  ", provider: "anthropic", model: "m", personaId: "p1", runtimeId: "goose", start: true, agentCommand: "rm -rf", envVars: { A: "b" }, systemPrompt: "evil", mcpCommand: "x", acpCommand: "y", agentArgs: ["z"] },
    d,
  );
  assert.deepEqual(d.calls[0][1], { name: "New", provider: "anthropic", model: "m", personaId: "p1", spawnAfterCreate: true, agentCommand: "goose-acp", harnessOverride: true });
  assert.equal(await code(handleMgmt("agent.create", { name: "" }, d)), "bad_request");
  assert.equal(await code(handleMgmt("agent.create", { name: "x\u0007" }, d)), "bad_request");
  assert.equal(await code(handleMgmt("agent.create", { name: "x", personaId: "nope" }, d)), "bad_request");
  assert.equal(await code(handleMgmt("agent.create", { name: "x", runtimeId: "gone" }, d)), "runtime_unavailable");
  assert.equal(await code(handleMgmt("agent.create", { name: "x", runtimeId: "../../bin/sh" }, d)), "runtime_unavailable");
});

test("rename and update: provider/model/runtime only, clearing works, restart hint for running agents", async () => {
  const d = deps();
  const r = await handleMgmt("agent.rename", { pubkey: PK, name: "Bea", model: "ignored" }, d);
  assert.deepEqual(d.calls[0][1], { pubkey: PK, name: "Bea" });
  assert.equal(r.agent.name, "Bea");
  await handleMgmt("agent.update", { pubkey: PK, provider: "", model: "m2", runtimeId: "goose", envVars: { A: "1" } }, d);
  assert.deepEqual(d.calls[1][1], { pubkey: PK, provider: null, model: "m2", agentCommand: "goose-acp", harnessOverride: true });
  assert.equal(await code(handleMgmt("agent.update", { pubkey: PK }, d)), "bad_request");
  const run = deps({ listAgents: async () => [agent({ status: "running" })] });
  assert.equal((await handleMgmt("agent.update", { pubkey: PK, model: "m3" }, run)).restartNeeded, true);
});

test("unknown operations are refused; errors serialize as '<code>: <message>'", async () => {
  assert.equal(await code(handleMgmt("agent.delete", { pubkey: PK }, deps())), "unknown_op");
  assert.equal(await code(handleMgmt("agent.exec", {}, deps())), "unknown_op");
  assert.equal(mgmtErrorString(new MgmtError("bad_request", "Name is required.")), "bad_request: Name is required.");
  assert.match(mgmtErrorString(new Error("boom")), /^failed: boom/);
});
