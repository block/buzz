// "Pass thread context to Buzz Term" on the web: picker data from the desktop (relay bridge), term.handoff command, auto-open of Buzz term.
// The desktop side is the REAL BridgePublisher + BridgeCommandHandler (termHandoff executor faked), the remote terminal is the real
// buzz-remote-access listener harness (loopback). Needs local relay (:3100), a web dev server, and RA_SERVE_BIN (default /tmp/ra-target/debug/examples/serve).
// Run: node --import ./test-loader.mjs --experimental-strip-types scripts/pwa/s14-term-handoff-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { finalizeEvent, getPublicKey, nip44 } from "nostr-tools";
import { installMockExtension } from "./mock-nip07.mjs";
import { BridgePublisher } from "../../src/features/bridge/publisher.ts";
import { BridgeCommandHandler, makeSeenSet } from "../../src/features/bridge/commandHandler.ts";
import { nowS } from "../../src/features/bridge/protocol.ts";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const RELAY_HTTP = "http://localhost:3100";
const SK_HEX = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const sk = new Uint8Array(Buffer.from(SK_HEX, "hex"));
const myPk = getPublicKey(sk);
const selfKey = nip44.v2.utils.getConversationKey(sk, myPk);
const AG = "ab".repeat(32);
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const signNode = (e) => finalizeEvent({ kind: e.kind, content: e.content, created_at: e.createdAt, tags: e.tags }, sk);
const authHeader = (url, bytes) =>
  "Nostr " +
  Buffer.from(
    JSON.stringify(
      finalizeEvent(
        { kind: 27235, content: "", created_at: nowS(), tags: [["u", url], ["method", "POST"], ["payload", createHash("sha256").update(bytes).digest("hex")], ["nonce", randomUUID()]] },
        sk,
      ),
    ),
  ).toString("base64");
async function postEvent(ev) {
  const url = `${RELAY_HTTP}/events`;
  const bytes = Buffer.from(JSON.stringify(ev));
  const res = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json", Authorization: authHeader(url, bytes) }, body: bytes });
  if (!res.ok) throw new Error(`publish ${res.status}: ${await res.text()}`);
}

const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
await installMockExtension(ctx, SK_HEX);
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk: myPk });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", String(e).slice(0, 200)));
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });
const seeded = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  const general = chs.find((c) => c.name === "general") ?? chs[0];
  const stamp = Date.now();
  const root = await window.__buzzWebInvoke("send_channel_message", { channelId: general.id, content: `handoff root ${stamp}` });
  await window.__buzzWebInvoke("send_channel_message", { channelId: general.id, content: `handoff reply ${stamp}`, parentEventId: root.event_id ?? root.eventId ?? root.id });
  return { channelId: general.id, rootId: root.event_id ?? root.eventId ?? root.id, stamp };
});

// Desktop: the real listener harness + real publisher/handler.
const RA_BIN = process.env.RA_SERVE_BIN || "/tmp/ra-target/debug/examples/serve";
const raProc = spawn(RA_BIN, [myPk, new URL(APP).origin], { stdio: ["pipe", "pipe", "inherit"], env: { ...process.env, RA_MGMT: "fake" } });
const raAddr = await new Promise((resolve, reject) => {
  const t = setTimeout(() => reject(new Error("serve harness did not start")), 20000);
  raProc.stdout.on("data", (d) => {
    const m = /listening (\S+)/.exec(String(d));
    if (m) { clearTimeout(t); resolve(m[1]); }
  });
});
process.on("exit", () => raProc.kill());
let terminalsOn = true;
let mgmtOn = true;
let harnesses = [{ id: "claude", available: true }, { id: "codex", available: false }];
const pub = new BridgePublisher(
  { encryptToSelf: async (p) => nip44.v2.encrypt(p, selfKey), signEvent: async (e) => signNode(e), publish: postEvent },
  {
    deviceId: "e2e-desktop",
    platform: "e2e",
    caps: ["agents.snapshot", "transcript.chunk", "agent.start", "agent.stop", "agent.message", "term.handoff"],
    harnesses: () => harnesses,
    remote: () => ({ desktopId: "dev-desktop", name: "E2E Mac", url: `ws://${raAddr}/v1/ws`, status: "on", terminals: terminalsOn, termHost: "shell", agents: mgmtOn, sigTtlS: 3600 }),
    onError: (e) => console.log("PUBLISHER ERROR", String(e).slice(0, 300)),
  },
);
pub.start();
pub.update({
  generatedAt: nowS(),
  agents: [
    { pubkey: AG, name: "Stitch", status: "running", activeTurns: [], channelIds: [seeded.channelId] },
    { pubkey: "cd".repeat(32), name: "ElsewhereBot", status: "running", activeTurns: [], channelIds: ["some-other-channel"] },
  ],
});
const handoffCalls = [];
const handler = new BridgeCommandHandler({
  memberPubkey: myPk,
  sessionKey: () => ({ kid: pub.kid, key: pub.sessionKey }),
  agents: () => pub.agents(),
  seen: makeSeenSet(() => [], () => {}),
  executors: { start: async () => {}, stop: async () => {}, message: async () => {} },
  publishResult: (id, b) => pub.publishResult(id, b),
  agentManagementAllowed: () => mgmtOn,
  termHandoff: async (input) => {
    handoffCalls.push(input);
    await sleep(1500); // the agent "writes the card"
    return { sid: "sid-e2e", name: "E2E handoff", tool: "claude", launchCommand: "echo HANDOFF_E2E_$((6*7))", channelId: input.channelId, threadId: input.threadId };
  },
});
const seenEv = new Set();
const poll = setInterval(async () => {
  try {
    const url = `${RELAY_HTTP}/query`;
    const bytes = Buffer.from(JSON.stringify([{ kinds: [30078], authors: [myPk], "#t": ["buzz-bridge"], since: nowS() - 120, limit: 100 }]));
    const r = await fetch(url, { method: "POST", headers: { "Content-Type": "application/json", Authorization: authHeader(url, bytes) }, body: bytes });
    for (const ev of await r.json()) {
      if (seenEv.has(ev.id)) continue;
      seenEv.add(ev.id);
      await handler.handle(ev);
    }
  } catch {}
}, 1000);

async function openThreadPanel() {
  await page.goto(`${APP}#/channels/${seeded.channelId}`);
  const row = page.locator('[data-testid="message-row"]', { hasText: `handoff root ${seeded.stamp}` }).first();
  await row.waitFor({ timeout: 30000 });
  await row.hover();
  await row.getByText(/View thread|repl/).first().click({ timeout: 10000 }).catch(async () => {
    await page.getByText(/View thread/).last().click({ timeout: 5000 }).catch(() => {});
  });
  await page.locator('[data-testid="thread-term-session-button"]').waitFor({ timeout: 20000 });
}

await openThreadPanel();
await page.click('[data-testid="thread-term-session-button"]');
const sel = page.locator('[data-testid="thread-term-session-agent"]');
await sel.waitFor({ timeout: 60000 }).catch(() => {});
const agentOpts = await sel.locator("option").allInnerTexts().catch(() => []);
check("a. picker lists the desktop's running agents in THIS channel only", agentOpts.some((t) => /Stitch/.test(t)) && !agentOpts.some((t) => /ElsewhereBot/.test(t)), agentOpts.join("|"));
const harnessOpts = await page.locator('[data-testid="thread-term-session-harness"] option').allInnerTexts().catch(() => []);
check("b. harnesses come from the desktop: Claude installed, Codex (not installed)", harnessOpts.some((t) => t.trim() === "Claude") && harnessOpts.some((t) => /Codex \(not installed\)/.test(t)), harnessOpts.join("|"));
check("c. the 'No running local agents' / 'Claude (not installed)' desktop-state copy is gone", !(await page.locator('[data-testid="thread-term-session-panel"]').innerText()).match(/No running local agents|Claude \(not installed\)/));

// d. terminals ON: Go → ack → #/buzz-term, terminal opens and the launch line runs
await sel.selectOption({ label: "Stitch" });
await page.click('[data-testid="thread-term-session-go"]');
await page.waitForFunction(() => location.hash.includes("buzz-term"), null, { timeout: 90000 }).catch(() => {});
check("d. after the desktop's ack the web navigates to #/buzz-term", page.url().includes("buzz-term"), page.url());
check("d2. the desktop got exactly one handoff for the chosen agent/thread/harness", handoffCalls.length === 1 && handoffCalls[0].agentPubkey === AG && handoffCalls[0].threadId === seeded.rootId && handoffCalls[0].channelId === seeded.channelId && handoffCalls[0].harness === "claude", JSON.stringify(handoffCalls));
const screen = page.locator('[data-testid="remote-screen"]');
await screen.waitFor({ timeout: 90000 }).catch(() => {});
await page.waitForFunction(() => /HANDOFF_E2E_42/.test(document.querySelector('[data-testid="remote-screen"]')?.textContent || ""), null, { timeout: 30000 }).catch(() => {});
check("e. Buzz term opened by itself and ran the session's launch line", /HANDOFF_E2E_42/.test(await screen.innerText().catch(() => "")), (await screen.innerText().catch(() => "")).slice(-120));

// f. terminals OFF: clear message, no navigation
terminalsOn = false;
await pub.sendHeartbeat();
await sleep(1500);
await openThreadPanel();
await page.click('[data-testid="thread-term-session-button"]');
await sel.waitFor({ timeout: 60000 });
await sel.selectOption({ label: "Stitch" });
// wait for the heartbeat (terminals off) to be seen
await page.waitForFunction(() => true);
await sleep(2000);
await page.click('[data-testid="thread-term-session-go"]');
const note = page.locator('[data-testid="thread-term-session-note"]');
await note.waitFor({ timeout: 90000 }).catch(() => {});
const noteText = await note.innerText().catch(() => "");
check("f. with Allow remote terminal off: stays put and says the session started on the desktop and what to enable", !page.url().includes("buzz-term") && /started on your desktop/.test(noteText) && /Allow remote terminal/.test(noteText), noteText);

// g. agent management OFF on the desktop: refused, nothing runs
mgmtOn = false;
const before = handoffCalls.length;
await openThreadPanel();
await page.click('[data-testid="thread-term-session-button"]');
await sel.waitFor({ timeout: 60000 });
await sel.selectOption({ label: "Stitch" });
await page.click('[data-testid="thread-term-session-go"]');
const err = page.locator('[data-testid="thread-term-session-error"]');
await err.waitFor({ timeout: 60000 }).catch(() => {});
check("g. with Allow remote agent management off the command is refused and shown", handoffCalls.length === before && /Remote agent management is off/.test(await err.innerText().catch(() => "")), await err.innerText().catch(() => ""));

clearInterval(poll);
pub.destroy();
raProc.kill();
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
