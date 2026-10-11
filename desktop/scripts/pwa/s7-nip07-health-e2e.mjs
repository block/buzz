// NIP-07 (browser extension) health e2e: a slow/unclicked extension must NOT show remote-signer wording, must not escalate
// at the 25 s remote-signer limit, and must recover without a reload once the extension answers. Local relay (:3100) + mock extension.
// Usage: node scripts/pwa/s7-nip07-health-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
async function boot(escalateMs) {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const { pk } = await installMockExtension(ctx, TYLER);
  await ctx.addInitScript(({ relay, pk, escalateMs }) => {
    if (escalateMs) window.__BUZZ_NIP07_ESCALATE_MS = escalateMs;
    if (!localStorage.getItem("buzz-communities")) {
      localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
      localStorage.setItem("buzz-active-community-id", "c1");
    }
  }, { relay: RELAY_WS, pk, escalateMs });
  const page = await ctx.newPage();
  await page.goto(APP);
  await page.waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 15000 });
  await page.click('[data-testid="web-unlock-nip07"]');
  const ok = await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 }).catch(() => null);
  return { ctx, page, ok };
}
const poke = (page) => page.evaluate(() => window.__buzzWebInvoke("send_channel_message", { channelId: "00000000-0000-0000-0000-000000000000", content: "nip07-probe" }).then(() => "ok", (e) => String(e?.message ?? e)));

// A. stalled extension, short test limit: extension copy, Retry only, auto/Retry recovery without reload
{
  const { page, ok } = await boot(6000);
  check("A0 signed in with NIP-07, app boots", ok);
  await page.evaluate(() => { window.__nostrStall = true; });
  const pending = poke(page);
  const banner = await page.waitForSelector('[data-testid="signer-down-banner"]', { timeout: 40000 }).catch(() => null);
  check("A1 stalled extension -> banner appears", banner);
  const t = banner ? await banner.innerText() : "";
  check("A2 banner uses extension copy", /browser extension isn't responding/.test(t) && /unlocked and approve any prompts/.test(t), t.replace(/\s+/g, " ").slice(0, 140));
  check("A3 no remote-signer wording (Primal/Amber/nsec.app/signer app)", !/Primal|Amber|nsec\.app|signer app|Reconnect signer|Sign out/.test(t));
  check("A4 Retry only", (await page.$('[data-testid="signer-retry"]')) && !(await page.$('[data-testid="signer-reconnect"]')) && !(await page.$('[data-testid="signer-signout"]')));
  await page.evaluate(() => { window.__nostrStall = false; });
  const gone = await page.waitForSelector('[data-testid="signer-down-banner"]', { state: "detached", timeout: 30000 }).then(() => true, () => false);
  check("A5 extension answers again -> banner clears without reload", gone);
  await pending.catch(() => null);
  check("A6 app still mounted (no reload)", await page.$('[data-testid="sidebar-channel-content"]'));
  await page.context().close();
}

// B. default limits: a 30 s popup wait (past the 25 s remote-signer limit) must not show the banner
{
  const { page, ok } = await boot(0);
  check("B0 signed in with NIP-07", ok);
  await page.evaluate(() => { window.__nostrStall = true; });
  const pending = poke(page);
  await page.waitForTimeout(32000);
  check("B1 no banner after 32 s of an un-clicked extension prompt", !(await page.$('[data-testid="signer-down-banner"]')));
  await page.evaluate(() => { window.__nostrStall = false; });
  await pending.catch(() => null);
  check("B2 request completes after the prompt is approved; still no banner", !(await page.$('[data-testid="signer-down-banner"]')));
  await page.context().close();
}
await browser.close();
const failed = results.filter((r) => !r.ok).length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
