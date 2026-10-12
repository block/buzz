// Production-build PWA checks. Serves dist-web with a tiny static server (SPA fallback), then verifies installability bits.
// Usage: node scripts/pwa/pwa-check.mjs   (run `vite build -c vite.config.web.ts` first)
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const dist = path.resolve(import.meta.dirname, "../../dist-web");
const types = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
};
const server = http
  .createServer((req, res) => {
    const p = decodeURIComponent(new URL(req.url, "http://x").pathname);
    let f = path.join(dist, p);
    if (
      !f.startsWith(dist) ||
      !fs.existsSync(f) ||
      fs.statSync(f).isDirectory()
    )
      f = path.join(dist, "index.html");
    res.writeHead(200, {
      "Content-Type": types[path.extname(f)] || "application/octet-stream",
      ...(p === "/sw.js"
        ? { "Service-Worker-Allowed": "/", "Cache-Control": "no-cache" }
        : {}),
    });
    fs.createReadStream(f).pipe(res);
  })
  .listen(4174, "127.0.0.1");

const results = [];
const check = (n, ok, d = "") => {
  results.push(ok);
  console.log(`${ok ? "PASS" : "FAIL"}  ${n}${d ? "  — " + d : ""}`);
};
const base = "http://localhost:4174"; // localhost = secure context for service workers
const html = fs.readFileSync(path.join(dist, "index.html"), "utf8");
check(
  "P1 index.html has apple-mobile-web-app-capable, status-bar style, touch icon, viewport-fit=cover, theme-color, manifest link",
  /apple-mobile-web-app-capable/.test(html) &&
    /apple-mobile-web-app-status-bar-style/.test(html) &&
    /apple-touch-icon/.test(html) &&
    /viewport-fit=cover/.test(html) &&
    /name="theme-color"/.test(html) &&
    /rel="manifest"/.test(html),
);
const mf = JSON.parse(
  fs.readFileSync(path.join(dist, "manifest.webmanifest"), "utf8"),
);
check(
  "P2 manifest: standalone, start_url, scope, name/short_name, theme/background",
  mf.display === "standalone" &&
    mf.start_url &&
    mf.scope &&
    mf.name &&
    mf.short_name &&
    mf.theme_color &&
    mf.background_color,
);
const sizes = (mf.icons || []).map((i) => i.sizes + ":" + i.purpose);
check(
  "P3 manifest has 192 + 512 'any' and a 512 maskable icon, all files exist",
  ["192x192:any", "512x512:any", "512x512:maskable"].every((x) =>
    sizes.includes(x),
  ) && mf.icons.every((i) => fs.existsSync(path.join(dist, i.src))),
  sizes.join(" "),
);
const jsFiles = fs
  .readdirSync(path.join(dist, "assets"))
  .filter((f) => f.endsWith(".js"));
const bundle = jsFiles
  .map((f) => fs.readFileSync(path.join(dist, "assets", f), "utf8"))
  .join("\n");
check(
  "P4 prod bundle defaults relay to wss://buzz.huladesk.com",
  bundle.includes("wss://buzz.huladesk.com"),
);
check(
  "P5 prod bundle has no DEV test hooks / localStorage nsec path",
  !bundle.includes("__buzzWebInvoke") && !bundle.includes("buzz-web-nsec-hex"),
);

const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
});
await installMockExtension(ctx, "11".repeat(32));
const page = await ctx.newPage();
const relayReqs = [];
page.on("request", (r) => {
  if (/buzz\.huladesk\.com/.test(r.url())) relayReqs.push(r.url());
});
await page.goto(base + "/");
await page.waitForSelector('[data-testid="web-unlock"]');
await page
  .waitForFunction(
    () =>
      navigator.serviceWorker?.controller ||
      navigator.serviceWorker?.ready.then(() => true),
    null,
    { timeout: 15000 },
  )
  .catch(() => {});
await page.evaluate(() => navigator.serviceWorker.ready);
await page.reload();
await page.waitForSelector('[data-testid="web-unlock"]');
check(
  "P6 service worker registered and controlling the page",
  await page.evaluate(() => !!navigator.serviceWorker.controller),
);
const cached = await page.evaluate(async () => {
  const out = [];
  for (const k of await caches.keys())
    for (const r of await (await caches.open(k)).keys())
      out.push(new URL(r.url).pathname);
  return out;
});
check(
  "P7 SW cached the app shell",
  cached.includes("/") &&
    cached.includes("/manifest.webmanifest") &&
    cached.some((p) => p.startsWith("/assets/")),
  `${cached.length} entries`,
);
check(
  "P8 SW does not cache API/relay paths",
  !cached.some((p) => /^\/(query|events|count|media|upload|api|info)/.test(p)),
);
await ctx.setOffline(true);
await page.reload();
check(
  "P9 offline: app shell still loads (unlock screen) from SW cache",
  !!(await page
    .waitForSelector('[data-testid="web-unlock"]', { timeout: 10000 })
    .catch(() => null)),
);
await ctx.setOffline(false);
const mq = await page.evaluate(() => ({
  standalone: matchMedia("(display-mode: standalone)").matches,
  vv: document.querySelector("meta[name=viewport]").content,
}));
check(
  "P10 viewport meta includes viewport-fit=cover",
  /viewport-fit=cover/.test(mq.vv),
);
await browser.close();
server.close();
console.log(
  `\n${results.filter(Boolean).length}/${results.length} checks passed`,
);
process.exit(results.every(Boolean) ? 0 : 1);
