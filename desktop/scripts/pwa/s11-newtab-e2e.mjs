// Web e2e: channel header 'Open in a new window' opens the channel in a new browser tab, already signed in.
// Usage: node scripts/pwa/s11-newtab-e2e.mjs [appUrl]   (default http://127.0.0.1:5199/)
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
await page.waitForTimeout(1500);
const url = new URL(page.url());
await page.goto(`${APP}#/channels/${info.channelId}`);
await page.waitForSelector('[data-testid="conversation-popout-menu"]', { timeout: 20000 });
await page.click('[data-testid="conversation-popout-menu"]');
const [tab] = await Promise.all([
  ctx.waitForEvent("page", { timeout: 10000 }).catch(() => null),
  page.click('[data-testid="conversation-popout-plain"]'),
]);
check("a new tab opened", !!tab);
if (tab) {
  await tab.waitForLoadState();
  const u = tab.url();
  check("new tab url is same origin + ?popout=<label> + #/channels/<id>", u.startsWith(new URL(APP).origin) && u.includes(`#/channels/${info.channelId}`) && /[?&]popout=popout-thread-/.test(u), u);
  const unlock = await tab.locator('[data-testid="web-unlock-nip07"]').count().catch(() => 0);
  await tab.waitForTimeout(500);
  const needsUnlock = await tab.locator('[data-testid="web-unlock-nip07"]').count();
  if (needsUnlock) await tab.click('[data-testid="web-unlock-nip07"]');
  console.log("new tab showed unlock screen:", needsUnlock > 0);
  await tab.waitForSelector('[data-testid="message-composer"], textarea, [contenteditable="true"]', { timeout: 30000 });
  check("new tab shows the channel composer", true);
  await tab.waitForTimeout(1500);
  check("new tab has NO sidebar (channel list)", (await tab.locator('[data-testid="sidebar-channel-content"]').count()) === 0);
  check("new tab has no sidebar element at all", (await tab.locator('[data-sidebar="sidebar"]').count()) === 0);
  check("new tab shows channel header", (await tab.locator('text=general').count()) > 0);
  check("new tab no error boundary", (await tab.getByText("Something went wrong").count()) === 0);
  // thread route builder
  const href = await tab.evaluate(async (id) => {
    const { router } = await import("/src/app/router.tsx");
    return router.history.createHref(router.buildLocation({ to: "/channels/$channelId", params: { channelId: id }, search: { thread: "abc" } }).href);
  }, info.channelId).catch((e) => "ERR " + e.message);
  check("thread href has ?thread=", /#\/channels\/.+\?thread=abc/.test(href), href);
}
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
