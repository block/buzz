// Slice A (relay bridge) e2e against the LOCAL relay (:3100 only) with a mock NIP-46 signer (2s latency).
// The desktop side is the REAL BridgePublisher (src/features/bridge/publisher.ts) acting as "the desktop"; the page is the real web build.
// Run: node --import ./test-loader.mjs --experimental-strip-types scripts/pwa/s2-bridge-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { finalizeEvent, getPublicKey, nip44 } from "nostr-tools";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import { BridgePublisher } from "../../src/features/bridge/publisher.ts";
import { BrowserSessionManager } from "../../src/features/bridge/browserSessions.ts";
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
    agents: [agent({ name: "StaleBot", status: "running" })],
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

// Slice B harness: the real buzz-remote-access listener (examples/serve.rs), loopback only, standing in for the tailnet.
const RA_BIN = process.env.RA_SERVE_BIN || "/tmp/ra-target/debug/examples/serve";
const raProc = spawn(RA_BIN, [myPk, new URL(APP).origin], { stdio: ["pipe", "pipe", "inherit"], env: { ...process.env, RA_MGMT: "fake" } }); // fake = in-memory agent store behind the real listener
const raAddr = await new Promise((resolve, reject) => {
  const t = setTimeout(() => reject(new Error("serve harness did not start")), 20000);
  raProc.stdout.on("data", (d) => {
    const m = /listening (\S+)/.exec(String(d));
    if (m) { clearTimeout(t); resolve(m[1]); }
  });
});
process.on("exit", () => raProc.kill());

let raAgentsOn = true;
let raTermHost = "shell"; // what the desktop advertises from Settings → Buzz Term → Session host
let raTermSession = "work";
const pub = new BridgePublisher(
  {
    encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey),
    signEvent: async (e) => signNode(e),
    publish: postEvent,
  },
  { deviceId: "e2e-desktop", platform: "e2e", caps: ["agents.snapshot", "transcript.chunk", "agent.start", "agent.stop", "agent.message", "browser.open", "browser.close", "browser.screenshot", "browser.agent"], remote: () => ({ desktopId: "dev-desktop", name: "E2E Mac", url: `ws://${raAddr}/v1/ws`, status: "on", terminals: true, termHost: raTermHost, ...(raTermHost === "herdr" ? { termSession: raTermSession } : {}), agents: raAgentsOn, sigTtlS: 3600 }),
    onError: (e) => console.log("PUBLISHER ERROR", String(e).slice(0, 300)) },
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
    !rows.some((r) => /Sonnet/.test(r)) && // relay bridge lists ONLY running agents: the stopped one is hidden
    (await page.locator('[data-testid="my-agents-hidden-note"]').innerText().catch(() => "")).includes("1 stopped agent is hidden") &&
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
      agent({ name: `Opus v${i}`, status: "running" }), // (the relay bridge lists running agents only)
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
const brManager = new BrowserSessionManager(2);
const brCalls = [];
// The desktop's Browsers page (sync): one browser that was added ON the desktop, plus whatever the web opens.
const deskBrowsers = new Map([["native-1", { sessionId: "native-1", title: "Desktop news", host: "news.example.org", url: "https://news.example.org/", status: "cold", origin: "desktop", openedAt: nowS() }]]);
const brAgentCalls = [];
const publishDesk = () => pub.publishBrowserSessions([...deskBrowsers.values()]);
const JPEG1 = Buffer.from("/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA=", "base64");
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
  browser: {
    manager: brManager,
    open: async (url, host) => (brCalls.push(["open", url]), deskBrowsers.set("pin-e2e", { sessionId: "pin-e2e", title: host, host, url, status: "live", origin: "web", openedAt: nowS() }), { sessionId: "pin-e2e", title: host }),
    close: async (id) => void (brCalls.push(["close", id]), deskBrowsers.delete(id)),
    exists: (id) => deskBrowsers.has(id),
    setAgent: async (i) => void brAgentCalls.push(i),
    capture: async () => ({ mime: "image/jpeg", w: 1, h: 1, bytes: new Uint8Array(JPEG1) }),
    publishShot: (id, s) => pub.publishShot(id, s),
    publishSessions: publishDesk,
  },
  agentManagementAllowed: () => raAgentsOn,
});
const seenEv = new Set();
let wireCmds = 0;
let handlerPaused = false;
const queryCmds = async () => {
  const url = `${RELAY_HTTP}/query`;
  const bytes = Buffer.from(JSON.stringify([{ kinds: [30078], authors: [myPk], "#t": ["buzz-bridge"], since: nowS() - 120, limit: 100 }]));
  const auth = finalizeEvent({ kind: 27235, content: "", created_at: nowS(), tags: [["u", url], ["method", "POST"], ["payload", createHash("sha256").update(bytes).digest("hex")], ["nonce", randomUUID()]] }, sk);
  const r = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json", Authorization: "Nostr " + Buffer.from(JSON.stringify(auth)).toString("base64") }, body: bytes });
  for (const ev of await r.json()) {
    if (seenEv.has(ev.id)) continue;
    seenEv.add(ev.id);
    if (handlerPaused) continue; // simulates a desktop that never receives the command
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

// 7. slice R: remote browser (view-only), over the real relay
await page.locator('[data-testid="agent-detail-back"]').click();
check("7-0. My agents points to the Browsers page instead of hosting its own browser panel", (await page.locator('[data-testid="browsers-pointer"]').count()) === 1 && (await page.locator('[data-testid="browser-panel"]').count()) === 0);
await publishDesk();
await page.evaluate(() => { location.hash = "#/browsers"; });
await page.locator('[data-testid="browser-panel"]').waitFor({ timeout: 40000 });
check("7-1. Browsers page lists a browser that exists only on the desktop (synced), with Cold and its address", await page.waitForFunction(() => { const t = document.querySelector('[data-testid="browser-panel"]')?.textContent || ""; return /Desktop news/.test(t) && /Cold/.test(t) && /news\.example\.org/.test(t) && /On desktop/.test(t); }, null, { timeout: 60000 }).then(() => true, () => false));
const addViaDialog = async (u) => {
  await page.locator('[data-testid="browsers-add"]').click();
  await page.locator('[data-testid="browsers-add-url"]').fill(u);
};
await addViaDialog("http://localhost:9999/admin");
await page.getByRole("button", { name: "Cancel" }).first().click();
await sleep(3000);
check("7a. Add browser dialog: Cancel sends nothing", brCalls.length === 0);
await addViaDialog("http://localhost:9999/admin");
await page.locator('[data-testid="browsers-add-submit"]').click();
await page.waitForFunction(() => /failed/.test(document.querySelector('[data-testid="browser-job-text"]')?.textContent || ""), null, { timeout: 60000 }).catch(() => {});
check("7b. desktop refuses a private/local address (no session opened); the failure reason is shown", brCalls.length === 0 && brManager.list().length === 0 && /Private and local/.test(await page.locator('[data-testid="browser-job-text"]').innerText()));
await page.locator('[data-testid="browser-job-cancel"]').click();
await addViaDialog("https://example.com/page");
await page.locator('[data-testid="browsers-add-submit"]').click();
await page.waitForFunction(() => /sending|waiting|accepted|done/.test(document.querySelector('[data-testid="browser-job"]')?.getAttribute("data-phase") || ""), null, { timeout: 30000 }).catch(() => {});
check("7b2. a progress line is shown while the command runs", (await page.locator('[data-testid="browser-job"]').count()) === 1);
await page.locator('[data-testid="br-session"]').nth(1).waitFor({ timeout: 60000 }).catch(() => {});
check("7c. confirmed open -> session listed on the web next to the desktop one", (await page.waitForFunction(() => document.querySelectorAll('[data-testid="br-session"]').length === 2, null, { timeout: 30000 }).then(() => true, () => false)) && brCalls.length === 1, JSON.stringify(brCalls));
check("7c2. job finishes as done", await page.waitForFunction(() => /done/.test(document.querySelector('[data-testid="browser-job-text"]')?.textContent || ""), null, { timeout: 30000 }).then(() => true, () => false));
await page.locator('[data-testid="br-session"]').filter({ hasText: "example.com" }).locator('[data-testid="br-shot"]').click();
await page.locator('[data-testid="browser-row-preview-img-pin-e2e"]').waitFor({ timeout: 60000 }).catch(() => {});
const shotSrc = await page.locator('[data-testid="browser-row-preview-img-pin-e2e"]').getAttribute("src").catch(() => "");
check("7d. See screenshot shows one encrypted-chunk image", /^data:image\/jpeg;base64,/.test(shotSrc || ""), (shotSrc || "").slice(0, 40));
await sleep(1500);
check("7e. no screenshot is taken without a press (one capture only)", brManager.beginShot("pin-e2e", Date.now()) === "rate_limited");

// 7f. robot (Observe/Drive) is gated by 'Allow remote agent management'
check("7f. robot button is shown while remote agent management is on", (await page.locator('[data-testid="br-robot"]').count()) >= 1);
await page.locator('[data-testid="br-session"]').filter({ hasText: "Desktop news" }).locator('[data-testid="br-robot"]').click();
await page.locator('[data-testid="br-robot-agent"]').selectOption({ index: 1 });
await page.locator('[data-testid="br-robot-mode-drive"]').click();
await page.locator('[data-testid="br-robot-apply"]').click();
await sleep(6000);
check("7g. Drive command reaches the desktop (agent + mode) while the switch is on", brAgentCalls.length === 1 && brAgentCalls[0].mode === "drive" && brAgentCalls[0].sessionId === "native-1", JSON.stringify(brAgentCalls));
raAgentsOn = false;
await pub.sendHeartbeat();
check("7h. robot button disappears when remote agent management is turned off", await page.waitForFunction(() => document.querySelectorAll('[data-testid="br-robot"]').length === 0, null, { timeout: 60000 }).then(() => true, () => false));
raAgentsOn = true;
await pub.sendHeartbeat();
await page.waitForFunction(() => document.querySelectorAll('[data-testid="br-robot"]').length >= 1, null, { timeout: 60000 }).catch(() => {});
await page.locator('[data-testid="br-session"]').filter({ hasText: "Desktop news" }).locator('[data-testid="br-remove"]').click();
await page.locator('[data-testid="br-confirm-yes"]').click();
check("7i. Remove on web removes a desktop-added browser on the desktop and the list syncs", await page.waitForFunction(() => document.querySelectorAll('[data-testid="br-session"]').length === 1, null, { timeout: 60000 }).then(() => true, () => false) && !deskBrowsers.has("native-1"));

// 7j. desktop never answers -> hard 30 s timeout with a clear message, Retry works
handlerPaused = true;
await addViaDialog("https://timeout.example.com/");
await page.locator('[data-testid="browsers-add-submit"]').click();
const tTimeout = Date.now();
check("7j. no answer from the desktop -> 'didn't respond' error with Retry/Cancel within ~35 s", await page.waitForFunction(() => document.querySelector('[data-testid="browser-job"]')?.getAttribute("data-phase") === "timeout", null, { timeout: 50000 }).then(() => true, () => false) && Date.now() - tTimeout < 40000 && /didn't respond.*Allow remote agent management.*Reload the desktop app/.test(await page.locator('[data-testid="browser-job-text"]').innerText()) && (await page.locator('[data-testid="browser-job-retry"]').count()) === 1, `${Date.now() - tTimeout} ms`);
handlerPaused = false;
await page.locator('[data-testid="browser-job-retry"]').click();
check("7k. Retry sends a fresh command that completes", await page.waitForFunction(() => document.querySelector('[data-testid="browser-job"]')?.getAttribute("data-phase") === "done", null, { timeout: 60000 }).then(() => true, () => false));
// 7n. thumbnail opens a large viewer with New screenshot (in place), Close, Esc and tap-outside
const vis = (id) => page.locator(`[data-testid="${id}"]`);
await page.locator('[data-testid="browser-row-preview-pin-e2e"]').click();
await vis("shot-viewer").waitFor({ timeout: 10000 }).catch(() => {});
check("7n. clicking the thumbnail opens the viewer with title, address, 'Screenshot taken' and the big image", (await vis("shot-viewer-title").count()) === 1 && /example\.com/.test(await vis("shot-viewer-title").innerText()) && /^Screenshot taken/.test(await vis("shot-viewer-time").innerText()) && /^data:image\/jpeg/.test((await vis("shot-viewer-img").getAttribute("src").catch(() => "")) || ""));
await sleep(3500); // per-session screenshot spacing
await vis("shot-viewer-new").click();
check("7o. New screenshot in the viewer shows progress and finishes (image updated in place)", await page.waitForFunction(() => /done/.test(document.querySelector('[data-testid="shot-viewer-status"]')?.textContent || ""), null, { timeout: 60000 }).then(() => true, () => false) && (await vis("shot-viewer-img").count()) === 1);
// 7t. full screen: height fills the page, wheel/+/- zoom, double-click toggle, drag pan, Back/Esc
await vis("shot-viewer-fullscreen").click();
await vis("shot-fullscreen").waitFor({ timeout: 5000 }).catch(() => {});
const fsGeo = await page.evaluate(() => { const st = document.querySelector('[data-testid="shot-fs-stage"]').getBoundingClientRect(); const im = document.querySelector('[data-testid="shot-fs-img"]').getBoundingClientRect(); return { sh: st.height, ih: im.height, top: st.top }; });
check("7t. full screen: the screenshot's height fills the page area (contain by height)", Math.abs(fsGeo.sh - fsGeo.ih) < 2, JSON.stringify(fsGeo));
const scaleOf = () => page.locator('[data-testid="shot-fs-img"]').getAttribute("data-scale");
await vis("shot-fs-in").click();
check("7u. + zooms in", Number(await scaleOf()) > 1.4, await scaleOf());
await page.locator('[data-testid="shot-fs-stage"]').hover();
await page.mouse.wheel(0, -300);
await sleep(300);
check("7v. mouse wheel / trackpad zooms further (real wheel, app scroll-lock bypassed)", Number(await scaleOf()) > 1.6, await scaleOf());
await vis("shot-fs-reset").click();
const sb = await page.locator('[data-testid="shot-fs-stage"]').boundingBox();
await page.mouse.dblclick(sb.x + sb.width / 2, sb.y + sb.height / 2);
await sleep(200);
check("7w. double-click zooms in", Number(await scaleOf()) > 2, await scaleOf());
await page.mouse.dblclick(sb.x + sb.width / 2, sb.y + sb.height / 2);
await sleep(200);
check("7x. double-click again resets to 1x", Number(await scaleOf()) === 1, await scaleOf());
await vis("shot-fs-in").click();
await vis("shot-fs-in").click();
const tr0 = await page.locator('[data-testid="shot-fs-img"]').evaluate((e) => e.style.transform);
await page.mouse.move(sb.x + sb.width / 2, sb.y + sb.height / 2);
await page.mouse.down();
await page.mouse.move(sb.x + sb.width / 2 + 40, sb.y + sb.height / 2 + 30, { steps: 4 });
await page.mouse.up();
const tr1 = await page.locator('[data-testid="shot-fs-img"]').evaluate((e) => e.style.transform);
check("7y. dragging pans when zoomed in", tr0 !== tr1, `${tr0} -> ${tr1}`);
await vis("shot-fs-new").waitFor();
await page.keyboard.press("Escape");
check("7z. Esc goes Back to the viewer (viewer still open, zoom reset on next entry)", await page.waitForFunction(() => !document.querySelector('[data-testid="shot-fullscreen"]') && !!document.querySelector('[data-testid="shot-viewer"]'), null, { timeout: 5000 }).then(() => true, () => false));
await vis("shot-viewer-fullscreen").click();
check("7z2. zoom is reset when re-entering full screen", Number(await scaleOf()) === 1);
await vis("shot-fs-back").click();
await page.setViewportSize({ width: 390, height: 844 });
await sleep(400);
const box = await vis("shot-viewer-panel").boundingBox();
check("7p. phone width: the viewer fits the screen", !!box && box.x >= 0 && box.y >= 0 && box.x + box.width <= 390.5 && box.y + box.height <= 844.5, JSON.stringify(box));
await page.setViewportSize({ width: 1280, height: 800 });
await page.keyboard.press("Escape");
check("7q. Esc closes the viewer", await page.waitForFunction(() => !document.querySelector('[data-testid="shot-viewer"]'), null, { timeout: 5000 }).then(() => true, () => false));
await page.locator('[data-testid="browser-row-preview-pin-e2e"]').click();
await vis("shot-viewer").waitFor();
await page.mouse.click(5, 5);
check("7r. tapping outside closes the viewer", await page.waitForFunction(() => !document.querySelector('[data-testid="shot-viewer"]'), null, { timeout: 5000 }).then(() => true, () => false));
await page.locator('[data-testid="browser-row-preview-pin-e2e"]').click();
await vis("shot-viewer-close").click();
check("7s. Close button closes the viewer", await page.waitForFunction(() => !document.querySelector('[data-testid="shot-viewer"]'), null, { timeout: 5000 }).then(() => true, () => false));
// 7l. parity: same structure as the desktop Browsers screen at desktop and phone widths
const structure = () => page.evaluate(() => ({
  content: !!document.querySelector('[data-testid="browsers-page-content"]'),
  header: /Browsers/.test(document.querySelector('[data-testid="browsers-page-content"] h1, [data-testid="browsers-page-content"] h2')?.textContent || ""),
  add: !!document.querySelector('[data-testid="browsers-add"]'),
  list: !!document.querySelector('ul.divide-y.rounded-lg'),
  thumbs: document.querySelectorAll('[data-testid^="browser-row-preview-"]').length,
  chips: document.querySelectorAll('[data-testid="br-live"], [data-testid="br-cold"]').length,
  robots: document.querySelectorAll('[data-testid="br-robot"]').length,
  overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
}));
await page.screenshot({ path: "/tmp/web-browsers-desktop.png" });
const sd = await structure();
check("7l. desktop width: header, Add, bordered list with thumbnail + Live/Cold chips + robot, no horizontal overflow", sd.content && sd.header && sd.add && sd.list && sd.thumbs >= 2 && sd.chips >= 1 && sd.robots >= 1 && !sd.overflow, JSON.stringify(sd));
await page.setViewportSize({ width: 390, height: 844 });
await sleep(500);
await page.screenshot({ path: "/tmp/web-browsers-phone.png" });
const sp = await structure();
check("7m. phone width: same structure, no horizontal overflow", sp.content && sp.add && sp.list && sp.thumbs >= 2 && !sp.overflow, JSON.stringify(sp));
await page.setViewportSize({ width: 1280, height: 800 });
await page.evaluate(() => { location.hash = "#/agents"; });
// 8. slice B: terminal over the direct connection (member-key challenge per launch)
// the web Buzz term page needs this device's own switch + host (same gate as the nav row)
await page.evaluate(() => localStorage.setItem("buzz-web-remote-prefs", JSON.stringify({ agentsOn: false, terminalOn: true, agentsHost: null, terminalHost: "dev-desktop" })));
await page.reload();
await page.evaluate(() => { location.hash = "#/buzz-term"; });
await page.locator('[data-testid="remote-panel"]').waitFor({ timeout: 60000 }).catch(() => {});
check("8a. remote terminal panel appears on its own Buzz term page (route /buzz-term) from the signed heartbeat", (await page.locator('[data-testid="remote-panel"]').count()) === 1 && /Buzz term/.test(await page.locator('[data-testid="buzz-term-title"]').innerText().catch(() => "")));
check("8a2. Buzz term row is highlighted on its page and Agents is not", await page.evaluate(() => document.querySelector('[data-testid*="open-buzz-term-view"]')?.getAttribute("data-active") === "true" && document.querySelector('[data-testid*="open-agents-view"]')?.getAttribute("data-active") !== "true"));
check("8a3. shell setting: page says 'Using a normal shell', no herdr picker", /Using a normal shell/.test(await page.locator('[data-testid="remote-term-setting"]').innerText().catch(() => "")) && (await page.locator('[data-testid="remote-sessions"]').count()) === 0 && (await page.locator('[data-testid="remote-session-select"]').count()) === 0);
check("8b. no herdr session listing is offered or needed in shell mode, no errors", (await page.locator('[data-testid="remote-message"]').count()) === 0);
await page.locator('[data-testid="remote-open"]').click();
await page.locator('[data-testid="remote-terminal"]').waitFor({ timeout: 60000 }).catch(() => {});
await page.locator('[data-testid="remote-input"]').fill("echo e2e-$((6*7))").catch(() => {});
await page.locator('[data-testid="remote-input"]').press("Enter").catch(() => {});
await page.waitForFunction(() => /e2e-42/.test(document.querySelector('[data-testid="remote-screen"]')?.textContent || "") && (document.querySelector('[data-testid="remote-screen"]')?.textContent || "").split("e2e-42").length > 2, null, { timeout: 30000 }).catch(() => {});
const screenTxt = await page.locator('[data-testid="remote-screen"]').innerText().catch(() => "");
check("8c. terminal opens and runs a command on the desktop shell", screenTxt.split("\n").some((l) => l.trim() === "e2e-42"), screenTxt.slice(-80).replace(/\n/g, "|"));
await page.locator('[data-testid="remote-close"]').click();
await sleep(1000);
raTermHost = "herdr";
await pub.sendHeartbeat();
check("8e. herdr setting: page says 'Using herdr: session work' and offers the herdr session picker defaulting to it", await page.waitForFunction(() => /Using herdr: session work/.test(document.querySelector('[data-testid="remote-term-setting"]')?.textContent || "") && !!document.querySelector('[data-testid="remote-sessions"]') && document.querySelector('[data-testid="remote-session-select"]')?.value === "work", null, { timeout: 30000 }).then(() => true, () => false));
raTermHost = "shell";
await pub.sendHeartbeat();
check("8f. Agents page no longer hosts the terminal panel", await (async () => { await page.evaluate(() => { location.hash = "#/agents"; }); await page.locator('[data-testid="my-agents"]').waitFor({ timeout: 20000 }).catch(() => {}); return (await page.locator('[data-testid="remote-panel"]').count()) === 0; })());
// 9. web Settings: two switches, host pickers, path labels (Direct while the listener is up, Via relay once it is gone)
await page.evaluate(() => { location.hash = "#/settings"; });
await page.getByRole("button", { name: /^Agents$/ }).first().click({ timeout: 20000 }).catch(() => {});
await page.locator('[data-testid="web-remote-agents-toggle"]').waitFor({ timeout: 20000 }).catch(() => {});
check("9a. web Settings → Agents has 'Allow remote agent management' (enabled: a host supports it)", await page.waitForFunction(() => { const e = document.querySelector('[data-testid="web-remote-agents-toggle"]'); return !!e && !e.hasAttribute("disabled"); }, null, { timeout: 30000 }).then(() => true, () => false));
raAgentsOn = false; // management OFF on the desktop: list must still show, with instructions and no controls
await pub.sendHeartbeat();
await page.evaluate(() => { location.hash = "#/projects"; });
await page.locator('[data-testid*="open-agents-view"]').first().waitFor({ timeout: 20000 }).catch(() => {});
check("9a2. Agents nav row is never greyed out (no data-unavailable, full opacity) while management is off", await page.evaluate(() => { const el = document.querySelector('[data-testid*="open-agents-view"]'); return !!el && !el.hasAttribute("data-unavailable") && Number(getComputedStyle(el).opacity) === 1; }));
await page.locator('[data-testid*="open-agents-view"]').first().click().catch(() => {});
check("9a2b. clicking Agents opens the Agents page: bridge list still shown + instructions card for enabling management", await page.waitForFunction(() => !!document.querySelector('[data-testid="my-agents"]') && !!document.querySelector('[data-testid="agents-management-info"]') && /Settings → Agents → Allow remote agent management on your desktop/.test(document.body.innerText) && /Over the relay/.test(document.body.innerText) && /direct tailnet/.test(document.body.innerText), null, { timeout: 15000 }).then(() => true, () => false));
check("9a2c. with management off, agents are listed but there are no start/stop/message controls", (await page.locator('[data-testid="my-agent-row"]').count()) >= 1 && (await page.locator('[data-testid="agent-controls"]').count()) === 0);
raAgentsOn = true;
await pub.sendHeartbeat();
await page.evaluate(() => { location.hash = "#/settings"; });
await page.getByRole("button", { name: /^Agents$/ }).first().click({ timeout: 20000 }).catch(() => {});
await page.locator('[data-testid="web-remote-agents-toggle"]').waitFor({ timeout: 20000 }).catch(() => {});
await page.locator('[data-testid="web-remote-agents-toggle"]').click().catch(() => {});
await page.locator('[data-testid="remote-hosts-agents"]').waitFor({ timeout: 10000 }).catch(() => {});
check("9b. host list shows the host with a 'Direct' label once reachable", await page.waitForFunction(() => /Direct/.test(document.querySelector('[data-testid="remote-hosts-agents"]')?.textContent || ""), null, { timeout: 20000 }).then(() => true, () => false));
await page.getByRole("button", { name: /Buzz Term/ }).first().click({ timeout: 10000 }).catch(() => {});
await page.locator('[data-testid="web-remote-terminal-toggle"]').waitFor({ timeout: 15000 }).catch(() => {});
check("9c. web Settings → Buzz Term has 'Allow remote terminal', enabled while the tailnet host is reachable", await page.waitForFunction(() => !document.querySelector('[data-testid="web-remote-terminal-toggle"]')?.hasAttribute("disabled"), null, { timeout: 20000 }).then(() => true, () => false));
{ // section 8 already turned this device's terminal switch on (so the Buzz term page works): only click if it is off
  const t = page.locator('[data-testid="web-remote-terminal-toggle"]');
  if ((await t.getAttribute("aria-checked").catch(() => null)) !== "true") await t.click().catch(() => {});
}
await page.evaluate(() => { location.hash = "#/agents"; });
check("9a3. Agents nav entry appears once it is on with a host selected", await page.waitForFunction(() => !!document.querySelector('[data-testid*="open-agents-view"]:not([data-unavailable])'), null, { timeout: 15000 }).then(() => true, () => false));
check("9c2. Terminal nav entry appears with a reachable terminal host selected", await page.waitForFunction(() => !!document.querySelector('[data-testid*="open-buzz-term-view"]:not([data-unavailable])'), null, { timeout: 15000 }).then(() => true, () => false));
// 10. agent management over the direct connection (real listener + in-memory agent store; relay stays the fallback)
await page.evaluate(() => { location.hash = "#/agents"; });
await page.locator('[data-testid="agents-path"]').waitFor({ timeout: 20000 }).catch(() => {});
check("10a. with management on and the host reachable the Agents page connects by itself: no click, one signer approval, label 'Direct' with the signature expiry", await page.waitForFunction(() => document.querySelector('[data-testid="agents-path"]')?.getAttribute("data-path") === "direct" && /signed until/.test(document.querySelector('[data-testid="agents-path-line"]')?.textContent || ""), null, { timeout: 60000 }).then(() => true, () => false));
check("10b. no 'Connect directly' button once connected", !(await page.locator('[data-testid="direct-connect"]').count()));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10c. direct list shows ALL agents including stopped ones (relay list shows running only)", await page.waitForFunction(() => document.querySelectorAll('[data-testid="direct-agent-row"]').length >= 3 && !!document.querySelector('[data-testid="direct-agent-row"][data-status="stopped"]'), null, { timeout: 20000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-new"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-create-name"]').fill("E2E Agent").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-create-runtime"]').selectOption("claude").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-create-model"]').fill("opus").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-create-submit"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10d. create: the new agent appears over the direct connection (runtime + model applied)", await page.waitForFunction(() => /Created E2E Agent/.test(document.querySelector('[data-testid="direct-note"]')?.textContent || "") && [...document.querySelectorAll('[data-testid="direct-agent-row"]')].some((r) => /E2E Agent/.test(r.textContent || "") && /claude/.test(r.textContent || "") && /opus/.test(r.textContent || "")), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): const row = () => page.locator('[data-testid="direct-agent-row"]', { hasText: /E2E Agent|E2E Renamed/ }).first();
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await row().locator('[data-testid="direct-edit"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-edit-name"]').fill("E2E Renamed").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-edit-save"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10e. rename works over the direct connection", await page.waitForFunction(() => [...document.querySelectorAll('[data-testid="direct-agent-row"]')].some((r) => /E2E Renamed/.test(r.textContent || "")), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await row().locator('[data-testid="direct-edit"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-edit-provider"]').fill("openai").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-edit-model"]').fill("gpt-x").catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-edit-save"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10f. provider/model edit is saved; a running agent is told to restart for it to apply", await page.waitForFunction(() => /Restart E2E Renamed for the change to apply/.test(document.querySelector('[data-testid="direct-note"]')?.textContent || "") && [...document.querySelectorAll('[data-testid="direct-agent-row"]')].some((r) => /openai/.test(r.textContent || "") && /gpt-x/.test(r.textContent || "")), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): const bravo = () => page.locator('[data-testid="direct-agent-row"]', { hasText: "Bravo" });
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await bravo().locator('[data-testid="direct-start"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10g. start a stopped agent directly", await page.waitForFunction(() => [...document.querySelectorAll('[data-testid="direct-agent-row"]')].some((r) => /Bravo/.test(r.textContent || "") && r.getAttribute("data-status") === "running"), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await bravo().locator('[data-testid="direct-stop"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await page.locator('[data-testid="direct-stop-yes"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10h. stop needs a confirmation and then stops it", await page.waitForFunction(() => [...document.querySelectorAll('[data-testid="direct-agent-row"]')].some((r) => /Bravo/.test(r.textContent || "") && r.getAttribute("data-status") === "stopped"), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): raProc.stdin.write("agents off\n"); // desktop turns "Allow remote agent management" off: every request is refused at once
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await sleep(500);
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): await bravo().locator('[data-testid="direct-start"]').click().catch(() => {});
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): check("10i. with management switched off on the desktop, a direct request is refused with the Settings hint", await page.waitForFunction(() => /Allow remote agent management/.test(document.querySelector('[data-testid="direct-op-error"]')?.textContent || ""), null, { timeout: 30000 }).then(() => true, () => false));
// STALE (agent page now uses the desktop UI over the cmd proxy; fake desktop needs the "cmd" op): raProc.stdin.write("agents on\n");
await page.evaluate(() => { location.hash = "#/settings"; });
await page.getByRole("button", { name: /Buzz Term/ }).first().click({ timeout: 10000 }).catch(() => {});
raProc.kill();
await sleep(1000);
await page.evaluate(() => { location.hash = "#/projects"; }); // the left nav is not rendered on the Settings view
check("9d0. Terminal nav row greys out when the host becomes unreachable", await page.waitForFunction(() => !!document.querySelector('[data-testid*="open-buzz-term-view"][data-unavailable="true"]'), null, { timeout: 40000 }).then(() => true, () => false));
check("9d0b. web nav row is named 'Buzz term' (not 'Terminal'), plain grey, not crossed out", await page.evaluate(() => { const el = document.querySelector('[data-testid*="open-buzz-term-view"][data-unavailable="true"]'); return !!el && /Buzz term/.test(el.textContent || "") && !/Terminal/.test(el.textContent || "") && Number(getComputedStyle(el).opacity) < 1 && !/line-through/.test(getComputedStyle(el).textDecorationLine); }));
await page.locator('[data-testid*="open-buzz-term-view"]').first().click().catch(() => {});
check("9d0c. clicking the greyed Buzz term row opens its instructions page (not a toast) with the exact settings", await page.waitForFunction(() => !!document.querySelector('[data-testid="remote-unavailable-page"][data-feature="term"]') && /Buzz term isn't available/.test(document.body.innerText) && /Settings → Buzz Term → Allow remote terminal/.test(document.body.innerText) && /tailnet/.test(document.body.innerText) && location.hash.includes("/buzz-term"), null, { timeout: 8000 }).then(() => true, () => false));
check("9d0d. on the Buzz term instructions page the Buzz term row is highlighted (still grey) and Agents is not", await page.evaluate(() => { const t = document.querySelector('[data-testid*="open-buzz-term-view"]'); const a = document.querySelector('[data-testid*="open-agents-view"]'); return t?.getAttribute("data-active") === "true" && t?.getAttribute("data-unavailable") === "true" && Number(getComputedStyle(t).opacity) < 1 && a?.getAttribute("data-active") !== "true"; }));
await page.evaluate(() => { location.hash = "#/settings"; });
await page.getByRole("button", { name: /Buzz Term/ }).first().click({ timeout: 10000 }).catch(() => {});
check("9d. host gone: terminal switch greys out; agent management falls back to 'Via relay'", await page.waitForFunction(() => document.querySelector('[data-testid="web-remote-terminal-toggle"]')?.hasAttribute("disabled"), null, { timeout: 40000 }).then(() => true, () => false));
await page.getByRole("button", { name: /^Agents$/ }).first().click().catch(() => {});
check("9e. agents path label says 'Via relay' with the slower/limited note", await page.waitForFunction(() => /Via relay/.test(document.querySelector('[data-testid="remote-hosts-agents"]')?.textContent || "") && !!document.querySelector('[data-testid="remote-host-relay-note"]'), null, { timeout: 40000 }).then(() => true, () => false));
await page.evaluate(() => { location.hash = "#/agents"; });
await page.locator('[data-testid="my-agents"]').waitFor({ timeout: 20000 }).catch(() => {});

await page.evaluate(() => { location.hash = "#/buzz-term"; });
check("8d. listener gone -> the Buzz term page shows its instructions (tailnet / Allow remote terminal), no terminal panel", await page.waitForFunction(() => !!document.querySelector('[data-testid="remote-unavailable-page"]') && /tailnet/.test(document.body.innerText) && !document.querySelector('[data-testid="remote-panel"]'), null, { timeout: 30000 }).then(() => true, () => false));
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
