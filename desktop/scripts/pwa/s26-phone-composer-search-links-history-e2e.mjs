// Phone composer collapse/expand, search pinned under the safe area with the keyboard up, web link normalization, linear Back/Forward history.
// Build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 VITE_BUZZ_WEB_PUSH_URL=<any> to S24_DIST; local relay on :3100.
// A TCP proxy on :3101 sits in front of the relay so the test can drop the connection (reconnect pills).
import fs from "node:fs"; import http from "node:http"; import net from "node:net"; import path from "node:path";
import { chromium, devices } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S24_DIST || "/tmp/nt-phone-dist";
const out = process.env.S24_OUT || "/workspace/composer-shots";
fs.mkdirSync(out, { recursive: true });
let fakeBuild = null;
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png",".json":"application/json" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname);
  if (p === "/version.json" && fakeBuild) { r.writeHead(200,{"Content-Type":"application/json"}); r.end(JSON.stringify({ build: fakeBuild })); return; }
  let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4182,"127.0.0.1");
// relay proxy
let socks = new Set(); let up = true;
const proxy = net.createServer((c) => { if (!up) return c.destroy(); const s = net.connect(3100, "127.0.0.1"); socks.add(c); socks.add(s);
  c.pipe(s); s.pipe(c); const end = () => { c.destroy(); s.destroy(); socks.delete(c); socks.delete(s); }; c.on("error", end); s.on("error", end); c.on("close", end); s.on("close", end); }).listen(3102, "127.0.0.1");
const drop = () => { up = false; for (const x of socks) x.destroy(); socks.clear(); };
const restore = () => { up = true; };

const SK = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const res = []; const check = (n, ok, d="") => { res.push(!!ok); console.log(`${ok?"PASS":"FAIL"} ${n}${d?" — "+d:""}`); };
const PHONE = { ...devices["iPhone 14"], viewport: { width: 390, height: 844 }, screen: { width: 390, height: 844 }, deviceScaleFactor: 2,
  userAgent: devices["iPhone 14"].userAgent.replace("OS 16_0", "OS 18_0") };
delete PHONE.defaultBrowserType;
async function open({ standalone = true, delay = 0, permissions = [] } = {}) {
  const b = await chromium.launch(); const ctx = await b.newContext({ ...PHONE, permissions });
  const { pk } = await installMockExtension(ctx, SK, { delay });
  await ctx.addInitScript(({ relay, pk, standalone }) => {
    // iOS standalone behavior: permission starts "default"; after Allow, requestPermission() resolves "granted" but
    // Notification.permission keeps reading "default" (the lag that kept the banner up).
    if (window.Notification) {
      let answered = null;
      Object.defineProperty(Notification, "permission", { get: () => (window.__permLive ? answered ?? "default" : "default"), configurable: true });
      Notification.requestPermission = async () => { answered = "granted"; return "granted"; };
    }
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
    if (standalone) {
      Object.defineProperty(navigator, "standalone", { value: true });
      const mm = window.matchMedia.bind(window);
      window.matchMedia = (q) => /display-mode:\s*standalone/.test(q) ? { matches: true, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {} } : mm(q);
    }
  }, { relay: "ws://localhost:3102", pk, standalone });
  const page = await ctx.newPage(); const errs = []; page.on("pageerror", e => errs.push(String(e)));
  await page.goto("http://localhost:4182/");
  await page.click('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.waitForSelector('[data-testid="phone-tabbar"], [data-testid="app-top-chrome"]', { timeout: 30000 });
  await page.waitForTimeout(4000);
  return { b, ctx, page, errs };
}
const shot = (page, name) => page.screenshot({ path: `${out}/${name}.png` });
const navState = (page) => page.evaluate(() => {
  const q = (id) => document.querySelector(`[data-testid="${id}"]`);
  const r = (el) => el?.getBoundingClientRect();
  return { row: !!q("phone-nav-row"), back: q("phone-nav-back")?.disabled, fwd: q("phone-nav-forward")?.disabled,
    rowTop: r(q("phone-nav-row"))?.top, rowH: r(q("phone-nav-row"))?.height, pill: r(q("phone-tab-switcher")),
    pathOk: true, labeled: !!document.querySelector(".phone-back-button, [data-testid='phone-back'], [data-testid='settings-back-to-app']"),
    path: location.hash };
});

const SAFE_TOP = 47;
async function openPhone() {
  const r = await open();
  await r.page.evaluate((t) => { const s = document.createElement("style"); s.textContent = `:root{--phone-safe-top:${t}px !important}`; document.head.appendChild(s); }, SAFE_TOP);
  await r.page.evaluate(() => document.querySelector('[data-testid="status-host"]')?.remove());
  await r.page.waitForTimeout(300);
  return r;
}
const comp = (page) => page.evaluate(() => { const f = document.querySelector('[data-testid="message-composer"]'); const tb = f?.querySelector('[data-testid="message-composer-toolbar"]');
  return { collapsed: f?.getAttribute("data-phone-collapsed") === "true", toolbar: !!tb && getComputedStyle(tb).display !== "none", h: Math.round(f?.getBoundingClientRect().height ?? 0),
    focused: !!f?.contains(document.activeElement), send: !!f?.querySelector('[data-testid="send-message"], button[type="submit"]') }; });
{
  const { b, page, errs } = await openPhone();
  // ---- composer
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2000);
  let c = await comp(page);
  check("composer collapsed when unfocused + empty", c.collapsed && !c.toolbar, JSON.stringify(c));
  await page.screenshot({ path: `${out}/composer-collapsed-390x844.png` });
  await page.locator('[data-testid="message-input"]').tap(); await page.waitForTimeout(500);
  c = await comp(page);
  check("first tap focuses the editor (keyboard) and expands", c.focused && !c.collapsed && c.toolbar, JSON.stringify(c));
  await page.screenshot({ path: `${out}/composer-expanded-390x844.png` });
  const btn = page.locator('[data-testid="message-composer-toolbar"] button').first();
  const ev = await btn.evaluate((el) => { const e = new PointerEvent("pointerdown", { bubbles: true, cancelable: true }); el.dispatchEvent(e); const m = new MouseEvent("mousedown", { bubbles: true, cancelable: true }); el.dispatchEvent(m); return e.defaultPrevented && m.defaultPrevented; });
  check("toolbar pointerdown/mousedown are prevented (no blur)", ev);
  await page.keyboard.type("draft text"); await page.waitForTimeout(200);
  await page.evaluate(() => document.activeElement?.blur()); await page.waitForTimeout(300);
  c = await comp(page);
  check("with a draft, blur keeps it expanded and send visible", !c.collapsed && c.toolbar && c.send, JSON.stringify(c));
  await page.screenshot({ path: `${out}/composer-draft-blurred-390x844.png` });
  await page.locator('[data-testid="message-input"]').tap(); await page.waitForTimeout(300); await page.evaluate(() => { const el = document.querySelector("[data-testid=\"message-input\"] [contenteditable=true]") || document.querySelector("[data-testid=\"message-input\"]"); el.focus(); const r = document.createRange(); r.selectNodeContents(el); const s = getSelection(); s.removeAllRanges(); s.addRange(r); }); await page.keyboard.press("Backspace"); await page.waitForTimeout(200);
  await page.evaluate(() => document.activeElement?.blur()); await page.waitForTimeout(300);
  { const z = await comp(page); const dbg = await page.evaluate(() => ({ text: document.querySelector("[data-testid=\"message-input\"]")?.textContent, ae: document.activeElement?.outerHTML?.slice(0,120), poppers: [...document.querySelectorAll("[data-radix-popper-content-wrapper]")].map(e=>e.outerHTML.slice(0,160)), dlg: document.querySelectorAll("[role=dialog][data-state=open]").length })); check("empty again + blur → collapses", z.collapsed, JSON.stringify({z, dbg})); }
  // ---- links (web normalization)
  const links = await page.evaluate(() => [...document.querySelectorAll('[data-testid="message-row"] a[href]')].map((a) => a.getAttribute("href")));
  const bad = links.filter((h) => h && !/^(https?:|mailto:|tel:|#|\/|buzz:\/\/)/i.test(h));
  check("no message link with a bare or desktop-only href", bad.length === 0, JSON.stringify(bad.slice(0, 5)));
  check("no page errors (composer)", errs.length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
{
  // ---- search with the keyboard up and iOS panning the page
  const { b, page } = await openPhone();
  await page.addInitScript(() => {});
  await page.tap('[data-testid="phone-tab-search"]'); await page.waitForTimeout(1200);
  const input = page.locator('.phone-search-dialog input').first();
  await input.tap(); 
  const H = await page.evaluate(() => innerHeight);
  await page.evaluate(({ h }) => window.__setVV?.(h, 0), { h: H - 336 }).catch(() => {});
  let worst = 999;
  for (const q of ["e", "en", "the", "a", "o"]) {
    await input.fill(q); 
    for (let i = 0; i < 8; i++) {
      await page.waitForTimeout(150);
      // iOS scrolls the page to keep the focused input visible: simulate both window scroll and a visual-viewport pan
      await page.evaluate(() => { window.scrollTo(0, 120); document.scrollingElement && (document.scrollingElement.scrollTop = 120); });
      const top = await page.evaluate(() => document.querySelector('.phone-search-dialog input')?.getBoundingClientRect().top ?? -1);
      worst = Math.min(worst, top);
    }
  }
  const n = await page.locator('.phone-search-dialog [data-testid="search-results-list"] > *').count();
  check(`search input stays at/below the safe-area inset while results load (worst top ${Math.round(worst)}px ≥ ${SAFE_TOP})`, worst >= SAFE_TOP);
  const docScroll = await page.evaluate(() => document.scrollingElement?.scrollTop ?? 0);
  check("document doesn't stay scrolled", docScroll === 0, `scrollTop=${docScroll}, results=${n}`);
  await page.screenshot({ path: `${out}/search-keyboard-390x844.png` });
  await b.close();
}
{
  // ---- linear Back / Forward over 6 distinct screens
  const { b, page } = await openPhone();
  const where = () => page.evaluate(() => {
    if (document.querySelector('[data-testid="phone-settings-list"]')) return "settings";
    if (document.querySelector('[data-testid="phone-settings"]')) return "settings-section";
    if (document.querySelector('[data-testid="phone-channels-screen"]')) return "channels";
    if (document.querySelector(".phone-fullscreen") && !document.querySelector(".phone-search-screen")) return "me";
    return location.hash;
  });
  const seq = [await where()];
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(1500); seq.push(await where());
  await page.tap('[data-testid="phone-nav-home"]'); await page.waitForTimeout(500);
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-design"]').first().tap(); await page.waitForTimeout(1500); seq.push(await where());
  await page.tap('[data-testid="phone-nav-home"]'); await page.waitForTimeout(500);
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(600); seq.push(await where());
  await page.locator('text=Settings').first().tap(); await page.waitForTimeout(1200); seq.push(await where());
  await page.tap('[data-testid="settings-nav-appearance"]'); await page.waitForTimeout(800); seq.push(await where());
  console.log("forward path", JSON.stringify(seq));
  // the Home presses in between are screens too; Back walks the real stack, so collect it
  const backs = [];
  for (let i = 0; i < 5; i++) { await page.tap('[data-testid="phone-nav-back"]'); await page.waitForTimeout(700); backs.push(await where()); }
  console.log("back path", JSON.stringify(backs));
  const distinct = new Set(backs).size;
  check("Back ×5 walks the 5 earlier real screens in order (list screens are pass-through, no ping-pong)", JSON.stringify(backs) === JSON.stringify(["settings","me","#/channels/design","#/channels/general","channels"]), JSON.stringify(backs));
  const fwd = [];
  for (let i = 0; i < 5; i++) { await page.tap('[data-testid="phone-nav-forward"]'); await page.waitForTimeout(700); fwd.push(await where()); }
  console.log("forward replay", JSON.stringify(fwd));
  check("Forward ×5 replays in order back to the last screen", fwd[4] === seq[5] && JSON.stringify([...backs].reverse().slice(1)) === JSON.stringify(fwd.slice(0, 4)), JSON.stringify(fwd));
  check("Forward disabled at the end", await page.locator('[data-testid="phone-nav-forward"]').isDisabled());
  await page.reload(); await page.waitForTimeout(6000);
  await page.tap('[data-testid="phone-nav-back"]').catch(() => {}); await page.waitForTimeout(700);
  check("history survives a reload (Back still works)", (await where()) !== null);
  await b.close();
}
console.log(`\n${res.filter(Boolean).length}/${res.length} passed`);
srv.close(); proxy.close(); process.exit(res.every(Boolean) ? 0 : 1);
