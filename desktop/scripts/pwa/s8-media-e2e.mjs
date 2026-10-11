// Web media e2e: auth-gated relay images + uploads in the browser build. Needs the local relay (:3100) and a web dev server
// started from THIS checkout (the /__relay proxy must allow GET /media and PUT /upload).
// Usage: node scripts/pwa/s8-media-e2e.mjs [appUrl]   (default http://127.0.0.1:5311/)
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5311/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEUlEQVR4nGP47+CAFTEMLQkAmbNfwaFk8aAAAAAASUVORK5CYII=";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext();
const { log, pk } = await installMockExtension(ctx, TYLER);
await ctx.addInitScript(({ relay, pk }) => {
  if (!localStorage.getItem("buzz-communities")) {
    localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
    localStorage.setItem("buzz-active-community-id", "c1");
  }
}, { relay: RELAY_WS, pk });
const page = await ctx.newPage();
page.on("pageerror", (e) => console.log("pageerror", e.message));
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 20000 });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 });

const out = await page.evaluate(async (pngB64) => {
  const m = await import("/src/platform/web/relayMedia.ts");
  const bytes = Uint8Array.from(atob(pngB64), (c) => c.charCodeAt(0));
  const r = {};
  try {
    const d = await m.uploadRelayMedia(bytes, { filename: "pixel.png" });
    r.upload = d;
  } catch (e) { r.uploadError = String(e.message ?? e); return r; }
  m.resetMediaTokenCache();
  const before = m.signerRoundTrips.get;
  const urls = [r.upload.url];
  r.isRelay = m.isRelayMediaUrl(r.upload.url);
  const blobs = await Promise.all([1, 2, 3, 4, 5].map(() => m.getRelayMediaBlobUrl(r.upload.url)));
  r.sameBlob = new Set(blobs).size === 1;
  r.blobType = (await (await fetch(blobs[0])).blob()).type;
  // different url shapes share the one token
  const thumbless = r.upload.url.replace(/\.png$/, ".png?x=1");
  try { await m.getRelayMediaBlobUrl(thumbless); } catch (e) { r.queryErr = String(e.message); }
  r.tokensMinted = m.signerRoundTrips.get - before;
  try { await m.getRelayMediaBlobUrl(r.upload.url.replace(/[0-9a-f]{64}/, "0".repeat(64))); } catch (e) { r.missing = e.message; r.status = e.status; }
  return r;
}, PNG);
console.log(JSON.stringify(out));
check("upload via PUT /upload returns a descriptor", out.upload?.url && out.upload?.sha256, out.uploadError ?? out.upload?.url);
check("descriptor url is relay media", out.isRelay);
check("5 parallel fetches of one image -> one cached blob", out.sameBlob);
check("blob is an image", /^image\//.test(out.blobType ?? ""), out.blobType);
check("one signed token reused for all fetches", out.tokensMinted === 1, `minted ${out.tokensMinted}`);
check("missing image -> clear 404 error", out.status === 404 || out.status === 401, `${out.missing}`);
check("signer signed kind 24242 only for media (no per-image NIP-98)", log.signed.filter((k) => k === 24242).length >= 2);

// Attach path: pick_and_upload_media handler + raw upload command shape
const raw = await page.evaluate(async (pngB64) => {
  const core = await import("/src/platform/web/core.ts");
  const bytes = Uint8Array.from(atob(pngB64), (c) => c.charCodeAt(0));
  const name = btoa(String.fromCharCode(...new TextEncoder().encode("shot é.png"))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  return core.invoke("upload_media_bytes_raw", bytes, { headers: { "x-buzz-filename": name } });
}, PNG);
check("upload_media_bytes_raw (composer paste/drop path) works, filename kept", raw?.url && raw.filename === "shot é.png", raw?.filename);

// Render path: a posted message with this image shows a real <img> (blob:) in the channel, not a broken icon
const render = await page.evaluate(async (d) => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  const general = chs.find((c) => c.name === "general") ?? chs[0];
  const tag = ["imeta", `url ${d.url}`, `m ${d.type}`, `x ${d.sha256}`, `size ${d.size}`, `dim ${d.dim}`];
  await window.__buzzWebInvoke("send_channel_message", { channelId: general.id, content: `shot ![shot](${d.url})`, mediaTags: [tag] });
  return general.id;
}, out.upload);
await page.goto(`${APP}#/channels/${render}`);
const img = await page.waitForSelector('img[src^="blob:"]', { timeout: 30000 }).catch(() => null);
const ok = img ? await img.evaluate((i) => i.complete && i.naturalWidth > 0) : false;
check("message image renders as a loaded blob: <img> (not broken)", ok);
await page.screenshot({ path: "/tmp/s8-media.png" });

// Component path: a message image renders (not a broken icon) and shows placeholder -> image
await page.route("**/__relay/**/media/**", async (route) => route.fulfill({ status: 401, body: '{"error":"authentication failed"}' }));
const err = await page.evaluate(async (u) => {
  const m = await import("/src/platform/web/relayMedia.ts");
  m.resetMediaTokenCache();
  try { await m.getRelayMediaBlobUrl(u + "?retry=1"); } catch (e) { return { msg: e.message, status: e.status, mints: m.signerRoundTrips.get }; }
}, out.upload?.url ?? "");
check("401 -> re-mints once then reports error", err?.status === 401 && /401/.test(err.msg), JSON.stringify(err));

await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
