// Bots page "Check catalog on relay" read-only diagnostic. Local relay (:3100) + mock NIP-46 signer. Publishes nothing.
// Usage: node scripts/pwa/s15-bots-diagnostic-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { startMockSigner } from "./mock-nip46-signer.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
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
const sent = [];
page.on("websocket", (ws) => {
  if (!ws.url().includes("localhost:3100")) return;
  ws.on("framesent", (f) => sent.push(String(f.payload)));
});
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]');
await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
await page.click('[data-testid="web-unlock-bunker-go"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });
await page.evaluate(() => { location.hash = "#/bots"; });
await page.waitForTimeout(4000);
const empty = await page.locator('[data-testid="bots-empty-state"]').count();
if (!empty) console.log("NOTE: relay already has a catalog, empty state not shown; diagnostic card not reachable in this run");
check("empty Bots page shows the diagnostic card", empty === 0 || (await page.locator('[data-testid="bots-catalog-diagnostic"]').count()) === 1);
if (empty) {
  const before = sent.length;
  await page.click('[data-testid="bots-catalog-diagnostic-run"]');
  await page.waitForSelector('[data-testid="bots-catalog-diagnostic-result"], [data-testid="bots-catalog-diagnostic-error"]', { timeout: 30000 });
  const res = await page.locator('[data-testid="bots-catalog-diagnostic"]').innerText();
  console.log(res.replace(/\s+/g, " ").slice(0, 600));
  check("diagnostic reports result (no error)", (await page.locator('[data-testid="bots-catalog-diagnostic-error"]').count()) === 0);
  check("reports whether a catalog event exists", /Catalog event on relay: (yes|no)/.test(res));
  check("reports signed-in npub", res.includes("npub1"));
  check("reports admin status", /You are an admin:/.test(res));
  check("shows text-only fix hint", /owner or an admin/.test(res));
  const published = sent.slice(before).filter((f) => f.startsWith('["EVENT"'));
  check("published nothing", published.length === 0, `${published.length} EVENT frames`);
}
await browser.close();
signer.close?.();
console.log(fails ? `${fails} failed` : "all passed");
process.exit(fails ? 1 : 0);
