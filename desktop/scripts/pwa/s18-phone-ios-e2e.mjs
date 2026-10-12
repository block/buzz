// Phone iOS polish checks (WebKit, iPhone 14). Safe areas are simulated via --phone-safe-top/bottom overrides
// (WebKit headless reports env(safe-area-inset-*) = 0). Usage: build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 to
// S18_DIST (default /tmp/phone-dist), local relay on :3100.
import fs from "node:fs"; import http from "node:http"; import path from "node:path";
import { chromium, devices, webkit } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S18_DIST || "/tmp/phone-dist";
const out = process.env.S18_OUT || "/Users/jm/Documents/phone-audit-shots";
fs.mkdirSync(out, { recursive: true });
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname); let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4176,"127.0.0.1");
const SK = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const res = []; const check = (n, ok, d="") => { res.push(!!ok); console.log(`${ok?"PASS":"FAIL"} ${n}${d?" — "+d:""}`); };
async function open(engine, opts, safe) {
  const b = await engine.launch(); const ctx = await b.newContext(opts);
  const { pk } = await installMockExtension(ctx, SK);
  await ctx.addInitScript(({ relay, pk, safe }) => {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
    if (safe) document.addEventListener("DOMContentLoaded", () => {
      const s = document.createElement("style"); s.textContent = `:root{--phone-safe-top:${safe[0]}px !important;--phone-safe-bottom:${safe[1]}px !important}`; document.head.appendChild(s); });
  }, { relay: "ws://localhost:3100", pk, safe });
  const page = await ctx.newPage(); const errs = []; page.on("pageerror", e => errs.push(String(e)));
  await page.goto("http://localhost:4176/");
  await page.click('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.waitForSelector('[data-testid="app-top-chrome"]', { timeout: 30000 });
  await page.waitForTimeout(5000);
  return { b, page, errs };
}
const overflowers = (page) => page.evaluate(() => { const W = innerWidth; const o = [];
  for (const el of document.querySelectorAll("body *")) { const r = el.getBoundingClientRect(); const cs = getComputedStyle(el);
    if (r.width > 0 && r.height > 0 && r.right > W + 1 && cs.visibility !== "hidden" && !el.closest("[data-radix-popper-content-wrapper]")) o.push(`${el.tagName.toLowerCase()}.${String(el.className).slice(0,40)}[${el.getAttribute("data-testid")||""}] r=${Math.round(r.right)}`); }
  return o; });
const iphone = { ...devices["iPhone 14"] };
const { b, page, errs } = await open(webkit, iphone, [47, 34]);
await page.tap('[data-testid="phone-tab-channels"]'); await page.waitForTimeout(1000);
await page.locator('[data-testid="phone-channels-screen"] [data-testid="sidebar-channel-content"] >> text=general').first().tap();
await page.waitForTimeout(3000);
await page.screenshot({ path: `${out}/ios-1-channel.png` });
const top = await page.evaluate(() => { const c = document.querySelector('[data-testid="app-top-chrome"]'); return c ? Math.round(c.getBoundingClientRect().top) : -1; });
check("top chrome starts below the simulated safe area (47px)", top >= 47, `top=${top}`);
const ov = await overflowers(page);
check("nothing overflows the right edge in a channel", ov.length === 0, ov.slice(0, 6).join(" | "));
const comp = page.locator('[data-testid="message-composer"], [contenteditable="true"], textarea').last();
const cb = await comp.boundingBox();
const vh = page.viewportSize().height;
check("composer fully on screen above the bottom safe area", cb && cb.y + cb.height <= vh - 34 + 1, JSON.stringify(cb));
check("header actions collapsed into one ⋮ overflow", (await page.locator('[data-testid="phone-header-overflow"]').count()) === 1 && (await page.locator('[data-testid="chat-header"] [data-testid="conversation-info-button"]').count()) === 0);
await page.tap('[data-testid="phone-header-overflow"]'); await page.waitForTimeout(500);
check("overflow menu shows channel actions", await page.locator('[data-testid="phone-header-overflow-menu"] [data-testid="conversation-info-button"]').isVisible());
await page.screenshot({ path: `${out}/ios-3-header-menu.png` });
await page.keyboard.press("Escape"); await page.waitForTimeout(400);
const surf = await page.evaluate(() => { const s = document.querySelector("[data-buzz-content-surface]").getBoundingClientRect(); return [Math.round(s.left), Math.round(s.right)]; });
check("content surface is edge to edge", surf[0] === 0 && surf[1] === 390, JSON.stringify(surf));
const actionsVisible = await page.evaluate(() => [...document.querySelectorAll('[data-testid^="message-action"], [data-message-actions]')].filter(e => { const r = e.getBoundingClientRect(); return r.width && getComputedStyle(e).opacity !== "0" && getComputedStyle(e).display !== "none" && getComputedStyle(e).visibility !== "hidden"; }).length);
check("no persistent message action bars on phone", actionsVisible === 0, `visible=${actionsVisible}`);
const rows = page.locator('[data-testid="message-row"]');
const lastRow = rows.last(); const rb = await lastRow.boundingBox();
await page.touchscreen.tap(rb.x + 200, rb.y + 22); await page.waitForTimeout(400);
check("tapping a message opens its action bar", (await page.locator('[data-phone-actions-open] [data-message-action-rail]').count()) === 1);
await page.screenshot({ path: `${out}/ios-4-message-actions.png` });
await page.touchscreen.tap(200, 100); await page.waitForTimeout(300);
check("tapping elsewhere dismisses it", (await page.locator('[data-phone-actions-open]').count()) === 0);
// keyboard simulation: shrink the layout viewport like iOS does with the keyboard up (visualViewport height)
await comp.tap(); await page.waitForTimeout(300);
await page.setViewportSize({ width: 390, height: 844 - 336 }); await page.waitForTimeout(800);
const cb2 = await comp.boundingBox();
check("composer stays visible when the keyboard shrinks the viewport", cb2 && cb2.y + cb2.height <= 844 - 336 + 1, JSON.stringify(cb2));
await page.screenshot({ path: `${out}/ios-2-keyboard.png` });
const list = page.locator('[data-testid="message-timeline"], [data-testid="channel-timeline"], [role="log"]').first();
const before = await page.evaluate(() => !!document.activeElement?.isContentEditable || /TEXTAREA|INPUT/.test(document.activeElement?.tagName || ""));
check("composer focused before the list tap", before);
const rb2 = await page.locator('[data-testid="message-row"]').last().boundingBox();
await page.touchscreen.tap(rb2.x + 200, rb2.y + 22); await page.waitForTimeout(300);
check("tapping the message list dismisses the keyboard (composer blurred)", await page.evaluate(() => !document.activeElement || !(document.activeElement.isContentEditable || /TEXTAREA|INPUT/.test(document.activeElement.tagName))));
await page.setViewportSize({ width: 390, height: 844 });
await page.tap('[data-testid="phone-back"]').catch(() => {}); await page.waitForTimeout(800);
const tb = await page.locator('[data-testid="phone-tabbar"]').boundingBox();
check("tab bar floats: inset from edges and above the home indicator", tb && tb.x >= 12 && tb.x + tb.width <= 378 && tb.y + tb.height <= 844 - 34 - 6, JSON.stringify(tb));
const radius = await page.locator('[data-testid="phone-tabbar"]').evaluate(e => parseFloat(getComputedStyle(e).borderRadius));
check("tab bar is a capsule", radius >= 28, String(radius));
await page.screenshot({ path: `${out}/ios-5-floating-tabbar.png` });
await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(600);
await page.locator('text=Settings').first().tap(); await page.waitForTimeout(1500);
const navCount = await page.locator('[data-testid="phone-settings-list"] [data-testid^="settings-nav-"]').count();
check("phone Settings opens the full section list", navCount >= 4, `sections=${navCount}`);
await page.screenshot({ path: `${out}/ios-6-settings-list.png` });
await page.locator('[data-testid="phone-settings-list"] [data-testid^="settings-nav-"]').nth(1).tap(); await page.waitForTimeout(1200);
check("a section opens full screen with a back button", await page.locator('[data-testid="phone-settings-back"]').isVisible());
await page.screenshot({ path: `${out}/ios-7-settings-section.png` });
await page.tap('[data-testid="phone-settings-back"]'); await page.waitForTimeout(500);
check("back returns to the list", await page.locator('[data-testid="phone-settings-list"]').isVisible());
await page.locator('[data-testid="settings-nav-profile"]').tap().catch(() => {}); await page.waitForTimeout(1200);
await page.screenshot({ path: `${out}/ios-8-settings-profile.png` });
const ov2 = await overflowers(page); check("settings section: nothing overflows", ov2.length === 0, ov2.slice(0, 4).join(" | "));
check("phone: no page errors", errs.filter(e => !/ResizeObserver/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
await b.close();
for (const [name, vp] of [["ipad", { width: 820, height: 1180 }], ["desktop", { width: 1440, height: 900 }]]) {
  const o = await open(chromium, { viewport: vp, hasTouch: name === "ipad" }, null);
  check(`${name}: no data-phone / tab bar`, !(await o.page.evaluate(() => document.documentElement.hasAttribute("data-phone"))) && (await o.page.locator('[data-testid="phone-tabbar"]').count()) === 0);
  await o.page.locator('[data-testid="sidebar-channel-content"] >> text=general').first().click(); await o.page.waitForTimeout(2500);
  await o.page.screenshot({ path: `${out}/ios-${name}-channel.png` }); await o.b.close();
}
srv.close(); console.log(res.every(Boolean) ? "ALL PASS" : "SOME FAIL");
