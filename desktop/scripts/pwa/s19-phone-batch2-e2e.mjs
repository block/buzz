// Phone batch 2 (WebKit, iPhone 14): standalone tab bar position, avatar Me tab, tab order, Channels default,
// channels list without profile bar / menu dock, Me rows + counts, settings horizontal overflow at 390 and 375.
// Build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 to S19_DIST (default /tmp/phone-dist); local relay on :3100.
import fs from "node:fs"; import http from "node:http"; import path from "node:path";
import { chromium, devices, webkit } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S19_DIST || "/tmp/phone-dist";
const out = process.env.S19_OUT || "/Users/jm/Documents/phone-audit-shots";
fs.mkdirSync(out, { recursive: true });
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname); let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4178,"127.0.0.1");
const SK = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const res = []; const check = (n, ok, d="") => { res.push(!!ok); console.log(`${ok?"PASS":"FAIL"} ${n}${d?" — "+d:""}`); };
async function open(engine, opts, { safe = null, standalone = false, prefs = null } = {}) {
  const b = await engine.launch(); const ctx = await b.newContext(opts);
  const { pk } = await installMockExtension(ctx, SK);
  await ctx.addInitScript(({ relay, pk, safe, standalone, prefs }) => {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
    if (prefs) for (const [k, v] of Object.entries(prefs)) localStorage.setItem(k, v);
    if (standalone) {
      Object.defineProperty(navigator, "standalone", { value: true });
      const mm = window.matchMedia.bind(window);
      window.matchMedia = (q) => /display-mode:\s*standalone/.test(q) ? { matches: true, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} } : mm(q);
    }
    if (safe) document.addEventListener("DOMContentLoaded", () => {
      const s = document.createElement("style"); s.textContent = `:root{--phone-safe-top:${safe[0]}px !important;--phone-safe-bottom:${safe[1]}px !important}`; document.head.appendChild(s); });
  }, { relay: "ws://localhost:3100", pk, safe, standalone, prefs });
  const page = await ctx.newPage(); const errs = []; page.on("pageerror", e => errs.push(String(e)));
  await page.goto("http://localhost:4178/");
  await page.click('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.waitForSelector('[data-testid="app-top-chrome"], [data-testid="phone-tabbar"]', { timeout: 30000 });
  await page.waitForTimeout(5000);
  return { b, page, errs };
}
const overflowers = (page) => page.evaluate(() => { const W = document.documentElement.clientWidth; const o = [];
  for (const el of document.querySelectorAll("body *")) { const r = el.getBoundingClientRect(); const cs = getComputedStyle(el);
    if (r.width > 0 && r.height > 0 && (r.right > W + 1 || r.left < -1) && cs.visibility !== "hidden" && !el.closest("[data-radix-popper-content-wrapper],[aria-hidden=true]")) {
      // ignore content clipped by a scroll/overflow-hidden ancestor that itself fits
      let a = el.parentElement, clipped = false;
      while (a && a !== document.body) { const s = getComputedStyle(a); if (/(hidden|auto|scroll|clip)/.test(s.overflowX)) { const ar = a.getBoundingClientRect(); if (ar.right <= W + 1 && ar.left >= -1) { clipped = true; break; } } a = a.parentElement; }
      if (!clipped) o.push(`${el.tagName.toLowerCase()}[${el.getAttribute("data-testid")||String(el.className).slice(0,30)}] l=${Math.round(r.left)} r=${Math.round(r.right)}`); } }
  return o; });

// 1-5: standalone PWA, notch 47 / home indicator 34, full 844 height
{
  // Installed PWA has no Safari chrome: the full 390x844 screen.
  const { b, page, errs } = await open(webkit, { ...devices["iPhone 14"], viewport: { width: 390, height: 844 }, screen: { width: 390, height: 844 } }, { safe: [47, 34], standalone: true });
  check("opens on the Channels tab", (await page.getAttribute('[data-testid="phone-tab-channels"]', "aria-current")) === "page" && await page.isVisible('[data-testid="phone-channels-screen"]'));
  const tb = await page.locator('[data-testid="phone-tabbar"]').boundingBox();
  const gap = 844 - (tb.y + tb.height);
  check("standalone: pill floats with a clear gap above the home indicator (≥ inset + 6px)", gap >= 40 && gap <= 48, `bottom gap=${Math.round(gap)}`);
  const bg = await page.evaluate(() => [getComputedStyle(document.documentElement).backgroundColor, getComputedStyle(document.body).backgroundColor]);
  check("standalone: page background painted (no grey strip under the shell)", bg.every((c) => c && c !== "rgba(0, 0, 0, 0)"), bg.join(" / "));
  const vh = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--phone-vh"));
  check("standalone: shell uses the full height", parseInt(vh) >= 844, vh);
  const order = await page.$$eval('[data-testid="phone-tabbar"] > button', (bs) => bs.map((x) => x.getAttribute("data-testid").replace("phone-tab-", "")));
  check("tab order: me, inbox, channels, term, search", order.join(",") === "me,inbox,channels,term,search", order.join(","));
  const meTab = page.locator('[data-testid="phone-tab-me"]');
  check("Me tab is the avatar with an online dot, no icon/label", (await meTab.locator('[data-testid="phone-tab-me-avatar"]').count()) === 1 && (await meTab.locator('[data-testid="phone-tab-me-online"]').count()) === 1 && (await meTab.innerText()).replace(/\s/g, "").length <= 2);
  check("channels list: no bottom profile bar", !(await page.locator('[data-testid="phone-channels-screen"] [data-testid="sidebar-profile-card"]').isVisible().catch(() => false)));
  await page.evaluate(() => { const s = document.querySelector('[data-testid="phone-channels-screen"] [data-sidebar="content"]'); if (s) s.scrollTop = 400; s?.dispatchEvent(new Event("scroll")); });
  await page.waitForTimeout(600);
  check("channels list: no docked menu row while scrolling", !(await page.locator('[data-testid="phone-channels-screen"] [data-testid="sidebar-menu-dock"]').isVisible().catch(() => false)));
  await page.screenshot({ path: `${out}/b2-1-standalone-channels.png` });
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2500);
  const st = await page.evaluate(() => ({ focusedEditable: !!document.activeElement?.isContentEditable || /INPUT|TEXTAREA/.test(document.activeElement?.tagName || ""), scrollY: window.scrollY, root: document.getElementById("root")?.scrollTop ?? 0, html: document.documentElement.scrollTop }));
  check("opening a channel does not raise the keyboard (composer not auto-focused)", !st.focusedEditable, JSON.stringify(st));
  // simulate iOS scrolling the document to a focused field, then blurring: it must snap back
  await page.evaluate(() => { document.documentElement.scrollTop = 300; document.getElementById("root").scrollTop = 300; window.scrollTo(0, 300); document.body.focus(); document.dispatchEvent(new FocusEvent("focusout")); });
  await page.waitForTimeout(400);
  const st2 = await page.evaluate(() => ({ y: window.scrollY, root: document.getElementById("root")?.scrollTop ?? 0, html: document.documentElement.scrollTop, msgs: document.querySelectorAll('[data-testid="message-row"]').length, firstVisible: [...document.querySelectorAll('[data-testid="message-row"]')].some((r) => { const b = r.getBoundingClientRect(); return b.top >= 0 && b.bottom <= innerHeight; }) }));
  check("stale document scroll resets so messages are in view", st2.y === 0 && st2.root === 0 && st2.html === 0 && st2.firstVisible, JSON.stringify(st2));
  await page.screenshot({ path: `${out}/b3-channel-open.png` });
  await page.tap('[data-testid="phone-back"]').catch(() => {}); await page.waitForTimeout(800);
  await meTab.tap(); await page.waitForTimeout(800);
  const rows = await page.$$eval('[data-testid="phone-me-screen"] [data-testid^="phone-me-"]:not([data-testid$="-count"])', (bs) => bs.map((x) => x.getAttribute("data-testid").replace("phone-me-", "")).filter(x => x !== "screen"));
  check("Me rows in sidebar order, Settings last, Workflows hidden by default", rows.join(",") === "projects,agents,bots,browsers,settings", rows.join(","));
  check("Me counts hidden when the Appearance toggles are off (default)", (await page.locator('[data-testid$="-count"][data-testid^="phone-me-"]').count()) === 0);
  await page.screenshot({ path: `${out}/b2-2-me.png` });
  // 9: external links open outside the app; in-app links stay
  const appUrl = page.url();
  await page.evaluate(() => { const a = document.createElement("a"); a.href = "https://example.com/ext"; a.textContent = "ext"; a.id = "s19-ext"; a.style.cssText = "position:fixed;top:100px;left:100px;z-index:99999;padding:10px;background:red"; document.body.appendChild(a); });
  const [popup] = await Promise.all([page.context().waitForEvent("page", { timeout: 5000 }).catch(() => null), page.click("#s19-ext")]);
  await page.waitForTimeout(500);
  check("external link opens a new browsing context (not the app window)", !!popup && page.url() === appUrl, `popup=${!!popup} url=${page.url()}`);
  check("external link hardened with target=_blank rel=noopener noreferrer", await page.evaluate(() => { const a = document.getElementById("s19-ext"); return a.target === "_blank" && /noopener/.test(a.rel) && /noreferrer/.test(a.rel); }));
  if (popup) await popup.close();
  check("no page errors", errs.filter(e => !/ResizeObserver/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
// 4 + 6 with toggles on and Workflows enabled
{
  // Appearance per-item count toggles + Experimental "workflows" override (key is versioned by the manifest).
  const prefs = { "buzz.appearance.sidebarMenuCountPrefs": JSON.stringify({ inbox: true, browsers: true, agents: true, bots: true }) };
  for (let v = 0; v <= 30; v++) prefs[`buzz-feature-overrides-v${v}`] = JSON.stringify({ workflows: true });
  const { b, page } = await open(webkit, devices["iPhone 14"], { safe: [47, 34], prefs });
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(1500);
  const rows = await page.$$eval('[data-testid="phone-me-screen"] [data-testid^="phone-me-"]:not([data-testid$="-count"])', (bs) => bs.map((x) => x.getAttribute("data-testid").replace("phone-me-", "")).filter(x => x !== "screen"));
  check("Workflows row shown when enabled (between Browsers and Settings)", rows.join(",") === "projects,agents,bots,browsers,workflows,settings", rows.join(","));
  const counts = await page.$$eval('[data-testid$="-count"][data-testid^="phone-me-"]', (bs) => bs.map((x) => x.getAttribute("data-testid")));
  check("Agents/Bots/Browsers counts shown when toggles are on", ["agents","bots","browsers"].every(k => counts.includes(`phone-me-${k}-count`)), counts.join(","));
  await page.screenshot({ path: `${out}/b2-3-me-counts.png` });
  await b.close();
}
// 10: phone tab switcher (long-press "Open in new tab", floating count button, combined list, open/close/dismiss)
{
  const { b, page } = await open(webkit, { ...devices["iPhone 14"], viewport: { width: 390, height: 844 } }, { safe: [47, 34] });
  check("tab switcher hidden with zero tabs", (await page.locator('[data-testid="phone-tab-switcher"]').count()) === 0);
  for (const name of ["general", "engineering"]) {
    await page.locator(`[data-testid="phone-channels-screen"] [data-testid="channel-${name}"]`).first().dispatchEvent("contextmenu", { clientX: 120, clientY: 300 });
    await page.waitForTimeout(500);
    const item = page.locator('[data-testid="add-surface-tab"]');
    if (name === "general") check("long-press offers 'Open in new tab'", (await item.innerText().catch(() => "")).includes("Open in new tab"));
    await item.click().catch(() => {}); await page.waitForTimeout(800);
    await page.tap('[data-testid="phone-back"]').catch(() => {}); await page.waitForTimeout(800);
  }
  const btn = page.locator('[data-testid="phone-tab-switcher"]');
  check("floating button shows the tab count", (await btn.innerText().catch(() => "")).trim() === "2", await btn.innerText().catch(() => "none"));
  const bb = await btn.boundingBox();
  check("button sits top-left under the safe area", bb && bb.x <= 12 && bb.y >= 47 && bb.y <= 60, JSON.stringify(bb));
  const padTop = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector('[data-testid="phone-channels-screen"]')?.closest(".phone-fullscreen") || document.querySelector(".phone-fullscreen")).paddingTop));
  check("no back button: switcher floats over content (no extra header space)", padTop <= 48, `padding-top=${padTop}`);
  await page.screenshot({ path: `${out}/b3-switcher-overlay.png` });
  await btn.tap(); await page.waitForTimeout(400);
  const list = page.locator('[data-testid="phone-tab-switcher-list"]');
  const rowText = await list.innerText().catch(() => "");
  check("list shows all tabs with type labels", /general/.test(rowText) && /engineering/.test(rowText) && /channel/i.test(rowText), rowText.replace(/\n/g, " | "));
  await page.screenshot({ path: `${out}/b2-4-tab-switcher.png` });
  await list.locator('[data-testid="phone-tab-row-open"]', { hasText: "engineering" }).tap(); await page.waitForTimeout(1500);
  check("tapping a row navigates there", /channels\//.test(page.url()) && await page.isVisible('[data-testid="chat-header"]'), page.url());
  await page.screenshot({ path: `${out}/b2-5-tab-open.png` });
  const back = await page.locator('[data-testid="phone-back"]').boundingBox().catch(() => null);
  check("back button shifted right of the floating button", back && back.x >= 46, JSON.stringify(back));
  await btn.tap(); await page.waitForTimeout(300);
  check("current tab highlighted", (await list.locator('[data-active="true"]').innerText().catch(() => "")).includes("engineering"));
  await page.keyboard.press("Escape"); await page.waitForTimeout(300);
  check("Escape dismisses the list", !(await list.isVisible().catch(() => false)));
  await btn.tap(); await page.waitForTimeout(300);
  await page.touchscreen.tap(300, 380); await page.waitForTimeout(300);
  check("tap outside dismisses the list", !(await list.isVisible().catch(() => false)));
  await btn.tap(); await page.waitForTimeout(300);
  await list.locator('[data-active="true"] [data-testid="phone-tab-row-close"]').tap(); await page.waitForTimeout(1200);
  check("closing the active tab leaves 1 tab and moves to it", (await btn.innerText().catch(() => "")).trim() === "1" && (await list.locator('[data-active="true"]').innerText().catch(() => "")).includes("general"), (await list.innerText().catch(() => "")).replace(/\n/g, " | "));
  await b.close();
}
// 7: every settings section at 390 and 375
for (const w of [390, 375]) {
  const { b, page } = await open(webkit, { ...devices["iPhone 14"], viewport: { width: w, height: 812 } }, { safe: [47, 34] });
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-me-settings"]'); await page.waitForTimeout(1500);
  const ids = await page.$$eval('[data-testid="phone-settings-list"] [data-testid^="settings-nav-"]', (bs) => bs.map((x) => x.getAttribute("data-testid")));
  const bad = [];
  for (const id of ids) {
    await page.tap(`[data-testid="${id}"]`); await page.waitForTimeout(1200);
    const sec = id.replace("settings-nav-", "");
    const ov = await overflowers(page);
    if (ov.length) bad.push(`${sec}: ${ov.slice(0, 3).join(" ")}`);
    if (sec === "appearance") {
      // The accent picker only shows for non-Buzz themes: pick another theme style first.
      await page.tap('[data-testid="theme-style-trigger"]').catch(() => {}); await page.waitForTimeout(500);
      const opts = page.locator('[data-testid="theme-style-options"] button');
      const n = await opts.count();
      for (let i = 0; i < n && !(await page.locator('[data-testid="accent-color-options"]').count()); i++) { await opts.nth(i).tap().catch(() => {}); await page.waitForTimeout(500); }
      const last = page.locator('[data-testid="accent-color-options"] button').last();
      await last.tap().catch(() => {}); await page.waitForTimeout(400);
      const ap = await page.evaluate(() => { const W = document.documentElement.clientWidth; const sw = [...document.querySelectorAll('[data-testid="accent-color-options"] button')].map((b) => b.getBoundingClientRect()); const sc = document.querySelector('[data-testid="settings-content-scroll"]'); const h = [...document.querySelectorAll('[data-testid="settings-panel-appearance"] p')][0]?.getBoundingClientRect(); return { allIn: sw.length > 0 && sw.every((r) => r.left >= 0 && r.right <= W + 0.5), n: sw.length, scrollLeft: sc?.scrollLeft ?? -1, titleLeft: h ? Math.round(h.left) : -1 }; });
      await page.screenshot({ path: `${out}/b3-appearance-${w}.png` });
      check(`appearance @${w}: accent swatches fit and the page is not shifted`, ap.allIn && ap.scrollLeft === 0 && ap.titleLeft >= 8, JSON.stringify(ap));
    }
    if (sec === "appearance" || sec === "profile") await page.screenshot({ path: `${out}/b2-settings-${sec}-${w}.png`, fullPage: false });
    await page.tap('[data-testid="phone-settings-back"]'); await page.waitForTimeout(400);
  }
  check(`settings @${w}px: no element exceeds the viewport width (${ids.length} sections)`, bad.length === 0 && ids.length > 3, bad.join(" || "));
  await b.close();
}
// iPad/desktop unchanged
for (const [name, vp] of [["ipad", { width: 820, height: 1180 }], ["desktop", { width: 1440, height: 900 }]]) {
  const o = await open(chromium, { viewport: vp, hasTouch: name === "ipad" });
  check(`${name}: no data-phone / tab bar, profile card present`, !(await o.page.evaluate(() => document.documentElement.hasAttribute("data-phone"))) && (await o.page.locator('[data-testid="phone-tabbar"]').count()) === 0 && await o.page.isVisible('[data-testid="sidebar-profile-card"]') && (await o.page.locator('[data-testid="phone-tab-switcher"]').count()) === 0);
  await o.page.screenshot({ path: `${out}/b2-${name}.png` }); await o.b.close();
}
srv.close(); console.log(res.every(Boolean) ? "ALL PASS" : "SOME FAIL");
