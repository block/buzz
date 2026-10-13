// Phone nav row (Back / Forward / Home), Settings back → Me, shared status pills + timing policy, push banner hide on Allow.
// Build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 VITE_BUZZ_WEB_PUSH_URL=<any> to S24_DIST; local relay on :3100.
// A TCP proxy on :3101 sits in front of the relay so the test can drop the connection (reconnect pills).
import fs from "node:fs"; import http from "node:http"; import net from "node:net"; import path from "node:path";
import { chromium, devices } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S24_DIST || "/tmp/nt-phone-dist";
const out = process.env.S24_OUT || "/workspace/nav-toast-shots";
fs.mkdirSync(out, { recursive: true });
let fakeBuild = null;
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png",".json":"application/json" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname);
  if (p === "/version.json" && fakeBuild) { r.writeHead(200,{"Content-Type":"application/json"}); r.end(JSON.stringify({ build: fakeBuild })); return; }
  let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4181,"127.0.0.1");
// relay proxy
let socks = new Set(); let up = true;
const proxy = net.createServer((c) => { if (!up) return c.destroy(); const s = net.connect(3100, "127.0.0.1"); socks.add(c); socks.add(s);
  c.pipe(s); s.pipe(c); const end = () => { c.destroy(); s.destroy(); socks.delete(c); socks.delete(s); }; c.on("error", end); s.on("error", end); c.on("close", end); s.on("close", end); }).listen(3101, "127.0.0.1");
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
  }, { relay: "ws://localhost:3101", pk, standalone });
  const page = await ctx.newPage(); const errs = []; page.on("pageerror", e => errs.push(String(e)));
  await page.goto("http://localhost:4181/");
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

{
  const { b, page, errs } = await open();
  // the banner is in the shared host on first load (needs-tap) — shoot it, then hide it for the nav shots
  await page.waitForTimeout(1000);
  const bannerShown = await page.locator('[data-testid="web-push-banner"]').count();
  check("push banner shows before Allow (needs-tap)", bannerShown > 0);
  if (bannerShown) await shot(page, "popup-notifications-banner");
  await page.evaluate(() => document.querySelector('[data-testid="status-host"]')?.style.setProperty("visibility","hidden"));

  // channels list (home)
  let s = await navState(page);
  check("nav row on Channels list", s.row);
  check("no labeled back controls left", !s.labeled);
  await shot(page, "nav-channels-list");
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2500);
  s = await navState(page);
  check("nav row in a channel", s.row && s.back === false, JSON.stringify(s));
  check("Forward disabled with nothing ahead", s.fwd === true);
  check("tab pill sits in the nav row (same band)", !s.pill || Math.abs((s.pill.top + s.pill.height / 2) - (s.rowTop + s.rowH / 2)) <= 2, JSON.stringify(s.pill));
  await shot(page, "nav-channel");
  // Back returns to the Channels list; Forward goes back into the channel
  await page.tap('[data-testid="phone-nav-back"]'); await page.waitForTimeout(800);
  check("Back from channel → Channels list", await page.locator('[data-testid="phone-channels-screen"]').count() > 0);
  s = await navState(page);
  check("Forward enabled after Back", s.fwd === false);
  await page.tap('[data-testid="phone-nav-forward"]'); await page.waitForTimeout(800);
  check("Forward → channel again", (await navState(page)).path.startsWith("#/channels/") && await page.locator('[data-testid="phone-channels-screen"]').count() === 0);
  // Me tab → Settings → section, then back out
  await page.tap('[data-testid="phone-nav-home"]'); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(800);
  await shot(page, "nav-me");
  await page.locator('text=Settings').first().tap(); await page.waitForTimeout(1500);
  s = await navState(page);
  check("nav row in Settings", s.row && s.back === false, JSON.stringify(s));
  check("tab pill not shown in Settings", !s.pill);
  await shot(page, "nav-settings");
  await page.tap('[data-testid="settings-nav-appearance"]'); await page.waitForTimeout(1000);
  await shot(page, "nav-settings-section");
  await page.tap('[data-testid="phone-nav-back"]'); await page.waitForTimeout(800);
  check("Back from a section → Settings list", await page.locator('[data-testid="phone-settings-list"]').count() > 0);
  await page.tap('[data-testid="phone-nav-back"]'); await page.waitForTimeout(1000);
  const onMe = await page.evaluate(() => !document.querySelector('[data-testid="phone-settings"]') && !!document.querySelector('.phone-fullscreen') && !document.querySelector('[data-testid="phone-channels-screen"]'));
  check("Back from Settings → Me (never the channel)", onMe, (await navState(page)).path);
  await page.tap('[data-testid="phone-nav-home"]'); await page.waitForTimeout(800);
  check("Home → Channels list", await page.locator('[data-testid="phone-channels-screen"]').count() > 0);
  await page.evaluate(() => document.querySelector('[data-testid="status-host"]')?.style.removeProperty("visibility"));
  await page.evaluate(() => { const h = document.querySelector('[data-testid="status-host"]'); h?.querySelector('[data-testid="web-push-banner"]')?.remove(); });

  // relay blip under the grace period: nothing shows
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(1500);
  drop(); await page.waitForTimeout(700); restore();
  let saw = false; for (let i = 0; i < 12; i++) { if (await page.locator('[data-testid="relay-status-pill"]').count()) saw = true; await page.waitForTimeout(250); }
  check("brief relay blip (<1.5s) shows no pill", !saw);
  await page.waitForTimeout(3000);
  // real outage: reconnecting after grace, persists, then Connected for ~2s
  drop();
  await page.waitForTimeout(1000);
  check("no reconnect pill inside the 1.5s grace", await page.locator('[data-testid="relay-status-pill"]').count() === 0);
  let rc = 0; for (let i = 0; i < 40 && !rc; i++) { await page.waitForTimeout(250); rc = await page.locator('[data-testid="relay-status-pill"][data-tone="progress"]').count(); }
  check("Reconnecting pill after the grace", rc > 0);
  await shot(page, "popup-reconnecting-relay");
  const look = await page.evaluate(() => { const el = document.querySelector('[data-testid="relay-status-pill"]'); if (!el) return null; const cs = getComputedStyle(el);
    const row = document.querySelector('[data-testid="phone-nav-row"]').getBoundingClientRect(); const r = el.getBoundingClientRect();
    return { bg: cs.backgroundColor, color: cs.color, border: cs.borderTopWidth, radius: cs.borderTopLeftRadius, belowRow: r.top >= row.bottom }; });
  check("pill is solid themed, bordered, below the nav row", look && !/rgba\(.*, 0\)/.test(look.bg) && look.border === "1px" && look.belowRow, JSON.stringify(look));
  await page.waitForTimeout(4000);
  check("Reconnecting persists while down", await page.locator('[data-testid="relay-status-pill"][data-tone="progress"]').count() > 0);
  restore();
  let ok = false; for (let i = 0; i < 60 && !ok; i++) { await page.waitForTimeout(250); ok = await page.locator('[data-testid="relay-status-pill"][data-tone="success"]').count() > 0; }
  check("Connected shows after a visible Reconnecting", ok);
  if (ok) await shot(page, "popup-connected");
  await page.waitForTimeout(2600);
  check("Connected gone after ~2s", await page.locator('[data-testid="relay-status-pill"]').count() === 0);

  // update prompt: persists until tapped
  fakeBuild = "newer-build";
  for (let i = 0; i < 25 && !(await page.locator('[data-testid="web-update-prompt"]').count()); i++) { await page.evaluate(() => window.dispatchEvent(new Event("focus"))); await page.waitForTimeout(1500); }
  check("update prompt shows", await page.locator('[data-testid="web-update-prompt"]').count() > 0);
  await shot(page, "popup-update-available");
  await page.waitForTimeout(6000);
  check("update prompt persists until acted on", await page.locator('[data-testid="web-update-prompt"]').count() > 0);
  // reconnecting + update together: stacked, no overlap
  drop(); await page.waitForTimeout(2600);
  await shot(page, "popup-stacked-reconnecting-and-update");
  restore(); await page.waitForTimeout(3000);
  fakeBuild = null;
  // Direct (Buzz Term) reconnect card
  await page.tap('[data-testid="phone-nav-home"]'); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-tab-term"]').catch(() => {}); await page.waitForTimeout(3000);
  await shot(page, "popup-direct-reconnecting");
  check("no page errors", errs.length === 0, errs.slice(0, 3).join(" | "));
  await b.close();
}
{
  // Allow → banner hides at once (subscribe runs in the background)
  const { b, ctx, page } = await open();
  await page.waitForTimeout(1000);
  const before = await page.locator('[data-testid="web-push-banner"]').count();
  await page.tap('[data-testid="web-push-enable"]').catch(() => {});
  await page.waitForTimeout(600);
  const after = await page.locator('[data-testid="web-push-banner"]').count();
  check("banner hides right after Allow", before > 0 && after === 0, `before=${before} after=${after}`);
  await shot(page, "popup-notifications-after-allow");
  await page.reload(); await page.waitForTimeout(6000);
  check("banner stays hidden after reload", await page.locator('[data-testid="web-push-banner"]').count() === 0);
  await b.close();
}
{
  // signer waiting toast (slow signer)
  const { b, page } = await open({ delay: 9000 });
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2000);
  const comp = page.locator('[data-testid="message-composer"] [contenteditable="true"]').last();
  await comp.tap(); await page.keyboard.type("hello from s24"); await page.keyboard.press("Enter");
  let seen = false; for (let i = 0; i < 30 && !seen; i++) { await page.waitForTimeout(250); seen = await page.locator('[data-testid="signer-waiting"]').count() > 0; }
  if (seen) await shot(page, "popup-signer-waiting");
  console.log(`signer waiting toast ${seen ? "shown" : "not shown (pref delay)"}`);
  await b.close();
}
console.log(`\n${res.filter(Boolean).length}/${res.length} passed`);
srv.close(); proxy.close(); process.exit(res.every(Boolean) ? 0 : 1);
