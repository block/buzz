// Buzz term on the web: the desktop's real Buzz Term (emulator in the remote listener, renderer/substrate/splash in the page).
// The desktop side is the REAL BridgePublisher + BridgeCommandHandler (termHandoff executor faked), the remote terminal is the real
// buzz-remote-access listener harness (loopback). Needs local relay (:3100), a web dev server, and RA_SERVE_BIN (default /tmp/ra-target/debug/examples/serve).
// Run: node --import ./test-loader.mjs --experimental-strip-types scripts/pwa/s16-buzzterm-e2e.mjs [appUrl]
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
page.on("console", (m) => { if (["error","warning"].includes(m.type()) && !/favicon|DevTools/.test(m.text())) console.log("console." + m.type(), m.text().slice(0, process.env.S16_FULL ? 3000 : 240)); });
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


const TERM = 'section[aria-label="Buzz Term"]';
const rowsText = () => page.evaluate(() => [...document.querySelectorAll("[data-terminal-selection-row]")].map((r) => r.textContent.replace(/\u00a0/g, " ")).join("\n"));
const waitText = async (re, ms = 20000) => {
  const end = Date.now() + ms;
  let t = "";
  while (Date.now() < end) {
    t = await rowsText();
    if (re.test(t)) return t;
    await sleep(150);
  }
  return t;
};
const type = async (s) => {
  await page.locator(`${TERM} textarea`).focus();
  await page.keyboard.type(s, { delay: 8 });
};

await page.goto(`${APP}#/buzz-term`);
await page.waitForSelector(TERM, { timeout: 60000 }).catch(() => {});
await sleep(2000);
if (process.env.S16_DEBUG) { console.log("PAGE:", (await page.locator("body").innerText()).slice(0, 600)); await page.screenshot({ path: "/tmp/s16-a.png" }); }
check("a. #/buzz-term shows the real Buzz Term substrate (maximized), not the basic text box", (await page.locator(TERM).count()) === 1 && (await page.locator('[data-testid="remote-screen"]').count()) === 0 && (await page.locator(`${TERM}[data-terminal-mode="maximized"]`).count()) === 1);
const sawSplash = await page.waitForSelector(`${TERM} canvas.buzz-terminal-welcome`, { timeout: 8000 }).then(() => true).catch(() => false);
check("b. the Buzz Term splash/banner plays on first open", sawSplash);
await waitText(/\$|%|#|>/, 20000);
await type("echo FE_$((6*7))\n");
check("c. typing reaches the desktop shell and its output comes back through the emulator", /FE_42/.test(await waitText(/FE_42/)), (await rowsText()).slice(-120));

await type("printf '\\033[31mREDTEXT\\033[0m \\033[1;44mBLUEBG\\033[0m\\n'\n");
await waitText(/REDTEXT BLUEBG/);
await sleep(500);
const px = await page.evaluate(() => {
  const cv = document.querySelector('section[aria-label="Buzz Term"] [data-testid="buzz-terminal-viewport"] canvas:not(.buzz-terminal-welcome)');
  const g = cv.getContext("2d", { alpha: false });
  const d = g.getImageData(0, 0, cv.width, cv.height).data;
  let red = 0, blue = 0;
  for (let i = 0; i < d.length; i += 4) {
    if (d[i] > 170 && d[i + 1] < 110 && d[i + 2] < 130) red++;
    if (d[i + 2] > 140 && d[i] < 80 && d[i + 1] < 140) blue++;
  }
  return { red, blue, w: cv.width, h: cv.height };
});
if (process.env.S16_DEBUG) await page.screenshot({ path: "/tmp/s16-d.png" });
check("d. ANSI colors are really painted on the canvas (red text, blue background)", px.red > 20 && px.blue > 50, JSON.stringify(px));

await type("printf '\\033[?1049h\\033[2J\\033[5;10HALT_%s' SCREEN_OK\n");
check("e. full-screen apps use the alternate screen (cursor addressing works)", /ALT_SCREEN_OK/.test(await waitText(/ALT_SCREEN_OK/)));
await type("printf '\\033[?1049l'\n");
await waitText(/FE_42/);
const fText = await rowsText();
check("f. leaving the alternate screen restores the shell screen", /FE_42/.test(fText) && !/ALT_SCREEN_OK/.test(fText), JSON.stringify(fText.slice(0, 300)));

// mouse reporting (herdr relies on it): enable SGR mouse tracking, click, and the escape sequence must reach the program
await type("printf '\\033[?1000h\\033[?1006h'; cat -v\n");
await sleep(600);
const vp = await page.locator('[data-testid="buzz-terminal-viewport"] canvas').first().boundingBox();
await page.mouse.click(vp.x + 100, vp.y + 60);
check("g. mouse clicks are forwarded as SGR sequences while the program tracks the mouse", /\^\[\[<0;\d+;\d+M/.test(await waitText(/\^\[\[<0;\d+;\d+M/, 8000)), (await rowsText()).slice(-100));
await page.keyboard.press("Control+C");
await type("printf '\\033[?1000l\\033[?1006l'\n");
await sleep(300);

// resize: the PTY has the size the canvas has
await type("clear; stty size\n");
const size1 = await waitText(/\d+ \d+/);
const cols1 = Number(/(\d+) (\d+)\s*$/m.exec(size1.split("\n").filter((l) => /^\d+ \d+\s*$/.test(l.trim())).pop() ?? "")?.[2] ?? 0);
await page.setViewportSize({ width: 900, height: 700 });
await sleep(1500);
await type("clear; stty size\n");
await sleep(800);
const size2 = await waitText(/\d+ \d+/);
const cols2 = Number(/(\d+) (\d+)\s*$/m.exec(size2.split("\n").filter((l) => /^\d+ \d+\s*$/.test(l.trim())).pop() ?? "")?.[2] ?? 0);
check("h. resizing the window resizes the desktop PTY and re-flows the screen", cols1 > 0 && cols2 > 0 && cols2 < cols1, `${cols1} -> ${cols2}`);

// scrollback via wheel
await type("clear; i=0; while [ $i -lt 150 ]; do echo SB_LINE_$i; i=$((i+1)); done\n");
await waitText(/SB_LINE_149/);
await page.mouse.move(vp.x + 100, vp.y + 100);
await page.mouse.wheel(0, -900);
await sleep(800);
check("i. wheel scrolls back through history", !/SB_LINE_149/.test(await rowsText()) && /SB_LINE_/.test(await rowsText()), (await rowsText()).slice(-60));
await type("\n");
await waitText(/SB_LINE_149/);

// a second tab = a second launch (another signature) on the same connection
const opensBefore = await page.evaluate(() => performance.getEntriesByType("resource").length);
await page.click('button[aria-label="New Buzz Term tab"]');
await page.waitForFunction(() => document.querySelectorAll(".buzz-terminal-tab").length === 2, null, { timeout: 30000 }).catch(() => {});
check("j. the + button opens a second desktop shell tab", (await page.locator(".buzz-terminal-tab").count()) === 2);
await sleep(2500); // let the new shell print its prompt (keys typed before the PTY is attached are dropped, as on the desktop)
await type("echo TAB_TWO_$((5*5))\n");
const kText = await waitText(/TAB_TWO_25/);
check("k. the second tab is its own shell", /TAB_TWO_25/.test(kText), JSON.stringify(kText.slice(-200)));
if (process.env.S16_DEBUG) await page.screenshot({ path: "/tmp/s16-k.png" });

// leave and come back: sessions survive
await page.click('button[aria-label="Hide Buzz Term"]');
await sleep(500);
check("l. Hide returns to the Buzz term launcher page", (await page.locator('[data-testid="buzz-term-launcher"]').count()) === 1);
await page.click('[data-testid="buzz-term-open"]');
await page.waitForSelector(TERM, { timeout: 15000 });
check("m. re-opening shows the same tabs (sessions kept)", (await page.locator(".buzz-terminal-tab").count()) === 2);

// terminals OFF on the desktop: clear message, no terminal
terminalsOn = false;
await pub.sendHeartbeat();
await sleep(2500);
await page.goto(`${APP}#/buzz-term`);
await sleep(1500);
const txt = await page.locator('[data-testid="buzz-term-page"]').innerText().catch(() => "");
check("n. with Allow remote terminal off the page says so (no broken terminal)", /Allow remote terminal/.test(txt) && (await page.locator(TERM).count()) === 0, txt.slice(0, 160));

clearInterval(poll);
pub.destroy();
raProc.kill();
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
