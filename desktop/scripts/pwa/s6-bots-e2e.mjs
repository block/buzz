// Web Bots directory e2e: does /#/bots show the relay's kind:30624 community-bots catalog in the web build?
// Local relay (:3100) + mock NIP-46 signer. Usage: node scripts/pwa/s6-bots-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { getPublicKey } from "nostr-tools";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import { client } from "./relay-client.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100", RELAY_HTTP = "http://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const botSk = "11".repeat(32);
const botPk = getPublicKey(new Uint8Array(Buffer.from(botSk, "hex")));
const other = client(RELAY_HTTP, TYLER);
const signer = await startMockSigner({ userSkHex: TYLER });
const browser = await chromium.launch();
const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk: signer.userPk });
const page = await ctx.newPage();
const wsFrames = [];
page.on("websocket", (ws) => {
  if (!ws.url().includes("localhost:3100")) return;
  ws.on("framesent", (f) => wsFrames.push([">", String(f.payload)]));
  ws.on("framereceived", (f) => wsFrames.push(["<", String(f.payload)]));
});
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]');
await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
await page.click('[data-testid="web-unlock-bunker-go"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });
const body = async () => (await page.locator("body").innerText()) || "";

// A. what is on the relay for the catalog right now?
const existing = await other.query([{ kinds: [30624], "#d": ["buzz:community-bots"], limit: 50 }]).catch((e) => String(e));
console.log("relay catalog events before:", JSON.stringify(existing).slice(0, 300));

// B. publish the catalog as TYLER and see whether the relay accepts it
const payload = JSON.stringify({ version: 1, bots: [{ id: "e2e-bot", name: "E2E Catalog Bot", pubkey: botPk, source: "openclaw" }] });
let pub;
try { pub = await other.publish(30624, [["d", "buzz:community-bots"]], payload); } catch (e) { pub = String(e); }
console.log("publish result:", JSON.stringify(pub).slice(0, 300));
const accepted = typeof pub === "object" && pub && (pub.accepted === true || pub.ok === true || !String(JSON.stringify(pub)).match(/reject|restrict|error/i));
check("relay accepts a 30624 catalog from this identity", accepted, JSON.stringify(pub).slice(0, 120));

// C. web Bots directory shows it
await page.evaluate(() => { location.hash = "#/bots"; });
await page.reload();
await page.waitForSelector('[data-testid="sidebar-channel-content"]', { timeout: 30000 }).catch(() => null);
await page.waitForTimeout(6000);
const t = await body();
console.log("bots page text:", t.replace(/\s+/g, " ").slice(0, 400));
check("web /#/bots lists the catalog bot", /E2E Catalog Bot/.test(t));
check("web /#/bots is not the empty state", !/No community bots yet/.test(t));
const reqs = wsFrames.filter(([d, f]) => d === ">" && f.includes("30624"));
check("web client issued a REQ for kind 30624", reqs.length > 0, reqs[0]?.[1]?.slice(0, 160));
const resp = wsFrames.filter(([d, f]) => d === "<" && f.includes("30624"));
console.log("30624 frames received:", resp.length);
await browser.close();
signer.close?.();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
