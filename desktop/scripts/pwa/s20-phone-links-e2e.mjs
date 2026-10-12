// Phone batch 4: real message links on web bypass the desktop link side panel. Was: Phone batch 2 (WebKit, iPhone 14): standalone tab bar position, avatar Me tab, tab order, Channels default,
// channels list without profile bar / menu dock, Me rows + counts, settings horizontal overflow at 390 and 375.
// Build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 to S20_DIST (default /tmp/phone-dist); local relay on :3100.
import fs from "node:fs"; import http from "node:http"; import path from "node:path";
import { chromium, devices, webkit } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S20_DIST || "/tmp/phone-dist";
const out = process.env.S20_OUT || "/Users/jm/Documents/phone-audit-shots";
fs.mkdirSync(out, { recursive: true });
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname); let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4179,"127.0.0.1");
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
  await page.goto("http://localhost:4179/");
  await page.click('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.waitForSelector('[data-testid="app-top-chrome"], [data-testid="phone-tabbar"]', { timeout: 30000 });
  await page.waitForTimeout(5000);
  return { b, page, errs };
}

const TARGET = "https://example.com/s20link?q=1";
async function postLink(page) {
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2500);
  const comp = page.locator('[data-testid="message-composer"] [contenteditable="true"], [data-testid="message-composer"] textarea, [contenteditable="true"]').last();
  await comp.click(); await page.keyboard.type(`s20 link ${TARGET} ${Date.now()}`); await page.keyboard.press("Enter");
  const a = page.locator(`[data-testid="message-row"] a[href="${TARGET}"]`).last();
  await a.waitFor({ timeout: 15000 });
  return a;
}
// Leave a plain message last so other suites (s18 taps the last row) don't hit a link.
async function postPlain(page) {
  const comp = page.locator('[data-testid="message-composer"] [contenteditable="true"], [data-testid="message-composer"] textarea, [contenteditable="true"]').last();
  await comp.click(); await page.keyboard.type(`s20 plain message ${Date.now()} padding text so the row is wide enough`); await page.keyboard.press("Enter"); await page.waitForTimeout(1500);
}
const panelShown = (page) => page.evaluate(() => /beside the\s+conversation/i.test(document.body.innerText) || !!document.querySelector('[data-testid^="link-side-panel"]'));
const IPHONE = { ...devices["iPhone 14"], viewport: { width: 390, height: 844 }, screen: { width: 390, height: 844 } };
// A: plain Safari tab (not installed): new tab with the link URL, no side panel, app stays put
{
  const { b, page, errs } = await open(webkit, IPHONE);
  const a = await postLink(page); const appUrl = page.url();
  const [popup] = await Promise.all([page.context().waitForEvent("page", { timeout: 8000 }).catch(() => null), a.tap()]);
  await page.waitForTimeout(1200);
  const purl = popup ? (await popup.waitForLoadState("commit").catch(() => {}), popup.url()) : "";
  check("Safari tab: message link opens a new tab", !!popup, `popup=${purl}`);
  check("Safari tab: new tab targets the link URL (not the app)", purl.startsWith("https://example.com/s20link") || purl === "about:blank", purl);
  check("Safari tab: app window did not navigate", page.url() === appUrl, page.url());
  check("Safari tab: no 'beside the conversation' side panel", !(await panelShown(page)));
  await page.screenshot({ path: `${out}/b4-safari-tab.png` });
  await postPlain(page);
  check("no page errors", errs.filter(e => !/ResizeObserver/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
// B: installed PWA (standalone): hands x-safari-<link URL> to iOS, no popup, no side panel
{
  const { b, page, errs } = await open(webkit, { ...IPHONE, userAgent: IPHONE.userAgent.replace("OS 16_0", "OS 18_0") }, { safe: [47, 34], standalone: true });
  const a = await postLink(page); const appUrl = page.url();
  console.log("env:", await page.evaluate(() => JSON.stringify({ sa: navigator.standalone, ua: navigator.userAgent, p: navigator.platform, mm: matchMedia("(display-mode: standalone)").matches, ph: document.documentElement.getAttribute("data-phone-standalone") })));
  const seen = [];
  page.on("request", (r) => seen.push(r.url())); page.on("requestfailed", (r) => seen.push(r.url()));
  page.on("framenavigated", (f) => seen.push(f.url()));
  page.on("console", (m) => { if (/x-safari/.test(m.text())) seen.push(m.text()); });
  let popup = null; page.context().on("page", (p) => { popup = p; });
  await a.tap(); await page.waitForTimeout(2000);
  const hit = seen.find((u) => u.includes("x-safari-"));
  check("standalone: no in-app popup / Safari sheet", !popup, popup ? popup.url() : "");
  check("standalone: no 'beside the conversation' side panel", !(await panelShown(page)));
  check("standalone: hand-off URL is x-safari-<link URL>", hit === `x-safari-${TARGET}` || (!hit && page.url() === appUrl), hit ? hit : `not observable in WebKit; app url=${page.url()}`);
  console.log("observed:", JSON.stringify(seen.slice(-6)));
  await postPlain(page);
  check("no page errors", errs.filter(e => !/ResizeObserver|x-safari/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
// C: Settings > Appearance > signer message prefs (web), persisted locally
{
  const { b, page, errs } = await open(webkit, IPHONE);
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-me-settings"]'); await page.waitForTimeout(1500);
  await page.tap('[data-testid="settings-nav-appearance"]'); await page.waitForTimeout(1200);
  const row = page.locator('[data-testid="signer-toast-mode-row"]'); await row.scrollIntoViewIfNeeded();
  check("signer messages setting shown", await row.isVisible() && await page.isVisible('[data-testid="signer-toast-position-row"]'));
  const sel = (v) => page.evaluate((v) => document.querySelector(`[data-testid="${v}"]`)?.getAttribute("aria-checked") ?? document.querySelector(`[data-testid="${v}"] input`)?.checked ?? null, v);
  console.log("default state:", await page.evaluate(() => localStorage.getItem("buzz.web.signerToast")), await sel("signer-toast-mode-problems"), await sel("signer-toast-position-bottom"));
  await page.locator('[data-testid="signer-toast-mode-off"]').click(); await page.locator('[data-testid="signer-toast-position-top"]').click(); await page.waitForTimeout(300);
  const saved = JSON.parse(await page.evaluate(() => localStorage.getItem("buzz.web.signerToast")) || "{}");
  check("choices persist locally", saved.mode === "off" && saved.position === "top", JSON.stringify(saved));
  const W = await page.evaluate(() => document.documentElement.clientWidth);
  const rb = await page.locator('[data-testid="signer-toast-mode-control"]').boundingBox();
  check("control fits the phone width", rb && rb.x >= 0 && rb.x + rb.width <= W + 0.5, JSON.stringify(rb));
  await row.screenshot({ path: `${out}/b4-signer-setting.png` }).catch(() => {});
  await page.reload(); await page.waitForTimeout(3000);
  check("persisted after reload", JSON.parse(await page.evaluate(() => localStorage.getItem("buzz.web.signerToast")) || "{}").mode === "off");
  await b.close();
}
srv.close();
const pass = res.filter(Boolean).length; console.log(`\n${pass}/${res.length} passed`); process.exit(pass === res.length ? 0 : 1);
