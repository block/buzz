// Web thread-history e2e: opening a thread loads its replies (get_thread_replies over HTTP /query, read priority). Needs the local relay (:3100) and a web dev server.
// Usage: node scripts/pwa/s9-thread-e2e.mjs [appUrl]   (default http://127.0.0.1:5311/)
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5311/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const _PNG = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR4nGP47+CAFTEMLQkAmbNfwaFk8aAAAAAASUVORK5CYII=";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext();
const { log, pk } = await installMockExtension(ctx, TYLER);
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });


const seeded = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  const general = chs.find((c) => c.name === "general") ?? chs[0];
  const stamp = Date.now();
  const root = await window.__buzzWebInvoke("send_channel_message", { channelId: general.id, content: `thread root ${stamp}` });
  const rootId = root.event_id ?? root.eventId ?? root.id;
  for (let i = 1; i <= 5; i++)
    await window.__buzzWebInvoke("send_channel_message", { channelId: general.id, content: `reply ${i} ${stamp}`, parentEventId: rootId });
  return { channelId: general.id, rootId, stamp };
});
check("seeded a root + 5 replies", !!seeded.rootId, JSON.stringify(seeded));

const direct = await page.evaluate(async (s) => {
  const t0 = performance.now();
  const r = await window.__buzzWebInvoke("get_thread_replies", { rootEventId: s.rootId, channelId: s.channelId, limit: null, depthLimit: null, cursor: null });
  return { n: r.events.filter((e) => e.kind === 9).length, next: r.next_cursor, ms: Math.round(performance.now() - t0) };
}, seeded);
check("get_thread_replies returns the 5 replies (no NativeOnlyError)", direct.n === 5 && direct.next === null, JSON.stringify(direct));

// Slow settings decrypts must not delay the thread read: hold the extension for nip44 only via a stalled decrypt in flight
await page.evaluate(() => {
  const real = window.nostr.nip44.decrypt;
  window.nostr.nip44.decrypt = async (...a) => { await new Promise((r) => setTimeout(r, 20000)); return real(...a); };
});
await page.goto(`${APP}#/channels/${seeded.channelId}`);
const rootRow = page.locator(`text=thread root ${seeded.stamp}`).first();
await rootRow.waitFor({ timeout: 30000 }).catch(() => {});
await rootRow.hover().catch(() => {});
await page.locator('[data-testid="message-row"]', { hasText: `thread root ${seeded.stamp}` }).first().getByText(/View thread|repl/).first().click({ timeout: 10000 }).catch(async () => {
  await page.getByText(/View thread/).last().click({ timeout: 5000 }).catch(() => {});
});
const t0 = Date.now();
const got = await page.waitForSelector(`text=reply 5 ${seeded.stamp}`, { timeout: 15000 }).catch(() => null);
const errCard = await page.$('[data-testid="message-thread-replies-error"]');
check("thread panel shows replies promptly with a stalled settings decrypt", !!got && !errCard, `${Date.now() - t0}ms`);
if (!got || errCard) console.log("DEBUG", page.url(), (await page.innerText("body")).replace(/\s+/g, " ").slice(0, 600));
await page.screenshot({ path: "/tmp/s9-thread.png" });

await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
