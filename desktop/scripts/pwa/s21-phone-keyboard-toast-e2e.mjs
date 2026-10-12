// Phone batch 5: keyboard (fake visualViewport resize + pan) keeps the composer above the keyboard; signer toast is a true overlay.
// channels list without profile bar / menu dock, Me rows + counts, settings horizontal overflow at 390 and 375.
// Build with VITE_BUZZ_RELAY_URL=ws://localhost:3100 to S21_DIST (default /tmp/phone-dist); local relay on :3100.
import fs from "node:fs"; import http from "node:http"; import path from "node:path";
import { chromium, devices, webkit } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const dist = process.env.S21_DIST || "/tmp/phone-dist";
const out = process.env.S21_OUT || "/Users/jm/Documents/phone-audit-shots";
fs.mkdirSync(out, { recursive: true });
const T = { ".html":"text/html",".js":"text/javascript",".css":"text/css",".svg":"image/svg+xml",".png":"image/png" };
const srv = http.createServer((q, r) => { const p = decodeURIComponent(new URL(q.url,"http://x").pathname); let f = path.join(dist,p); if (!fs.existsSync(f)||fs.statSync(f).isDirectory()) f = path.join(dist,"index.html"); r.writeHead(200,{"Content-Type":T[path.extname(f)]||"application/octet-stream"}); fs.createReadStream(f).pipe(r); }).listen(4180,"127.0.0.1");
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
  await ctx.addInitScript(() => {
    const et = new EventTarget(); let kh = null, kt = 0;
    const vv = Object.defineProperties(et, {
      height: { get: () => kh ?? innerHeight }, width: { get: () => innerWidth }, offsetTop: { get: () => kt }, pageTop: { get: () => kt },
      offsetLeft: { value: 0 }, pageLeft: { value: 0 }, scale: { value: 1 } });
    Object.defineProperty(window, "visualViewport", { value: vv, configurable: true });
    window.__setVV = (h, top) => { kh = h; kt = top; vv.dispatchEvent(new Event("resize")); vv.dispatchEvent(new Event("scroll")); };
    window.__clearVV = () => { kh = null; kt = 0; vv.dispatchEvent(new Event("resize")); vv.dispatchEvent(new Event("scroll")); };
  });
  const page = await ctx.newPage(); const errs = []; page.on("pageerror", e => errs.push(String(e)));
  await page.goto("http://localhost:4180/");
  await page.click('[data-testid="web-unlock-nip07"]', { timeout: 30000 });
  await page.waitForSelector('[data-testid="app-top-chrome"], [data-testid="phone-tabbar"]', { timeout: 30000 });
  await page.waitForTimeout(5000);
  return { b, page, errs };
}

const IPHONE = { ...devices["iPhone 14"], viewport: { width: 390, height: 664 }, screen: { width: 390, height: 844 } };
const STANDALONE = { ...IPHONE, viewport: { width: 390, height: 844 }, userAgent: devices["iPhone 14"].userAgent.replace("OS 16_0", "OS 18_0") };
async function keyboardCase(label, opts, flags, kbH, pan) {
  const { b, page, errs } = await open(webkit, opts, flags);
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2500);
  const comp = page.locator('[data-testid="message-composer"]');
  await comp.locator('[contenteditable="true"], textarea').last().tap(); await page.waitForTimeout(300);
  const H = await page.evaluate(() => innerHeight);
  await page.evaluate(({ h, top }) => { window.__setVV(h, top); }, { h: H - kbH, top: pan });
  await page.waitForTimeout(800);
  const m = await page.evaluate(() => {
    const vv = window.visualViewport; const r = (s) => document.querySelector(s)?.getBoundingClientRect();
    const rows = [...document.querySelectorAll('[data-testid="message-row"]')].map((e) => e.getBoundingClientRect()).filter((x) => x.height > 0);
    const last = rows.length ? rows.reduce((a, x) => (x.bottom > a.bottom ? x : a)) : null;
    return { vTop: vv.offsetTop, vH: vv.height, root: r("#root")?.top, comp: r('[data-testid="message-composer"]'), last: last && { top: last.top, bottom: last.bottom }, kb: document.documentElement.hasAttribute("data-phone-keyboard") };
  });
  const vBottom = m.vTop + m.vH;
  const extra = await page.evaluate(() => ({ tab: !!document.querySelector('[data-testid="phone-tabbar"]') && getComputedStyle(document.querySelector('[data-testid="phone-tabbar"]')).display !== "none",
    fs: parseFloat(getComputedStyle(document.activeElement).fontSize), tag: document.activeElement?.getAttribute("data-testid") }));
  check(`${label}: tab bar hidden while typing`, !extra.tab);
  check(`${label}: focused composer text is >=16px (no iOS zoom)`, extra.fs >= 16, JSON.stringify(extra));
  check(`${label}: keyboard detected`, m.kb);
  check(`${label}: shell follows the visual viewport top`, Math.abs(m.root - m.vTop) <= 1, `root=${m.root} vTop=${m.vTop}`);
  check(`${label}: composer sits directly above the keyboard (no gap)`, m.comp.bottom <= vBottom + 1 && vBottom - m.comp.bottom <= 6, `compBottom=${Math.round(m.comp.bottom)} vvBottom=${vBottom}`);
  check(`${label}: composer is not at the top`, m.comp.top > m.vTop + m.vH / 2, `compTop=${Math.round(m.comp.top)}`);
  check(`${label}: latest message visible just above the composer`, m.last && m.last.bottom <= m.comp.top + 1 && m.last.top >= m.vTop - 1 && m.comp.top - m.last.bottom < 120, JSON.stringify(m.last));
  await page.screenshot({ path: `${out}/b5-${label}-keyboard.png` });
  await page.evaluate(() => { window.__clearVV(); document.activeElement?.blur(); });
  await page.waitForTimeout(600);
  const after = await page.evaluate(() => ({ root: document.getElementById("root").getBoundingClientRect().top, kb: document.documentElement.hasAttribute("data-phone-keyboard") }));
  check(`${label}: keyboard closed restores the shell`, after.root === 0 && !after.kb, JSON.stringify(after));
  check(`${label}: no page errors`, errs.filter((e) => !/ResizeObserver/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
// Every editable field on a touch phone is >=16px (iOS zooms the page into anything smaller).
{
  const { b, page } = await open(webkit, IPHONE);
  const scan = () => page.evaluate(() => [...document.querySelectorAll('input:not([type=checkbox]):not([type=radio]):not([type=range]):not([type=color]),textarea,select,[contenteditable="true"]')]
    .map((e) => ({ id: e.getAttribute("data-testid") || e.getAttribute("placeholder") || e.className.toString().slice(0, 30), fs: parseFloat(getComputedStyle(e).fontSize) })));
  const all = [];
  await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap(); await page.waitForTimeout(2000);
  all.push(...await scan());
  await page.tap('[data-testid="phone-back"]').catch(() => {}); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-tab-search"]'); await page.waitForTimeout(1000); all.push(...await scan());
  await page.tap('[data-testid="phone-tab-me"]'); await page.waitForTimeout(600);
  await page.tap('[data-testid="phone-me-settings"]'); await page.waitForTimeout(1200);
  for (const sec of ["profile", "appearance"]) { await page.tap(`[data-testid="settings-nav-${sec}"]`).catch(() => {}); await page.waitForTimeout(1000); all.push(...await scan()); await page.tap('[data-testid="phone-back"]').catch(() => {}); await page.waitForTimeout(500); }
  const small = all.filter((x) => x.fs < 16);
  check("touch: all text fields >=16px (composer, search, settings)", all.length >= 3 && small.length === 0, `n=${all.length} small=${JSON.stringify(small.slice(0, 4))}`);
  const meta = await page.evaluate(() => document.querySelector('meta[name=viewport]')?.content);
  check("viewport still allows pinch zoom (no maximum-scale / user-scalable=no)", !/maximum-scale|user-scalable\s*=\s*no/.test(meta), meta);
  await b.close();
}
// Diagnostics overlay: ?kbdebug=1 turns it on (saved), shows live numbers, never takes taps; Off button / ?kbdebug=0 turn it off.
{
  const { b, page } = await open(webkit, IPHONE);
  const pad = await page.evaluate(() => getComputedStyle(document.getElementById("root")).paddingBottom);
  check("phone: app frame has no bottom padding (home-indicator inset not double counted)", pad === "0px", pad);
  await page.goto("http://localhost:4180/?kbdebug=1"); await page.waitForTimeout(4000);
  const o = page.locator('[data-testid="kbdebug-overlay"]');
  check("kbdebug overlay shown via ?kbdebug=1", await o.isVisible());
  const txt = await o.innerText();
  check("overlay shows viewport, safe-area, frame, composer, tab bar, term lines", ["inner ", "vv h", "safe-area", "frame top", "--phone-vh", "composer bottom", "tabbar", "key→echo", "ping"].every((k) => txt.includes(k)), txt.split("\n").slice(0, 3).join(" | "));
  check("overlay does not take taps", (await o.evaluate((e) => getComputedStyle(e).pointerEvents)) === "none");
  await page.reload(); await page.waitForTimeout(4000);
  check("overlay setting persists across reload", await o.isVisible());
  await page.screenshot({ path: `${out}/b6-kbdebug.png` });
  await page.locator('[data-testid="kbdebug-off"]').click(); await page.waitForTimeout(300);
  check("Off button hides it and clears the setting", !(await o.isVisible()) && (await page.evaluate(() => localStorage.getItem("buzz.kbdebug"))) === null);
  await b.close();
}
// desktop (mouse) keeps its own field sizes
{
  const { b, page } = await open(webkit, { viewport: { width: 1280, height: 800 } });
  await page.locator('[data-testid="channel-general"]').first().click(); await page.waitForTimeout(2000);
  const fs = await page.evaluate(() => parseFloat(getComputedStyle(document.querySelector('[data-testid="message-input"]')).fontSize));
  console.log(`desktop composer font-size: ${fs}px (touch rule not applied: pointer is fine)`);
  await b.close();
}
await keyboardCase("safari-tab", IPHONE, {}, 336, 250);
await keyboardCase("standalone", STANDALONE, { safe: [47, 34], standalone: true }, 336, 0);

// Toast overlay: showing it must not move anything, Top and Bottom, phone + desktop.
async function toastCase(label, opts, position, flags = {}) {
  const { b, page, errs } = await open(webkit, opts, { ...flags, prefs: { "buzz.web.signerToast": JSON.stringify({ mode: "always", position }) } });
  const phone = opts.viewport.width < 600;
  if (phone) await page.locator('[data-testid="phone-channels-screen"] [data-testid="channel-general"]').first().tap();
  else await page.locator('[data-testid="channel-general"]').first().click();
  await page.waitForTimeout(2500);
  const snap = () => page.evaluate(() => [...document.querySelectorAll('#root *')].filter((e) => !e.closest('[data-testid="phone-tabbar"]') || true).filter((e) => e.getBoundingClientRect().height > 0 && !e.closest('[data-testid="message-composer"],[data-testid="message-timeline"]')).map((e) => { const r = e.getBoundingClientRect(); return `${e.getAttribute('data-testid') || e.tagName}:${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`; }).join("|") + `#${scrollY}/${document.getElementById("root").scrollTop}/${document.getElementById("root").getBoundingClientRect().top}`);
  const comp = page.locator('[contenteditable="true"], textarea').last();
  await comp.click(); await page.keyboard.type(`t${Date.now() % 100000}`);
  await page.evaluate(() => { window.__nostrStall = true; });
  const before = await snap();
  const tl0 = await page.evaluate(() => document.querySelector('[data-testid="message-timeline"]')?.getBoundingClientRect().height);
  await page.keyboard.press("Enter");
  const t = page.locator('[data-testid="signer-waiting"]');
  const shown = await t.waitFor({ timeout: 15000 }).then(() => true).catch(() => false);
  await page.waitForTimeout(400);
  check(`${label}: toast shows`, shown);
  if (shown) {
    const info = await page.evaluate(() => { const e = document.querySelector('[data-testid="signer-waiting"]'); const cs = getComputedStyle(e); const r = e.getBoundingClientRect(); return { pos: cs.position, parent: e.parentElement.tagName, pe: cs.pointerEvents, top: Math.round(r.top), bottom: Math.round(r.bottom), h: innerHeight }; });
    check(`${label}: toast is fixed, outside the app tree`, info.pos === "fixed" && info.parent === "HTML", JSON.stringify(info));
    check(`${label}: toast on screen at the ${position}`, info.top >= 0 && info.bottom <= info.h && (position === "top" ? info.top < info.h / 3 : info.bottom > info.h * 0.6), JSON.stringify(info));
    if (position === "bottom") {
      const dock = await page.evaluate(() => Math.min(...[...document.querySelectorAll('[data-testid="message-composer"], [data-testid="phone-tabbar"]')].map((e) => e.getBoundingClientRect()).filter((r) => r.height > 0).map((r) => r.top)));
      check(`${label}: bottom toast clears the composer / tab bar`, info.bottom <= dock + 1, `toast=${info.bottom} dock=${Math.round(dock)}`);
    }
    const after = await snap();
    const tl1 = await page.evaluate(() => document.querySelector('[data-testid="message-timeline"]')?.getBoundingClientRect().height);
    const A = before.split("|"), B2 = after.split("|");
    const diff = A.filter((x, i) => B2[i] !== x).length; if (diff) console.log('diff', A.filter((x, i) => B2[i] !== x).slice(0, 6).map((x) => x + ' -> ' + B2[A.indexOf(x)]).join(' ; '));
    check(`${label}: showing the toast moved no content`, A.length === B2.length && diff === 0 && Math.abs((tl0 ?? 0) - (tl1 ?? 0)) <= 1, `changed=${diff}/${A.length} len ${A.length}->${B2.length} timeline ${tl0}->${tl1}`);
    await page.screenshot({ path: `${out}/b5-toast-${label}.png` });
  }
  await page.evaluate(() => { window.__nostrStall = false; });
  check(`${label}: no page errors`, errs.filter((e) => !/ResizeObserver/.test(e)).length === 0, errs.slice(0, 2).join(" | "));
  await b.close();
}
await toastCase("phone-top", IPHONE, "top");
await toastCase("phone-bottom", IPHONE, "bottom");
await toastCase("standalone-bottom", STANDALONE, "bottom", { safe: [47, 34], standalone: true });
await toastCase("desktop-bottom", { viewport: { width: 1280, height: 800 } }, "bottom");
await toastCase("desktop-top", { viewport: { width: 1280, height: 800 } }, "top");
srv.close();
const pass = res.filter(Boolean).length; console.log(`\n${pass}/${res.length} passed`); process.exit(pass === res.length ? 0 : 1);
