// Slice-1c: boot under a slow / lossy REAL-ish remote signer (mock NIP-46 signer with latency + drops) against the local relay (:3100).
// Asserts the app reaches the channel list in <15s with 1.5s/request latency, doesn't hang when decrypts are dropped,
// shows the debug badge + stuck-boot hint, and that signer calls are deduped/capped.
// Usage: node scripts/pwa/s1c-signer-load-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { startMockSigner } from "./mock-nip46-signer.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();

async function boot(signerOpts, { url = APP, waitMs = 40000 } = {}) {
  const signer = await startMockSigner({ userSkHex: TYLER, ...signerOpts });
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(
    ({ relay, pk }) => {
      localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
      localStorage.setItem("buzz-active-community-id", "c1");
    },
    { relay: "ws://localhost:3100", pk: signer.userPk },
  );
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e).slice(0, 200)));
  await page.goto(url);
  await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', { timeout: 20000 });
  await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
  const t0 = Date.now();
  await page.click('[data-testid="web-unlock-bunker-go"]');
  const channels = await page
    .waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: waitMs })
    .then(() => true)
    .catch(() => false);
  return { signer, ctx, page, ms: Date.now() - t0, channels, errors };
}
const counts = (signer) => {
  const c = {};
  for (const r of signer.log.timeline) c[r.method] = (c[r.method] ?? 0) + 1;
  return c;
};

// A. 1.5s latency per signer request
{
  const r = await boot({ latencyMs: 1500 }, { url: APP + (APP.includes("?") ? "&" : "?") + "debug=1" });
  check("A1 1.5s/request signer: channel list within 15s of connect", r.channels && r.ms < 15000, `${r.ms}ms`);
  await r.page.waitForTimeout(4000);
  const c = counts(r.signer);
  console.log("   signer requests:", JSON.stringify(c));
  check("A2 decrypt requests deduped/cached (unique only; far fewer than the raw ~60)", (c.nip44_decrypt ?? 0) <= 30, `${c.nip44_decrypt ?? 0} decrypts`);
  check("A3 debug badge visible with ?debug=1", await r.page.$('[data-testid="signer-debug-badge"]'));
  await r.page.click('[data-testid="signer-debug-badge"]').catch(() => {});
  const txt = await r.page.locator('[data-testid="signer-debug-badge"]').innerText().catch(() => "");
  check("A4 badge lists calls with labels", /pending/.test(txt) && /(authoriz|decrypting|saving|presence|authenticating)/.test(txt), txt.split("\n").slice(0, 2).join(" | "));
  const maxInflight = await r.page.evaluate(() => {
    const calls = window.__buzzSignerCalls.get().filter((x) => x.ms !== null && x.status === "ok");
    const ev = [];
    for (const x of calls) { ev.push([x.startedAt, 1], [x.startedAt + x.ms, -1]); }
    ev.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    let cur = 0, mx = 0;
    for (const [, d] of ev) { cur += d; mx = Math.max(mx, cur); }
    return mx;
  });
  check("A5 in-flight signer requests capped (<=4)", maxInflight <= 4, `max ${maxInflight}`);
  // reload: decrypts come from sessionStorage cache, no repeat decrypt round trips
  const before = counts(r.signer).nip44_decrypt ?? 0;
  await r.page.reload();
  await r.page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 25000 }).catch(() => {});
  await r.page.waitForTimeout(3000);
  const after = counts(r.signer).nip44_decrypt ?? 0;
  check("A6 reload reuses cached decrypts (sessionStorage)", after - before <= 3, `+${after - before} decrypts`);
  await r.ctx.close();
  await r.signer.close();
}

// B. 500ms latency + 20% of decrypt/encrypt requests silently dropped
{
  const r = await boot({ latencyMs: 500, dropRate: 0.2 });
  check("B1 20% decrypts dropped: channel list within 15s", r.channels && r.ms < 15000, `${r.ms}ms dropped=${r.signer.log.dropped ?? 0}`);
  const splashGone = await r.page.waitForSelector('[data-testid="app-loading-gate"]', { state: "detached", timeout: 8000 }).then(() => true).catch(() => false);
  check("B2 splash goes away", splashGone);
  await r.ctx.close();
  await r.signer.close();
}

// C. every decrypt dropped: app still boots (decrypts are never on the boot path)
{
  const r = await boot({ latencyMs: 300, dropRate: 1 });
  check("C1 ALL decrypts dropped: channel list within 15s", r.channels && r.ms < 15000, `${r.ms}ms dropped=${r.signer.log.dropped ?? 0}`);
  await r.ctx.close();
  await r.signer.close();
}

// D. signer stops answering signatures right after sign-in: no silent hang -> hint + Sign out
{
  const r = await boot({ dropRate: 1, dropMethods: ["sign_event", "nip44_decrypt"] }, { waitMs: 3000 });
  const hint = await r.page.waitForSelector('[data-testid="boot-stuck-hint"]', { timeout: 20000 }).catch(() => null);
  check("D1 stuck boot shows 'Still starting… waiting for signer' hint", hint, hint ? (await hint.innerText()).replace(/\n/g, " | ") : "no hint");
  check("D2 'Waiting for your signer…' toast names the action", /authenticating with relay|signing|authorizing/.test(await r.page.locator('[data-testid="signer-waiting"]').innerText({ timeout: 4000 }).catch(() => "")));
  check("D4 stalled lookup never shows first-run 'create profile' onboarding", !(await r.page.evaluate(() => /enter your name|your name|display name|get started/i.test(document.body.innerText) && !document.querySelector('[data-testid="web-unlock"]'))));
  await r.page.click('[data-testid="boot-stuck-signout"]').catch(() => {});
  check("D3 Sign out from the splash returns to the unlock screen", await r.page.waitForSelector('[data-testid="web-unlock"]', { timeout: 15000 }).catch(() => null));
  await r.ctx.close();
  await r.signer.close();
}

await browser.close();
const bad = results.filter((x) => !x.ok);
console.log(`\n${results.length - bad.length}/${results.length} checks passed`);
process.exit(bad.length ? 1 : 0);
