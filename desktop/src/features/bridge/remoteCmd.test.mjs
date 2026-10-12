import assert from "node:assert/strict";
import test from "node:test";
import { installProxyHandlers, NeedsDirectError, proxyCmd, setProxyDirect } from "./remoteCmd.ts";

function fakeDirect(live, answer = (n) => [n]) {
  const calls = [];
  let active = 0;
  let maxActive = 0;
  return { calls, maxActive: () => maxActive, live, call: async (op, p) => { active++; maxActive = Math.max(maxActive, active); calls.push([op, p]); await new Promise((r) => setTimeout(r, 5)); active--; return answer(p.name); } };
}

test("without a live direct connection reads return the empty default and writes say 'needs Direct'", async () => {
  setProxyDirect(null);
  const h = {};
  installProxyHandlers(h);
  assert.deepEqual(await h.list_managed_agents(), []);
  assert.equal(await h.agent_access_owner_only(), false);
  assert.deepEqual((await h.get_global_agent_config()).env_vars, {});
  await assert.rejects(h.create_managed_agent({ input: { name: "x" } }), NeedsDirectError);
  await assert.rejects(proxyCmd("update_managed_agent", {}), /Direct connection over your tailnet/);
  setProxyDirect(fakeDirect(false));
  await assert.rejects(h.stop_managed_agent({ pubkey: "a" }), NeedsDirectError);
});

test("with a live connection every proxied command goes to the desktop's cmd op, at most 3 at a time", async () => {
  const d = fakeDirect(true);
  setProxyDirect(d);
  const h = {};
  installProxyHandlers(h);
  const r = await Promise.all(["list_managed_agents", "list_personas", "discover_acp_providers", "list_managed_agent_runtimes", "get_global_agent_config", "agent_access_owner_only"].map((n) => h[n]({})));
  assert.deepEqual(r[0], ["list_managed_agents"]);
  assert.ok(d.calls.every(([op]) => op === "cmd"));
  assert.ok(d.maxActive() <= 3);
  setProxyDirect(null);
});

test("a desktop refusal becomes a readable error", async () => {
  const { RemoteError } = await import("./remoteClient.ts");
  setProxyDirect({ live: true, call: async () => { throw new RemoteError("agents_disabled"); } });
  await assert.rejects(proxyCmd("list_managed_agents"), /Allow remote agent management/);
  setProxyDirect(null);
});

test("one shared direct client stays bound until the last holder (nav, Settings) releases it", async () => {
  const { acquireProxyDirect, getProxyDirect } = await import("./remoteCmd.ts");
  setProxyDirect(null);
  const d = fakeDirect(true);
  const releaseNav = acquireProxyDirect(d);
  const releaseSettings = acquireProxyDirect(d);
  assert.equal(getProxyDirect(), d);
  releaseNav();
  assert.equal(getProxyDirect(), d, "Settings still holds it after the nav goes away");
  releaseSettings();
  releaseSettings();
  assert.equal(getProxyDirect(), null);
  acquireProxyDirect(null)();
});

test("presence prefers the desktop's answer over Direct and falls back to the web's own", async () => {
  const PK = "a".repeat(64);
  const h = { get_presence: async () => ({ [PK]: "offline" }) };
  installProxyHandlers(h);
  setProxyDirect(null);
  assert.deepEqual(await h.get_presence({ pubkeys: [PK] }), { [PK]: "offline" });
  const d = fakeDirect(true, () => ({ [PK]: "online" }));
  setProxyDirect(d);
  assert.deepEqual(await h.get_presence({ pubkeys: [PK] }), { [PK]: "online" });
  assert.deepEqual(d.calls[0], ["cmd", { name: "get_presence", args: { pubkeys: [PK] } }]);
  setProxyDirect({ live: true, call: async () => { throw new Error("down"); } });
  assert.deepEqual(await h.get_presence({ pubkeys: [PK] }), { [PK]: "offline" });
  setProxyDirect(null);
});
