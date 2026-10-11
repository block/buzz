// Web profile-panel e2e: opening a profile (?profile=<pubkey>) must not crash into the error boundary.
// Usage: node scripts/pwa/s10-profile-e2e.mjs [appUrl]   (default http://127.0.0.1:5199/)
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
const errors = [];
page.on("pageerror", (e) => { errors.push(e.stack || e.message); });
page.on("console", (m) => { if (m.type() === "error") errors.push("console: " + m.text() + (m.location()?.url ? " @" + m.location().url : "")); });
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });

const info = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  const general = chs.find((c) => c.name === "general") ?? chs[0];
  return { channelId: general.id };
});
// Surface commands rejected while the profile panel is open (native-only placeholders)
await page.evaluate(() => {
  const inv = window.__buzzWebInvoke;
  window.__rejected = [];
  window.__buzzWebInvoke = async (c, a) => { try { return await inv(c, a); } catch (e) { window.__rejected.push(c + ": " + String(e.message).slice(0, 80)); throw e; } };
});
const others = [pk, "00".repeat(31) + "01"];
for (const target of others) {
  errors.length = 0;
  await page.goto(`${APP}#/channels/${info.channelId}?profile=${target}`);
  await page.waitForTimeout(4000);
  const boundary = await page.getByText("Something went wrong").count();
  check(`profile ${target.slice(0, 8)} opens without error boundary`, boundary === 0, boundary ? errors.slice(0, 3).join("\n") : "");
}
console.log("rejected commands:", JSON.stringify(await page.evaluate(() => window.__rejected ?? [])));
await page.screenshot({ path: "/tmp/s10-profile.png" });
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
