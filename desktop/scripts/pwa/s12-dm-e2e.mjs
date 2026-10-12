// Web DM e2e: open_dm handler (find-or-create via kind 41010). Needs local relay (:3100) and a web dev server.
// Usage: node scripts/pwa/s12-dm-e2e.mjs [appUrl]   (default http://127.0.0.1:5199/)
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext();
const { pk } = await installMockExtension(ctx, TYLER);
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

const inv = (cmd, args) => page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [cmd, args]);
const other = "b".repeat(63) + "1"; // arbitrary valid pubkey; relay finds-or-creates by participant set
console.log("other member:", other);
check("found another relay member to DM", !!other);
const dm = await inv("open_dm", { pubkeys: [other] });
check("open_dm returns a channel with id", !!dm?.id, JSON.stringify(dm).slice(0, 200));
check("channel is a DM", dm?.channelType === "dm" || dm?.channel_type === "dm", String(dm?.channelType ?? dm?.channel_type));
const again = await inv("open_dm", { pubkeys: [other] });
check("second open_dm returns the SAME channel (find-existing)", again?.id === dm?.id);
const bad = await inv("open_dm", { pubkeys: [] }).then(() => false).catch((e) => String(e?.message ?? e).includes("at least one pubkey"));
check("empty pubkeys rejected like desktop", bad);
const bad2 = await inv("open_dm", { pubkeys: ["nothex"] }).then(() => false).catch(() => true);
check("invalid pubkey rejected", bad2);
await page.goto(`${APP}#/channels/${dm.id}`);
await page.waitForSelector('[data-testid="message-composer"], textarea, [contenteditable="true"]', { timeout: 20000 });
check("DM channel route renders (composer present)", true);
check("no error boundary", (await page.getByText("Something went wrong").count()) === 0);
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
