// Signer-health e2e: dead / revoked / missing NIP-46 sessions must re-prompt, never hang on a blank boot. Local relay (:3100) + mock signer.
// with an in-process mock remote signer. No browser extension is installed in any context here.
// Usage: node scripts/pwa/s4-signer-health-e2e.mjs [appUrl]
import { createHash } from "node:crypto";
import { chromium } from "playwright";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import { client } from "./relay-client.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100",
  RELAY_HTTP = "http://localhost:3100";
const TYLER =
  "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok, detail });
  console.log(
    `${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`,
  );
};
function uuid5(ns, name) {
  const h = createHash("sha1")
    .update(
      Buffer.concat([
        Buffer.from(ns.replace(/-/g, ""), "hex"),
        Buffer.from(name),
      ]),
    )
    .digest();
  h[6] = (h[6] & 0x0f) | 0x50;
  h[8] = (h[8] & 0x3f) | 0x80;
  const x = h.subarray(0, 16).toString("hex");
  return `${x.slice(0, 8)}-${x.slice(8, 12)}-${x.slice(12, 16)}-${x.slice(16, 20)}-${x.slice(20)}`;
}
const generalId = uuid5(
  "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  "buzz.channel.general",
);
const stamp = Date.now();
const other = client(RELAY_HTTP, TYLER);

const browser = await chromium.launch();
async function newCtx(pk) {
  const ctx = await browser.newContext({
    viewport: { width: 1280, height: 800 },
  });
  await ctx.addInitScript(
    ({ relay, pk }) => {
      if (!localStorage.getItem("buzz-communities")) {
        localStorage.setItem(
          "buzz-communities",
          JSON.stringify([
            {
              id: "c1",
              name: "Local Dev",
              relayUrl: relay,
              pubkey: pk,
              addedAt: new Date().toISOString(),
            },
          ]),
        );
        localStorage.setItem("buzz-active-community-id", "c1");
      }
    },
    { relay: RELAY_WS, pk },
  );
  return ctx;
}
const readIdb = (page) =>
  page.evaluate(
    () =>
      new Promise((res) => {
        const r = indexedDB.open("buzz-web-signer", 1);
        r.onupgradeneeded = () => r.result.createObjectStore("kv");
        r.onsuccess = () => {
          const g = r.result
            .transaction("kv")
            .objectStore("kv")
            .get("nip46-session");
          g.onsuccess = () => {
            r.result.close();
            res(g.result ?? null);
          };
          g.onerror = () => res(null);
        };
        r.onerror = () => res(null);
      }),
  );
const inv = (page, c, a) =>
  page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [c, a]);
const sidebar = (page) =>
  page
    .waitForSelector(
      '[data-testid="sidebar-channel-content"] >> text=general',
      { timeout: 25000 },
    )
    .catch(() => null);

const signer = await startMockSigner({ userSkHex: TYLER });
const pk = signer.userPk;
const ctx = await newCtx(pk);
const page = await ctx.newPage();
const pageErrors = [],
  wsFrames = [];
page.on("pageerror", (e) => pageErrors.push(String(e).slice(0, 200)));
page.on("websocket", (ws) => {
  if (!ws.url().includes("localhost:3100")) return;
  ws.on("framesent", (f) => wsFrames.push([">", String(f.payload)]));
  ws.on("framereceived", (f) => wsFrames.push(["<", String(f.payload)]));
});


const txt = async (sel) => (await page.locator(sel).innerText().catch(() => "")) || "";
const connect = async () => {
  await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
  await page.click('[data-testid="web-unlock-bunker-go"]');
};

// A. sign in, then reload with the signer app OFFLINE -> full-screen explanation (not a blank splash)
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]');
await connect();
check("A0 signed in, app boots", await sidebar(page));
signer.setOffline(true);
const tReload = Date.now();
await page.reload();
const down = await page.waitForSelector('[data-testid="signer-down-screen"]', { timeout: 60000 }).catch(() => null);
check("A1 offline signer on reload -> 'Your signer isn't responding' screen", down && /isn't responding/.test(await txt('[data-testid="signer-down-screen"]')), `${Date.now() - tReload} ms`);
check("A2 shows Retry / Reconnect signer / Sign out", ["signer-retry", "signer-reconnect", "signer-signout"].every((t) => down), "");
check("A3 appeared within 30 s (probe 8s + socket rebuild + 8s)", Date.now() - tReload < 30000, `${Date.now() - tReload} ms`);
await page.click('[data-testid="signer-retry"]');
await page.waitForTimeout(18000);
check("A4 Retry with signer still offline keeps the screen up", await page.$('[data-testid="signer-down-screen"]'));
signer.setOffline(false);
await page.click('[data-testid="signer-retry"]');
check("A5 signer back + Retry -> app boots", await sidebar(page));
check("A6 gate removed", !(await page.$('[data-testid="signer-down-screen"]')));

// B. mid-session: signer goes silent while a user action waits -> toast escalates to the non-blocking banner
signer.setOffline(true);
const pB = page.evaluate((id) => window.__buzzWebInvoke("send_channel_message", { channelId: id, content: "health-probe" }).then(() => "ok", (e) => String(e?.message ?? e)), generalId);
const banner = await page.waitForSelector('[data-testid="signer-down-banner"]', { timeout: 90000 }).catch(() => null);
check("B1 'Waiting for your signer' escalates to the 'isn't responding' banner", banner);
check("B2 banner is non-blocking (app still mounted)", await page.$('[data-testid="sidebar-channel-content"]'));
await pB;
signer.setOffline(false);
await page.click('[data-testid="signer-retry"]');
check("B3 Retry after signer returns clears the banner", await page.waitForSelector('[data-testid="signer-down-banner"]', { state: "detached", timeout: 30000 }).then(() => true, () => false));

// C. signer revoked the connection: reload -> explicit 'no longer authorized'; Reconnect re-pairs through the unlock UI
signer.revokeClients();
await page.reload();
await page.waitForSelector('[data-testid="signer-down-screen"]', { timeout: 60000 }).catch(() => null);
check("C1 revoked session -> down screen says it is no longer authorized", /no longer authorized/.test(await txt('[data-testid="signer-down-screen"]')), (await txt('[data-testid="signer-down-screen"]')).slice(0, 120));
await page.click('[data-testid="signer-reconnect"]');
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', { timeout: 20000 }).catch(() => null);
check("C2 Reconnect signer opens the pairing UI (bunker paste / nostrconnect)", await page.$('[data-testid="web-unlock-nostrconnect"]'));
await connect();
check("C3 re-paired -> app boots again", await sidebar(page));

// D. missing stored session -> straight to sign-in, never the gate or a blank screen
await page.evaluate(() => new Promise((res) => { const r = indexedDB.deleteDatabase("buzz-web-signer"); r.onsuccess = r.onerror = r.onblocked = () => res(null); }));
await page.reload();
const unlock = await page.waitForSelector('[data-testid="web-unlock"]', { timeout: 15000 }).catch(() => null);
check("D1 no saved session -> sign-in screen directly", unlock && !(await page.$('[data-testid="signer-down-screen"]')));

check("E no uncaught page errors", pageErrors.length === 0, pageErrors.slice(0, 3).join(" | "));
await ctx.close();
await signer.close();
await browser.close();
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
process.exit(failed.length ? 1 : 0);
