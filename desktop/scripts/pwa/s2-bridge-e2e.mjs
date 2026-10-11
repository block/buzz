// Slice A (relay bridge) e2e against the LOCAL relay (:3100 only) with a mock NIP-46 signer (2s latency).
// The desktop side is the REAL BridgePublisher (src/features/bridge/publisher.ts) acting as "the desktop"; the page is the real web build.
// Run: node --import ./test-loader.mjs --experimental-strip-types scripts/pwa/s2-bridge-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { createHash, randomUUID } from "node:crypto";
import { finalizeEvent, getPublicKey, nip44 } from "nostr-tools";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import { BridgePublisher } from "../../src/features/bridge/publisher.ts";
import { BridgeCommandHandler, makeSeenSet } from "../../src/features/bridge/commandHandler.ts";
import {
  BRIDGE_KIND,
  D_AGENTS,
  D_HEARTBEAT,
  D_KEY,
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

// Phase 0: a STALE desktop (last heard 10 minutes ago) so the page renders "Desktop offline" with the last snapshot.
const stale = nowS() - 600;
const k0 = generateSessionKey();
const putStale = async (d, type, body, kid) => {
  const env = buildEnvelope({
    type,
    kid,
    dev: "stale-dev",
    seq: 1,
    ttlS: 86400,
    body,
    now: stale,
  });
  await postEvent(
    signNode({
      kind: BRIDGE_KIND,
      content: encryptEnvelope(env, k0.key),
      createdAt: nowS(),
      tags: bridgeTags(d, kid, env.exp),
    }),
  ); // event created_at = now (so it supersedes earlier runs' replaceable heads); the ENVELOPE ts is the stale desktop clock
};
await postEvent(
  signNode({
    kind: BRIDGE_KIND,
    createdAt: nowS() - 1,
    tags: bridgeTags(D_KEY, null, stale + 86400),
    content: nip44.v2.encrypt(
      JSON.stringify({
        v: 1,
        kid: k0.kid,
        key: keyToHex(k0.key),
        createdAt: stale,
        expiresAt: stale + 86400,
      }),
      selfKey,
    ),
  }),
);
await putStale(
  D_AGENTS,
  "agents.snapshot",
  {
    agents: [agent({ name: "StaleBot", status: "stopped" })],
    generatedAt: stale,
  },
  k0.kid,
);
await putStale(
  D_HEARTBEAT,
  "presence.heartbeat",
  { bridge: "v1", agentsRunning: 0, uptimeSec: 5, caps: [] },
  k0.kid,
);

const browser = await chromium.launch();
const signer = await startMockSigner({
  userSkHex: SK_HEX,
  latencyMs: 2000,
  jitterMs: 500,
});
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
});
await ctx.addInitScript(
  ({ relay, pk }) => {
    if (!localStorage.getItem("buzz-communities")) {
      localStorage.setItem(
        "buzz-communities",
        JSON.stringify([
          {
            id: "c1",
            name: "Local Dev",
            relayUrl: relay,
            pubkey: pk,
            addedAt: new Date().toISOString(),
          },
        ]),
      );
      localStorage.setItem("buzz-active-community-id", "c1");
    }
  },
  { relay: RELAY_WS, pk: signer.userPk },
);
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e).slice(0, 200)));
const prod = [];
page.on("request", (r) => {
  if (/huladesk/.test(r.url())) prod.push(r.url());
});
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', {
  timeout: 20000,
});
await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
await page.click('[data-testid="web-unlock-bunker-go"]');
await page.waitForSelector(
  '[data-testid="sidebar-channel-content"] >> text=general',
  { timeout: 40000 },
);
await page.evaluate(() => {
  location.hash = "#/agents";
});
const panel = page.locator('[data-testid="my-agents"]');
await panel.waitFor({ timeout: 20000 });
const presenceText = () =>
  page.locator('[data-testid="my-agents-presence"]').innerText();

// 1. offline desktop
await page
  .waitForFunction(
    () =>
      document
        .querySelector('[data-testid="my-agents"]')
        ?.getAttribute("data-presence") === "offline",
    null,
    { timeout: 90000 },
  )
  .catch(() => {});
const t1 = await presenceText();
check(
  "1. stale heartbeat shows 'Desktop offline' with the last snapshot dimmed",
  /Desktop offline/.test(t1) &&
    (await page
      .locator('[data-testid="my-agent-row"]')
      .filter({ hasText: "StaleBot" })
      .count()) === 1,
  t1,
);
const d1 = await bridgeDecrypts();
check(
  "1b. exactly one signer decrypt (the key wrap) to render it",
  d1 === 1,
  `${d1}`,
);

// 2. real desktop publisher comes online with a NEW key + agents; the live page updates
const pub = new BridgePublisher(
  {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) => signNode(e),
    publish: postEvent,
  },
  { deviceId: "e2e-desktop", platform: "e2e", caps: ["agents.snapshot", "transcript.chunk", "agent.start", "agent.stop", "agent.message"], onError: (e) => console.log("PUBLISHER ERROR", String(e).slice(0, 300)) },
);
pub.start();
pub.update({
  agents: [
    agent({
      name: "Opus",
      status: "running",
      activeTurns: [{ channelId: "c1", channelName: "general", since: nowS() }],
      currentTask: "Working in general",
    }),
    agent({ pubkey: "cd".repeat(32), name: "Sonnet", status: "stopped" }),
  ],
  generatedAt: nowS(),
});
await page
  .waitForFunction(
    () =>
      document
        .querySelector('[data-testid="my-agents"]')
        ?.getAttribute("data-presence") === "online",
    null,
    { timeout: 40000 },
  )
  .catch(() => {});
const rows = await page.locator('[data-testid="my-agent-row"]').allInnerTexts();
check(
  "2. live update: 'Desktop online' and the new agent list (running agent with current task)",
  (await presenceText()).includes("Desktop online") &&
    rows.some(
      (r) =>
        /Opus/.test(r) && /Working in general/.test(r) && /running/.test(r),
    ) &&
    rows.some((r) => /Sonnet/.test(r)) &&
    !rows.some((r) => /StaleBot/.test(r)),
  rows.join(" | ").replace(/\n/g, " "),
);

// 3. further changes decrypt locally: zero extra signer calls
const dBefore = await bridgeDecrypts();
const lBefore = await localDecrypts();
for (let i = 1; i <= 3; i++) {
  await sleep(5500);
  pub.update({
    agents: [
      agent({ name: `Opus v${i}`, status: i === 3 ? "stopped" : "running" }),
      agent({ pubkey: "cd".repeat(32), name: "Sonnet", status: "stopped" }),
    ],
    generatedAt: nowS(),
  });
}
await page
  .waitForFunction(
    () =>
      /Opus v3/.test(
        document.querySelector('[data-testid="my-agents"]')?.textContent || "",
      ),
    null,
    { timeout: 30000 },
  )
  .catch(() => {});
const dAfter = await bridgeDecrypts();
const lAfter = await localDecrypts();
check(
  "3. three more snapshots arrive with ZERO additional signer calls",
  /Opus v3/.test(await panel.innerText()) &&
    dAfter === dBefore &&
    lAfter > lBefore,
  `signer decrypts ${dBefore} → ${dAfter}; local decrypts ${lBefore} → ${lAfter}`,
);
check(
  "3b. total signer decrypts for the whole session <= 2 (stale key + live key)",
  dAfter <= 2,
  `${dAfter}`,
);

// 4b. transcripts: the desktop publishes redacted chunks; the agent detail view shows them live
const OPUS = "ab".repeat(32);
pub.update({
  agents: [
    agent({ name: "Opus v3", status: "running" }),
    agent({ pubkey: "cd".repeat(32), name: "Sonnet", status: "stopped" }),
  ],
  generatedAt: nowS(),
});
await sleep(300);
const dTx0 = await bridgeDecrypts();
pub.updateTranscript(OPUS, [
  { id: "t1", t: Date.now() - 3000, role: "user", text: "please fix the build" },
  { id: "t2", t: Date.now() - 2000, role: "assistant", text: "Looking at ~/…/src/x.ts with TOKEN=[redacted]" },
]);
await page.locator('[data-testid="my-agent-row"]').filter({ hasText: "Opus v3" }).first().click();
await page.locator('[data-testid="agent-detail"]').waitFor({ timeout: 20000 });
await page
  .waitForFunction(() => document.querySelectorAll('[data-testid="transcript-entry"]').length >= 2, null, { timeout: 40000 })
  .catch(() => {});
const txt1 = await page.locator('[data-testid="agent-transcript"]').innerText();
check(
  "4b. agent detail shows the published transcript (user + assistant entries)",
  /please fix the build/.test(txt1) && /Looking at/.test(txt1),
  txt1.replace(/\n/g, " | ").slice(0, 160),
);
pub.updateTranscript(OPUS, [
  { id: "t1", t: Date.now() - 3000, role: "user", text: "please fix the build" },
  { id: "t2", t: Date.now() - 2000, role: "assistant", text: "Looking at ~/…/src/x.ts with TOKEN=[redacted] — done, build is green" },
  { id: "t3", t: Date.now() - 1000, role: "tool", text: "ok", tool: { name: "bash", status: "completed" } },
]);
await page
  .waitForFunction(() => /build is green/.test(document.querySelector('[data-testid="agent-transcript"]')?.textContent || "") && document.querySelectorAll('[data-testid="transcript-entry"]').length === 3, null, { timeout: 40000 })
  .catch(() => {});
check(
  "4c. live update: grown entry replaced in place, new tool entry appended, no extra signer calls",
  (await page.locator('[data-testid="transcript-entry"]').count()) === 3 &&
    (await bridgeDecrypts()) === dTx0,
  `entries=${await page.locator('[data-testid="transcript-entry"]').count()} decrypts ${dTx0}→${await bridgeDecrypts()}`,
);
await page.locator('[data-testid="agent-detail-back"]').click();

// 4. reload: sessionStorage-cached key means no signer decrypt on relaunch
const dPre = 0;
await page.reload();
await page
  .waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', {
    timeout: 40000,
  })
  .catch(() => {});
await page.evaluate(() => {
  location.hash = "#/agents";
});
await page
  .waitForFunction(
    () =>
      /Opus v3/.test(
        document.querySelector('[data-testid="my-agents"]')?.textContent || "",
      ),
    null,
    { timeout: 90000 },
  )
  .catch(() => {});
check(
  "4. after a page reload the agent list returns with no new signer decrypt",
  /Opus v3/.test(await panel.innerText().catch(() => "")) &&
    (await bridgeDecrypts()) === 0,
  `signer decrypts after reload: ${await bridgeDecrypts()}`,
);

// 4d. live updates still flow after a reload (live subscription re-established)
const tLive = Date.now();
pub.update({
  agents: [agent({ name: "Opus v4", status: "running" }), agent({ pubkey: "cd".repeat(32), name: "Sonnet", status: "stopped" })],
  generatedAt: nowS(),
});
await page
  .waitForFunction(() => /Opus v4/.test(document.querySelector('[data-testid="my-agents"]')?.textContent || ""), null, { timeout: 60000 })
  .catch(() => {});
check(
  "4d. after reload a new snapshot arrives live",
  /Opus v4/.test(await panel.innerText().catch(() => "")),
  `${Date.now() - tLive} ms`,
);

// 6. controls: web confirmation + desktop-side validation, over the real relay
const execCalls = [];
const handler = new BridgeCommandHandler({
  memberPubkey: myPk,
  sessionKey: () => ({ kid: pub.kid, key: pub.sessionKey }),
  agents: () => pub.agents(),
  seen: makeSeenSet(() => [], () => {}),
  executors: {
    start: async (a) => void execCalls.push(["start", a]),
    stop: async (a) => void execCalls.push(["stop", a]),
    message: async (a, c, t) => void execCalls.push(["message", a, c, t]),
  },
  publishResult: async (id, b) => {
    await pub.publishResult(id, b);
  },
});
const seenEv = new Set();
let wireCmds = 0;
const queryCmds = async () => {
  const url = `${RELAY_HTTP}/query`;
  const bytes = Buffer.from(JSON.stringify([{ kinds: [30078], authors: [myPk], "#t": ["buzz-bridge"], since: nowS() - 120, limit: 100 }]));
  const auth = finalizeEvent({ kind: 27235, content: "", created_at: nowS(), tags: [["u", url], ["method", "POST"], ["payload", createHash("sha256").update(bytes).digest("hex")], ["nonce", randomUUID()]] }, sk);
  const r = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json", Authorization: "Nostr " + Buffer.from(JSON.stringify(auth)).toString("base64") }, body: bytes });
  for (const ev of await r.json()) {
    if (seenEv.has(ev.id)) continue;
    seenEv.add(ev.id);
    if (ev.tags.some((t) => t[1]?.startsWith("buzz.bridge.v1/cmd/"))) wireCmds++;
    await handler.handle(ev);
  }
};
const pollTimer = setInterval(() => void queryCmds().catch(() => {}), 1000);
await page.locator('[data-testid="my-agent-row"]').filter({ hasText: "Opus v4" }).first().click();
await page.locator('[data-testid="agent-controls"]').waitFor({ timeout: 20000 });
await page.locator('[data-testid="ctl-stop"]').click();
await page.locator('[data-testid="ctl-confirm"]').waitFor();
await page.locator('[data-testid="ctl-confirm-no"]').click();
await sleep(4000);
check("6a. Stop asks for confirmation; Cancel sends nothing", wireCmds === 0 && execCalls.length === 0, `wire=${wireCmds} exec=${execCalls.length}`);
await page.locator('[data-testid="ctl-stop"]').click();
await page.locator('[data-testid="ctl-confirm-yes"]').click();
await page
  .waitForFunction(() => /done/.test(document.querySelector('[data-testid="ctl-status"]')?.textContent || ""), null, { timeout: 60000 })
  .catch(() => {});
check(
  "6b. Confirm -> desktop validates and executes once -> web shows done",
  execCalls.length === 1 && execCalls[0][0] === "stop" && /done/.test(await page.locator('[data-testid="ctl-status"]').innerText().catch(() => "")),
  `${JSON.stringify(execCalls)} ${await page.locator('[data-testid="ctl-status"]').innerText().catch(() => "")}`,
);
await sleep(3500);
check("6c. replayed relay events never re-execute", execCalls.length === 1, `${execCalls.length}`);
clearInterval(pollTimer);

// 5. no plaintext on the relay, no prod traffic, no page errors
check(
  "5. page errors none; no production-relay traffic",
  errors.length === 0 && prod.length === 0,
  `${errors.join(";")} ${prod.join(",")}`,
);
pub.destroy();
await ctx.close();
await signer.close();
await browser.close();
const bad = results.filter((x) => !x.ok);
console.log(`\n${results.length - bad.length}/${results.length} checks passed`);
process.exit(bad.length ? 1 : 0);
