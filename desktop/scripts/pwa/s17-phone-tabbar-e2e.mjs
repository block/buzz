import fs from "node:fs"; import http from "node:http"; import path from "node:path";
import { chromium, devices } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S17_DIST || "/tmp/phone-dist";
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png",".webmanifest":"application/manifest+json",".woff2":"font/woff2" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname); let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4175,"127.0.0.1");
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const out = "/tmp/phone-audit"; const res = [];
const check = (n, ok, d="") => { res.push(ok); console.log(`${ok?"PASS":"FAIL"} ${n}${d?" — "+d:""}`); };
const browser = await chromium.launch();
async function open(opts) {
  const ctx = await browser.newContext(opts); const { pk } = await installMockExtension(ctx, TYLER);
  await ctx.addInitScript(({ relay, pk }) => {
    if (!localStorage.getItem("buzz-communities")) {
      localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
      localStorage.setItem("buzz-active-community-id", "c1");
    }
  }, { relay: "ws://localhost:3100", pk });
  const page = await ctx.newPage(); const errs = [];
  page.on("pageerror", e => errs.push(String(e)));
  if (process.env.S17_CONSOLE) page.on("console", m => { if (["error","warning"].includes(m.type())) console.log("console."+m.type(), m.text().slice(0,300)); });
  page.on("response", r => { if (/localhost:3100/.test(r.url()) && r.status() >= 400) console.log("HTTP", r.status(), r.url()); });
  await page.goto("http://localhost:4175/", { waitUntil: "domcontentloaded" });
  await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.click('[data-testid="web-unlock-nip07"]');
  await page.waitForSelector('[data-testid="app-top-chrome"]', { timeout: 30000 }).catch(async (e) => {
    await page.screenshot({ path: `${out}/boot-fail-${opts.viewport.width}.png` });
    console.log("BOOT TEXT:", (await page.innerText("body")).slice(0, 600)); throw e; });
  await page.waitForTimeout(4000);
  return { ctx, page, errs };
}
// Phone
const iphone = { ...devices["iPhone 13"], viewport: { width: 390, height: 844 } };
const { ctx, page, errs } = await open(iphone);
check("phone: data-phone set", await page.evaluate(() => document.documentElement.hasAttribute("data-phone")));
check("phone: tab bar visible", await page.isVisible('[data-testid="phone-tabbar"]'));
check("phone: no slide-out sidebar trigger visible", !(await page.isVisible('[data-sidebar="trigger"]')));
check("phone: sidebar not rendered on Inbox", (await page.locator('[data-testid="app-sidebar"]').count()) === 0);
await page.screenshot({ path: `${out}/phone-1-inbox.png` });
await page.tap('[data-testid="phone-tab-channels"]'); await page.waitForTimeout(1500);
check("phone: channels list full screen", await page.isVisible('[data-testid="phone-channels-screen"] [data-testid="app-sidebar"]'));
const sbBox = await page.locator('[data-testid="app-sidebar"]').boundingBox();
check("phone: channels list full width", sbBox && sbBox.width >= 385, JSON.stringify(sbBox));
await page.screenshot({ path: `${out}/phone-2-channels.png` });
console.log("CHANNELS TEXT:", (await page.innerText('[data-testid="phone-channels-screen"]')).replace(/\n+/g," | ").slice(0,500));
const ch = page.locator('[data-testid="phone-channels-screen"] [data-testid="sidebar-channel-content"] >> text=general').first();
const nCh = await ch.count();
if (nCh) { await ch.tap(); await page.waitForTimeout(2500);
  check("phone: channel opened full screen (route)", page.url().includes("/channels/"), page.url());
  check("phone: tab bar hidden in channel", !(await page.isVisible('[data-testid="phone-tabbar"]')));
  check("phone: back button shown", await page.isVisible('[data-testid="phone-back"]'));
  await page.screenshot({ path: `${out}/phone-3-channel.png` });
  await page.tap('[data-testid="phone-back"]'); await page.waitForTimeout(1200);
  check("phone: back returns to channels list", await page.isVisible('[data-testid="phone-channels-screen"]'));
} else check("phone: found a channel row", false);
check("phone: no Agents tab", (await page.locator('[data-testid="phone-tab-agents"]').count()) === 0);
check("phone: channels list has no search box", !(await page.locator('[data-testid="sidebar-pinned-header"]').isVisible().catch(() => false)));
await page.tap('[data-testid="phone-tab-search"]'); await page.waitForTimeout(1500);
check("phone: search screen with focused input", await page.isVisible('[data-testid="phone-search-screen"]') && await page.evaluate(() => document.activeElement?.tagName === "INPUT"));
await page.keyboard.type("general"); await page.waitForTimeout(2000);
check("phone: search results update as you type", await page.locator('[data-testid="search-results"]').isVisible());
await page.screenshot({ path: `${out}/phone-4-search.png` });
await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(1000);
check("phone: me screen", await page.isVisible('[data-testid="phone-me-screen"]'));
await page.tap('[data-testid="phone-me-agents"]'); await page.waitForTimeout(2000);
check("phone: Me > Agents route, Me tab active", page.url().endsWith("/agents") && (await page.getAttribute('[data-testid="phone-tab-me"]', "aria-current")) === "page", page.url());
await page.tap('[data-testid="phone-back"]'); await page.waitForTimeout(800);
check("phone: Agents back returns to Me", await page.isVisible('[data-testid="phone-me-screen"]'));
await page.screenshot({ path: `${out}/phone-5-me.png` });
await page.tap('[data-testid="phone-tab-term"]'); await page.waitForTimeout(3000);
check("phone: buzz-term route", page.url().includes("/buzz-term"), page.url());
await page.screenshot({ path: `${out}/phone-6-term.png` });
await page.goto("http://localhost:4175/#/agents"); await page.waitForTimeout(4000);
check("phone: deep link /agents → Me tab active", (await page.getAttribute('[data-testid="phone-tab-me"]', "aria-current")) === "page");
check("phone: no page errors", errs.length === 0, errs.slice(0,2).join(" | "));
await ctx.close();
for (const [name, vp] of [["ipad", { width: 820, height: 1180 }], ["desktop", { width: 1440, height: 900 }]]) {
  const o = await open({ viewport: vp, hasTouch: name === "ipad" });
  check(`${name}: no data-phone`, !(await o.page.evaluate(() => document.documentElement.hasAttribute("data-phone"))));
  check(`${name}: no tab bar`, (await o.page.locator('[data-testid="phone-tabbar"]').count()) === 0);
  check(`${name}: sidebar visible`, await o.page.isVisible('[data-testid="app-sidebar"]'));
  check(`${name}: sidebar trigger visible`, await o.page.isVisible('[data-sidebar="trigger"]'));
  console.log(name.toUpperCase()+" SIDEBAR:", (await o.page.innerText('[data-testid="app-sidebar"]')).replace(/\n+/g," | ").slice(0,400));
  await o.page.screenshot({ path: `${out}/${name}.png` }); await o.ctx.close();
}
await browser.close(); srv.close();
console.log(res.every(Boolean) ? "ALL PASS" : "SOME FAIL");
