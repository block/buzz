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

// ── desktop command proxy ("cmd" op) ─────────────────────────────────────────
const rawAgent = (o = {}) => ({ pubkey: PK, name: "Ann", status: "stopped", env_vars: { OPENAI_API_KEY: "sk-secret" }, system_prompt: "secret prompt", ...o });
function cmdDeps(over = {}) {
  const raw = [];
  const d = deps({
    ownerPubkey: "e".repeat(64),
    invokeRaw: async (cmd, args) => {
      raw.push([cmd, args]);
      if (cmd === "list_managed_agents") return [rawAgent(), rawAgent({ pubkey: "b".repeat(64), status: "running" })];
      if (cmd === "discover_acp_providers") return [{ id: "goose", command: "goose-acp", availability: "available", source: "builtin" }, { id: "mine", command: "/tmp/x", availability: "available", source: "custom", definition_env: { K: "v" } }];
      if (cmd === "create_managed_agent") return { agent: rawAgent({ name: args.input.name }), private_key_nsec: "nsec1secret", spawn_error: null };
      if (cmd === "update_managed_agent") return { agent: rawAgent() };
      if (cmd === "get_global_agent_config") return { env_vars: { A: "secret" }, provider: "anthropic", model: "m", preferred_runtime: null };
      return null;
    },
    ...over,
  });
  d.raw = raw;
  return d;
}

test("cmd: reads return the desktop's raw answer with secrets removed", async () => {
  const d = cmdDeps();
  const agents = await handleMgmt("cmd", { name: "list_managed_agents" }, d);
  assert.equal(agents.length, 2);
  assert.deepEqual(agents[0].env_vars, { OPENAI_API_KEY: "" });
  assert.equal(agents[0].system_prompt, "secret prompt", "prompts are user content: readable so the edit dialog doesn't wipe them");
  const rt = await handleMgmt("cmd", { name: "discover_acp_providers" }, d);
  assert.deepEqual(rt[1].definition_env, { K: "" });
  const g = await handleMgmt("cmd", { name: "get_global_agent_config" }, d);
  assert.deepEqual(g.env_vars, { A: "" });
  assert.equal(g.provider, "anthropic");
  assert.ok(!JSON.stringify([rt, g]).includes("secret"));
  assert.ok(!JSON.stringify(agents).includes("sk-secret"));
});

test("cmd: switch off, unknown names and a missing executor are refused", async () => {
  assert.equal(await code(handleMgmt("cmd", { name: "list_managed_agents" }, cmdDeps({ allowed: () => false }))), "agents_disabled");
  const d = cmdDeps();
  for (const name of ["delete_managed_agent", "get_identity", "delete_persona", "sign_event", "export_agent_secrets"]) assert.equal(await code(handleMgmt("cmd", { name }, d)), "unknown_op");
  assert.equal(await code(handleMgmt("cmd", { name: "list_managed_agents" }, deps())), "unknown_op");
  assert.deepEqual(d.raw, []);
});

test("cmd: create forwards only allow-listed fields, requires an installed non-custom harness, and never returns the key", async () => {
  const d = cmdDeps();
  const r = await handleMgmt("cmd", { name: "create_managed_agent", args: { input: { name: " New ", model: "m", provider: "p", personaId: "p1", spawnAfterCreate: true, agentCommand: "goose-acp", harnessOverride: true, envVars: { A: "b" }, systemPrompt: "be kind", agentArgs: ["x"], mcpCommand: "y", acpCommand: "z", relayUrl: "wss://evil" } } }, d);
  const call = d.raw.find((c) => c[0] === "create_managed_agent")[1].input;
  assert.deepEqual(call, { name: "New", model: "m", provider: "p", personaId: "p1", systemPrompt: "be kind", spawnAfterCreate: true, agentCommand: "goose-acp", harnessOverride: true });
  assert.equal(r.private_key_nsec, "");
  assert.ok(!JSON.stringify(r).includes("sk-secret") && !JSON.stringify(r).includes("nsec1secret"));
  assert.equal(await code(handleMgmt("cmd", { name: "create_managed_agent", args: { input: { name: "x", agentCommand: "/tmp/x" } } }, d)), "runtime_unavailable");
  assert.equal(await code(handleMgmt("cmd", { name: "create_managed_agent", args: { input: { name: "" } } }, d)), "bad_request");
});

test("cmd: update and start/stop validate keys; start carries the signed-in member as scope guard", async () => {
  const d = cmdDeps();
  await handleMgmt("cmd", { name: "update_managed_agent", args: { input: { pubkey: PK.toUpperCase(), model: "", envVars: { A: "b" }, systemPrompt: "x" } } }, d);
  assert.deepEqual(d.raw.find((c) => c[0] === "update_managed_agent")[1].input, { pubkey: PK, model: null, systemPrompt: "x" });
  assert.equal(await code(handleMgmt("cmd", { name: "update_managed_agent", args: { input: { pubkey: PK } } }, d)), "bad_request");
  assert.equal(await code(handleMgmt("cmd", { name: "stop_managed_agent", args: { pubkey: "zz" } }, d)), "bad_request");
  await handleMgmt("cmd", { name: "start_managed_agent", args: { pubkey: PK } }, d);
  assert.equal(d.raw.find((c) => c[0] === "start_managed_agent")[1].expectedSignerPubkey, "e".repeat(64));
});

test("cmd: access fields are validated (mode, hex keys, size)", async () => {
  const d = cmdDeps();
  const ok = await handleMgmt("cmd", { name: "update_managed_agent", args: { input: { pubkey: PK, respondTo: "allowlist", respondToAllowlist: ["B".repeat(64)] } } }, d);
  assert.ok(ok);
  assert.deepEqual(d.raw.at(-1)[1].input, { pubkey: PK, respondTo: "allowlist", respondToAllowlist: ["b".repeat(64)] });
  for (const bad of [{ respondTo: "root" }, { respondTo: "nobody" }, { respondToAllowlist: ["zz"] }, { respondToAllowlist: Array(501).fill(PK) }, { respondToAllowlist: "x" }])
    assert.equal(await code(handleMgmt("cmd", { name: "update_managed_agent", args: { input: { pubkey: PK, ...bad } } }, d)), "bad_request");
});

test("cmd: personas (the Add agent dialog) forward only allow-listed fields and a non-custom harness id; prompts are bounded", async () => {
  const d = cmdDeps({ invokeRaw: undefined });
  const raw = [];
  d.invokeRaw = async (cmd, args) => {
    raw.push([cmd, args]);
    if (cmd === "discover_acp_providers") return [{ id: "goose", command: "goose-acp", availability: "available", source: "builtin" }, { id: "mine", command: "/tmp/x", availability: "available", source: "custom" }];
    return { id: "p9", display_name: "Helper", system_prompt: "be nice", env_vars: { K: "secret" } };
  };
  const r = await handleMgmt("cmd", { name: "create_persona", args: { input: { displayName: " Helper ", systemPrompt: "be nice", runtime: "goose", model: "m", envVars: { A: "b" }, acpCommand: "rm", catalogSource: { owner_pubkey: "x" }, behavior: { respondTo: "allowlist", respondToAllowlist: [PK], parallelism: 2, sessionPolicy: "thread" } } } }, d);
  assert.deepEqual(raw.find((c) => c[0] === "create_persona")[1].input, { displayName: "Helper", systemPrompt: "be nice", model: "m", runtime: "goose", behavior: { respondTo: "allowlist", respondToAllowlist: [PK], parallelism: 2, sessionPolicy: "thread" } });
  assert.deepEqual(r.env_vars, { K: "" });
  assert.equal(await code(handleMgmt("cmd", { name: "create_persona", args: { input: { displayName: "x", systemPrompt: "", runtime: "mine" } } }, d)), "runtime_unavailable");
  assert.equal(await code(handleMgmt("cmd", { name: "update_persona", args: { input: { id: "p9", displayName: "x", systemPrompt: "y".repeat(20001) } } }, d)), "bad_request");
  assert.equal(await code(handleMgmt("cmd", { name: "update_persona", args: { input: { displayName: "x", systemPrompt: "" } } }, d)), "bad_request");
  await handleMgmt("cmd", { name: "update_persona", args: { input: { id: "p9", displayName: "H", systemPrompt: "z" } } }, d);
  assert.equal(raw.at(-1)[1].input.id, "p9");
});

test("cmd: agent defaults change only provider/model/harness; env vars are kept from the desktop", async () => {
  const raw = [];
  const d = cmdDeps();
  d.invokeRaw = async (cmd, args) => {
    raw.push([cmd, args]);
    if (cmd === "get_global_agent_config") return { env_vars: { A: "real-secret" }, provider: "old", model: null, preferred_runtime: null };
    if (cmd === "discover_acp_providers") return [{ id: "goose", command: "goose-acp", availability: "available", source: "builtin" }];
    return { config: { env_vars: { A: "real-secret" } }, restarted: 0 };
  };
  const r = await handleMgmt("cmd", { name: "set_global_agent_config", args: { config: { env_vars: { EVIL: "1" }, provider: "anthropic", model: "m", preferred_runtime: "goose" } } }, d);
  assert.deepEqual(raw.find((c) => c[0] === "set_global_agent_config")[1].config, { env_vars: { A: "real-secret" }, provider: "anthropic", model: "m", preferred_runtime: "goose" });
  assert.ok(!JSON.stringify(r).includes("real-secret"));
  assert.equal(await code(handleMgmt("cmd", { name: "set_global_agent_config", args: { config: { preferred_runtime: "nope" } } }, d)), "runtime_unavailable");
});
