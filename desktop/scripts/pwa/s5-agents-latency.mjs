// Agents-list latency (s5) — measures signer round trips and time to the agent list; originally from s2: Slice A (relay bridge) e2e against the LOCAL relay (:3100 only) with a mock NIP-46 signer (2s latency).
// The desktop side is the REAL BridgePublisher (src/features/bridge/publisher.ts) acting as "the desktop"; the page is the real web build.
// Run: node --import ./test-loader.mjs --experimental-strip-types scripts/pwa/s2-bridge-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { finalizeEvent, getPublicKey, nip44 } from "nostr-tools";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import {
  BRIDGE_KIND,
  D_AGENTS,
  D_HEARTBEAT,
  D_KEY,
  txChunkD,
  bridgeTags,
  buildEnvelope,
  encryptEnvelope,
  generateSessionKey,
  keyToHex,
  nowS,
} from "../../src/features/bridge/protocol.ts";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const RELAY_HTTP = "http://localhost:3100";
const SK_HEX =
  "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const sk = new Uint8Array(Buffer.from(SK_HEX, "hex"));
const myPk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, myPk);
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok });
  console.log(
    `${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`,
  );
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function postEvent(ev) {
  const url = `${RELAY_HTTP}/events`;
  const bytes = Buffer.from(JSON.stringify(ev));
  const auth = finalizeEvent(
    {
      kind: 27235,
      content: "",
      created_at: nowS(),
      tags: [
        ["u", url],
        ["method", "POST"],
        ["payload", createHash("sha256").update(bytes).digest("hex")],
        ["nonce", randomUUID()],
      ],
    },
    sk,
  );
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization:
        "Nostr " + Buffer.from(JSON.stringify(auth)).toString("base64"),
    },
    body: bytes,
  });
  if (!res.ok) throw new Error(`publish ${res.status}: ${await res.text()}`);
}
const signNode = (e) =>
  finalizeEvent(
    { kind: e.kind, content: e.content, created_at: e.createdAt, tags: e.tags },
    sk,
  );
const agent = (o) => ({
  pubkey: "ab".repeat(32),
  name: "Opus",
  status: "running",
  activeTurns: [],
  ...o,
});
// Signer decrypts attributable to the bridge (the app issues many unrelated ones: reminders etc.), counted in-page.
const bridgeDecrypts = () =>
  page.evaluate(() => window.__buzzBridgeStats?.signerDecrypts ?? 0);
const localDecrypts = () =>
  page.evaluate(() => window.__buzzBridgeStats?.localDecrypts ?? 0);


const LAT = Number(process.env.LAT_MS ?? 2000);
const CHUNKS = Number(process.env.CHUNKS ?? 170);
const now0 = nowS();
const k1 = generateSessionKey();
const put = async (d, type, body, createdAt = nowS(), seq = 1) => {
  const env = buildEnvelope({ type, kid: k1.kid, dev: "s5-dev", seq, ttlS: 86400, body, now: nowS() });
  await postEvent(signNode({ kind: BRIDGE_KIND, content: encryptEnvelope(env, k1.key), createdAt, tags: bridgeTags(d, k1.kid, env.exp) }));
};
await postEvent(signNode({ kind: BRIDGE_KIND, createdAt: nowS() - 1, tags: bridgeTags(D_KEY, null, now0 + 86400),
  content: nip44.v2.encrypt(JSON.stringify({ v: 1, kid: k1.kid, key: keyToHex(k1.key), createdAt: now0, expiresAt: now0 + 86400 }), selfKey) }));
await put(D_AGENTS, "agents.snapshot", { agents: [agent({ name: "LatBot", status: "running" })], generatedAt: now0 }, nowS() - 1, 5000000 + now0 % 100000);
await put(D_HEARTBEAT, "presence.heartbeat", { bridge: "v1", agentsRunning: 1, uptimeSec: 5, caps: [] }, nowS() - 1, 5000000 + now0 % 100000);
await sleep(1100); // everything below is NEWER than the snapshot/heartbeat
const agents3 = ["ab", "cd", "ef"].map((x) => x.repeat(32));
for (let i = 0; i < CHUNKS; i++) {
  const a = agents3[i % 3];
  await put(txChunkD(a, now0 * 1000 + i), "transcript.chunk", { agentPubkey: a, entries: Array.from({ length: 6 }, (_, j) => ({ id: `e${i}-${j}`, t: Date.now() + i * 10 + j, role: "assistant", text: "x".repeat(1500) })) }, nowS(), i + 1);
}
console.log(`published key+snapshot+heartbeat then ${CHUNKS} transcript chunks`);

const browser = await chromium.launch();
const signer = await startMockSigner({ userSkHex: SK_HEX, latencyMs: LAT, jitterMs: 300 });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk: signer.userPk });
const page = await ctx.newPage();
const counts = () => page.evaluate(() => { const c = window.__buzzSignerCalls?.counts?.() ?? {}; return c; });
const rowVisible = (timeout) => page.waitForFunction(() => /LatBot/.test(document.querySelector('[data-testid="my-agents"]')?.textContent || ""), null, { timeout }).then(() => true, () => false);
const sum = (c) => Object.values(c).reduce((a, b) => a + b, 0);
const fmt = (c) => Object.entries(c).map(([k, v]) => `${k}=${v}`).join(" ");

await page.goto(APP + "#/agents");
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', { timeout: 20000 });
await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
await page.click('[data-testid="web-unlock-bunker-go"]');
const tSignedIn = Date.now();
const ok1 = await rowVisible(120000);
const dt1 = Date.now() - tSignedIn;
const c1 = await counts();
const bs1 = await page.evaluate(() => ({ ...window.__buzzBridgeStats, log: undefined }));
console.log(`FIRST LOAD (after sign-in): ${ok1 ? "list shown" : "NOT SHOWN"} in ${(dt1 / 1000).toFixed(1)}s; signer calls ${sum(c1)} :: ${fmt(c1)}; bridge ${JSON.stringify(bs1)}`);
check("first load: agent list within 15s", ok1 && dt1 <= 15000, `${(dt1 / 1000).toFixed(1)}s`);

await page.waitForTimeout(3000);
const tR = Date.now();
await page.reload();
const okCache = await page.waitForFunction(() => /LatBot/.test(document.querySelector('[data-testid="my-agents"]')?.textContent || ""), null, { timeout: 120000 }).then(() => true, () => false);
const dt2 = Date.now() - tR;
const c2 = await counts();
console.log(`RELOAD: list shown in ${(dt2 / 1000).toFixed(1)}s; signer calls ${sum(c2)} :: ${fmt(c2)}`);
check("reload: agent list within 5s (cache)", okCache && dt2 <= 5000, `${(dt2 / 1000).toFixed(1)}s`);
const decRel = await page.evaluate(() => window.__buzzBridgeStats?.signerDecrypts ?? 0);
check("reload: no signer decrypt for the key wrap", decRel === 0, `${decRel}`);
await browser.close();
await signer.close();
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
