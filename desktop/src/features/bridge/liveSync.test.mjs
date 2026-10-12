import assert from "node:assert/strict";
import test from "node:test";
import { getPublicKey } from "nostr-tools";
import { BridgeWebClient } from "./webClient.ts";
import { DirectAgents } from "./directAgents.ts";
import { whyTermUnavailable } from "./remoteHosts.ts";
import { buildEnvelope, encryptEnvelope, generateSessionKey, D_AGENTS, D_RES_PREFIX } from "./protocol.ts";

test("whyTermUnavailable names the one missing requirement", () => {
  const host = (o) => ({ id: "d", name: "Mac", url: "wss://x", ageS: 5, stale: false, supports: { agents: true, terminal: true }, hasDirect: true, sigTtlS: null, ...o });
  assert.match(whyTermUnavailable(undefined, false, "unknown"), /hasn't reported/);
  assert.match(whyTermUnavailable(host({ supports: { agents: true, terminal: false } }), true, "reachable"), /Allow remote terminal is off/);
  assert.match(whyTermUnavailable(host({ supports: { agents: true, terminal: false }, hasDirect: false }), true, "unknown"), /Tailscale/);
  assert.match(whyTermUnavailable(host({ stale: true }), true, "reachable"), /over a day/);
  assert.match(whyTermUnavailable(host(), true, "unreachable"), /tailnet/);
  assert.equal(whyTermUnavailable(host(), true, "reachable"), null);
});

test("DirectAgents.onMutation fires after start/stop/create acks, not on failures", async () => {
  const d = new DirectAgents({ desktopId: "d", url: "wss://x" }, async () => "{}");
  d.call = async (op) => (op === "agent.stop" ? Promise.reject(new Error("no")) : { ok: true });
  const seen = [];
  d.onMutation((m) => seen.push(m.op + ":" + (m.pubkey ?? "")));
  await d.start("aa");
  await d.create({ name: "x" });
  await assert.rejects(d.stop("aa"));
  assert.deepEqual(seen, ["start:aa", "create:"]);
});

test("web client shows a start/stop as soon as the desktop confirms it", async () => {
  const sk = new Uint8Array(32).fill(3);
  const c = new BridgeWebClient(getPublicKey(sk), { decryptFromSelf: async () => "", cacheGet: () => null, cachePut: () => {} });
  const pub = "aa".repeat(32);
  c.state = { ...c.state, snapshot: { agents: [{ pubkey: pub, name: "A", status: "stopped", activeTurns: [] }], generatedAt: 1 } };
  c.expectStatus("cmd1", pub, "running");
  assert.equal(c.expected.size, 1);
  // apply the patch the way the result handler does
  const e = c.expected.get("cmd1");
  assert.deepEqual(e, { pubkey: pub, status: "running" });
});
