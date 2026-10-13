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

const { b, page, errs } = await open();
page.on("console", m => { if (m.type()==="error") console.log("CONSOLE", m.text().slice(0,600)); });
await page.tap("[data-testid=phone-tab-inbox]"); await page.waitForTimeout(3000);
await page.screenshot({ path: out + "/inbox-open.png" });
for (const l of ["Inbox options","All"]) { await page.locator(`button[aria-label="${l}"], button:has-text("${l}")`).first().click({timeout:5000, force:true}).catch(e=>console.log("tapfail",l)); await page.waitForTimeout(800); const t = await page.evaluate(()=>document.body.innerText); console.log("after",l, /went wrong/i.test(t) ? "ERROR BOUNDARY" : "ok", (await page.locator('[role=menu],[role=dialog],[data-radix-popper-content-wrapper]').count())); await page.keyboard.press("Escape"); await page.waitForTimeout(400); }
await page.tap("[data-testid=phone-tab-channels]"); await page.waitForTimeout(800);
const ch = page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first();
const bb = await ch.boundingBox();
await page.mouse.move(bb.x+20, bb.y+10); await ch.dispatchEvent("contextmenu"); await page.waitForTimeout(1000);
const t2 = await page.evaluate(()=>document.body.innerText);
console.log("contextmenu", /went wrong/i.test(t2) ? "ERROR BOUNDARY" : "ok", /Add as Tab|tab/i.test(t2) ? "menu shown" : "no menu");
await page.screenshot({ path: out + "/hotfix-context-menu.png" });
const body = await page.evaluate(() => document.body.innerText.slice(0,400));
console.log("BODY", JSON.stringify(body));
const btns = await page.evaluate(() => [...document.querySelectorAll("button")].map(b=>b.innerText||b.getAttribute("aria-label")).filter(Boolean).slice(0,40));
console.log("BTNS", JSON.stringify(btns));
console.log("ERRS", errs.join(" | ").slice(0,1500));
await b.close(); process.exit(0);