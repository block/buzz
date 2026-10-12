// Slice-1d: real-device performance under a SLOW remote signer (Primal-iOS-like: ~2s +jitter per request) against the LOCAL relay (:3100 only).
// Measures per-action signer call breakdown (boot / idle / open channel / send) and asserts latency budgets + send robustness.
// NEVER touches the production relay. Usage: node scripts/pwa/s1d-perf-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { createHash } from "node:crypto";
import { startMockSigner } from "./mock-nip46-signer.mjs";
import { client } from "./relay-client.mjs";
import { getPublicKey, nip44 } from "nostr-tools";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100";
const RELAY_HTTP = "http://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
function uuid5(ns, name) {
  const h = createHash("sha1").update(Buffer.concat([Buffer.from(ns.replace(/-/g, ""), "hex"), Buffer.from(name)])).digest();
  h[6] = (h[6] & 0x0f) | 0x50;
  h[8] = (h[8] & 0x3f) | 0x80;
  const x = h.subarray(0, 16).toString("hex");
  return `${x.slice(0, 8)}-${x.slice(8, 12)}-${x.slice(12, 16)}-${x.slice(16, 20)}-${x.slice(20)}`;
}
const generalId = uuid5("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "buzz.channel.general");
const stamp = Date.now();
const other = client(RELAY_HTTP, TYLER);
const seedText = `seed-d-${stamp}`;
await other.publish(9, [["h", generalId]], seedText);
// Realistic account state: ~30 encrypted self-reminders (kind 30300, deterministic d-tags so reruns replace instead of accumulate).
const skBytes = new Uint8Array(Buffer.from(TYLER, "hex"));
const myPk = getPublicKey(skBytes);
const selfKey = nip44.v2.utils.getConversationKey(skBytes, myPk);
const selfEnc = (t) => nip44.v2.encrypt(t, selfKey);
for (let i = 0; i < 30; i++)
  await other
    .publish(30300, [["d", `perf-rem-${i}`], ["not_before", String(Math.floor(Date.now() / 1000) + 86400)]], selfEnc(JSON.stringify({ target: { eventId: "a".repeat(64), channelId: generalId, preview: `reminder ${i}`, authorPubkey: myPk }, status: "pending" })))
    .catch((e) => (i === 0 ? console.log("   (could not seed reminders:", String(e).slice(0, 80), ")") : 0));

const browser = await chromium.launch();
const breakdown = (signer, from, to = Infinity) => {
  const c = {};
  for (const r of signer.log.timeline) {
    if (r.t < from || r.t >= to) continue;
    const k = r.method + (r.kind !== undefined && r.kind !== null ? `:${r.kind}` : "");
    c[k] = (c[k] ?? 0) + 1;
  }
  return c;
};
const total = (c) => Object.values(c).reduce((a, b) => a + b, 0);

async function boot(signerOpts, { debug = true } = {}) {
  const signer = await startMockSigner({ userSkHex: TYLER, ...signerOpts });
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(
    ({ relay, pk }) => {
      localStorage.setItem("buzz-communities", JSON.stringify([{ id: "c1", name: "Local Dev", relayUrl: relay, pubkey: pk, addedAt: new Date().toISOString() }]));
      localStorage.setItem("buzz-active-community-id", "c1");
    },
    { relay: RELAY_WS, pk: signer.userPk },
  );
  const page = await ctx.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e).slice(0, 200)));
  const prodHits = [];
  page.on("request", (r) => { if (/huladesk/.test(r.url())) prodHits.push(r.url()); });
  await page.goto(APP + (debug ? (APP.includes("?") ? "&" : "?") + "debug=1" : ""));
  await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', { timeout: 20000 });
  await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
  const t0 = Date.now();
  await page.click('[data-testid="web-unlock-bunker-go"]');
  const channels = await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 40000 }).then(() => true).catch(() => false);
  return { signer, ctx, page, t0, ms: Date.now() - t0, channels, errors, prodHits };
}
const LAT = { latencyMs: 2000, jitterMs: 800 };
const openGeneral = (page) =>
  page.locator('[data-testid="sidebar-channel-content"]').getByText("general", { exact: true }).first().click();
const sendMsg = async (page, text) => {
  const input = page.locator('[data-testid="message-input"]').first();
  await input.click();
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
};


const waitSentResolved = (page, text, timeout) =>
  page.waitForFunction((t) => {
    const rows = [...document.querySelectorAll('[data-testid="message-row"], [data-message-id]')];
    const row = rows.find((x) => x.textContent?.includes(t));
    return !!row && !row.querySelector('[data-testid="message-send-status"]') && !row.querySelector('[data-testid="message-send-failed"]');
  }, text, { timeout }).then(() => true).catch(() => false);
const relayHas = async (text) => (await other.query([{ kinds: [9], "#h": [generalId], authors: [myPk], limit: 50 }])).filter((e) => e.content === text);
const inv = (page, c, a) => page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [c, a]);

// A. realistic latency (2s ± 0.8s per request, parallel signer): journey boot → idle → open channel → send
{
  const r = await boot(LAT);
  const { signer, page } = r;
  const shownAt = r.t0 + r.ms;
  const bootCalls = breakdown(signer, r.t0, shownAt);
  const bootCallsStrict = breakdown(signer, r.t0, shownAt - 1500); // 1.5s tolerance: the list is detected by polling
  console.log(`   [boot→channel list] ${r.ms}ms signer requests=${total(bootCalls)}`, JSON.stringify(bootCalls));
  check("A1 channel list within 10s of connect (2s±0.8s signer)", r.channels && r.ms < 10000, `${r.ms}ms`);
  check("A1b only minimal signer calls before the channel list (no decrypts/presence)", !bootCallsStrict.nip44_decrypt && !bootCallsStrict["sign_event:20001"], JSON.stringify(bootCalls));
  await page.waitForTimeout(14000);
  const idleCalls = breakdown(signer, shownAt);
  console.log(`   [14s after list] signer requests=${total(idleCalls)}`, JSON.stringify(idleCalls));
  const early = signer.log.timeline.filter((x) => x.kind === 20001 && x.t < shownAt - 1500).length;
  check("A1c presence is never signed before the first screen", early === 0, `${early} early`);

  const tOpen = Date.now();
  await openGeneral(page);
  const seedVisible = await page.waitForSelector(`text=${seedText}`, { timeout: 25000 }).then(() => true).catch(() => false);
  const openMs = Date.now() - tOpen;
  const openCalls = breakdown(signer, tOpen);
  console.log(`   [open #general] ${openMs}ms signer requests=${total(openCalls)}`, JSON.stringify(openCalls));
  check("A2 opening a channel shows its messages within 8s", seedVisible && openMs < 8000, `${openMs}ms`);

  const sendText = `web-send-d-${stamp}`;
  const tSend = Date.now();
  await sendMsg(page, sendText);
  const echoed = await page.waitForSelector(`text=${sendText}`, { timeout: 25000 }).then(() => true).catch(() => false);
  const echoMs = Date.now() - tSend;
  const sentOk = await waitSentResolved(page, sendText, 25000);
  const sendMs = Date.now() - tSend;
  const sendCalls = breakdown(signer, tSend);
  console.log(`   [send] echoed=${echoed} in ${echoMs}ms resolved=${sentOk} ${sendMs}ms signer requests=${total(sendCalls)}`, JSON.stringify(sendCalls));
  check("A3 own message shown immediately (optimistic) and resolves within 10s, exactly once on the local relay", echoed && echoMs < 1500 && sentOk && sendMs < 10000 && (await relayHas(sendText)).length === 1, `echo ${echoMs}ms, resolved ${sendMs}ms`);
  const k9 = signer.log.timeline.find((x) => x.kind === 9 && x.t >= tSend);
  check("A4 the kind-9 sign request reached the signer within 3.5s of Send (not queued behind presence/decrypts)", k9 && k9.t - tSend < 3500, `${k9 ? k9.t - tSend : "never"}ms`);
  const pres = signer.log.timeline.filter((x) => x.kind === 20001).length;
  check("A5 presence is throttled (<= 2 signs in this session)", pres <= 2, `${pres}`);
  await page.click('[data-testid="signer-debug-badge"]').catch(() => {});
  const badge = await page.locator('[data-testid="signer-debug-badge"]').innerText().catch(() => "");
  console.log("   badge:", badge.replace(/\n/g, " | ").slice(0, 400));
  check("A6 debug badge shows p50/p95, queue depth by class and oldest pending", /p50/.test(badge) && /p95/.test(badge) && /queue \(queued\/in-flight\)/.test(badge) && /oldest:/.test(badge));
  check("A7 no production-relay traffic", r.prodHits.length === 0, r.prodHits.join(","));
  // persisted decrypt cache: wipe sessionStorage (new launch) → decrypts come from localStorage
  const lsKeys = await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith("buzz-sig-l:")).length);
  await page.waitForFunction(() => /settings 0\/0/.test(document.querySelector('[data-testid="signer-debug-badge"]')?.textContent || ''), null, { timeout: 90000 }).catch(() => {});
  const lsKeys2 = await page.evaluate(() => Object.keys(localStorage).filter((k) => k.startsWith("buzz-sig-l:")).length);
  const decBefore = (breakdown(signer, 0).nip44_decrypt ?? 0);
  await page.evaluate(() => sessionStorage.clear());
  await page.reload();
  await page.waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', { timeout: 30000 }).catch(() => {});
  await page.waitForTimeout(16000);
  const decAfter = (breakdown(signer, 0).nip44_decrypt ?? 0);
  check("A8 decrypt cache persists in localStorage across relaunch (no re-decrypt)", lsKeys2 > 0 && decAfter - decBefore <= 3, `${lsKeys2} cached, +${decAfter - decBefore} decrypts after relaunch`);
  await r.ctx.close();
  await signer.close();
}

// B. serial signer (one request at a time, 2s each) + a flood of presence / decrypt / typing requests right before Send
{
  const r = await boot({ latencyMs: 2000, jitterMs: 500, serial: true });
  const { signer, page } = r;
  check("B1 channel list within 12s with a SERIAL 2s signer", r.channels && r.ms < 12000, `${r.ms}ms`);
  await page.waitForTimeout(16000);
  await openGeneral(page);
  await page.waitForSelector(`text=${seedText}`, { timeout: 25000 }).catch(() => {});
  // flood: 40 presence signs (alternating), 40 distinct valid decrypts, 10 typing indicators
  const cts = Array.from({ length: 40 }, (_, i) => selfEnc(`flood-${i}-${stamp}`));
  await page.evaluate(([cts]) => {
    for (let i = 0; i < 40; i++) {
      window.__buzzWebInvoke("sign_event", { kind: 20001, content: i % 2 ? "away" : "online", tags: [] }).catch(() => {});
      window.__buzzWebInvoke("nip44_decrypt_from_self", { ciphertext: cts[i] }).catch(() => {});
    }
    for (let i = 0; i < 10; i++) window.__buzzWebInvoke("sign_event", { kind: 20002, content: "", tags: [["h", "x"]] }).catch(() => {});
  }, [cts]);
  const sendText = `web-send-flood-${stamp}`;
  const tSend = Date.now();
  await sendMsg(page, sendText);
  const sentOk = await waitSentResolved(page, sendText, 30000);
  const sendMs = Date.now() - tSend;
  const calls = breakdown(signer, tSend);
  console.log(`   [send during flood] resolved=${sentOk} ${sendMs}ms signer requests=${total(calls)}`, JSON.stringify(calls));
  check("B2 send resolves within 12s even with a flood of presence/decrypt/typing requests queued", sentOk && sendMs < 12000 && (await relayHas(sendText)).length === 1, `${sendMs}ms`);
  const dropped = await page.evaluate(() => window.__buzzSignerCalls.get().filter((c) => c.status === "dropped").length);
  check("B3 presence/typing flood is coalesced/dropped, not all signed", (calls["sign_event:20001"] ?? 0) + (calls["sign_event:20002"] ?? 0) <= 4 && dropped >= 5, `${(calls["sign_event:20001"] ?? 0) + (calls["sign_event:20002"] ?? 0)} signed, ${dropped} dropped`);
  await r.ctx.close();
  await signer.close();
}

// C. signer silently drops the kind-9 sign request: row ends "Failed — Retry" (never stuck "Sending…"); Retry succeeds, posted once
{
  const r = await boot({ latencyMs: 600 });
  const { signer, page } = r;
  await page.waitForTimeout(3000);
  await openGeneral(page);
  await page.waitForSelector(`text=${seedText}`, { timeout: 20000 }).catch(() => {});
  signer.state.dropSignKinds.set(9, 1000); // drop the request AND its automatic republishes
  const sendText = `web-send-fail-${stamp}`;
  const tSend = Date.now();
  await sendMsg(page, sendText);
  const waitingShown = await page.waitForFunction((t) => [...document.querySelectorAll('[data-testid="message-send-status"]')].some((e) => /Waiting for your signer/.test(e.textContent || "")), null, { timeout: 8000 }).then(() => true).catch(() => false);
  check("C1 pending row says 'Waiting for your signer…' while the signature is outstanding", waitingShown);
  const failed = await page.waitForSelector('[data-testid="message-send-failed"]', { timeout: 50000 }).then(() => true).catch(() => false);
  const failMs = Date.now() - tSend;
  const failTxt = failed ? await page.locator('[data-testid="message-send-failed"]').first().innerText() : "";
  check("C2 dropped signature → 'Failed — <reason>' + Retry within ~40s", failed && failMs < 45000 && /Failed/.test(failTxt) && (await page.$('[data-testid="message-send-retry"]')), `${failMs}ms: ${failTxt.replace(/\n/g, " ")}`);
  check("C3 nothing is stuck in 'Sending…' once failed", !(await page.$('[data-testid="message-send-status"]')));
  check("C4 nothing posted yet", (await relayHas(sendText)).length === 0);
  signer.state.dropSignKinds.set(9, 0); // signer is responsive again
  const tRetry = Date.now();
  await page.click('[data-testid="message-send-retry"]');
  const ok = await waitSentResolved(page, sendText, 25000);
  check("C5 Retry succeeds within 15s and the message is on the relay exactly once", ok && Date.now() - tRetry < 15000 && (await relayHas(sendText)).length === 1, `${Date.now() - tRetry}ms`);
  await r.ctx.close();
  await signer.close();
}

// D. reconnect: the signer-relay sockets die mid-session; a send still goes through
{
  const r = await boot({ latencyMs: 800 });
  const { signer, page } = r;
  await page.waitForTimeout(3000);
  await openGeneral(page);
  await page.waitForSelector(`text=${seedText}`, { timeout: 20000 }).catch(() => {});
  signer.dropAllSockets();
  await page.waitForTimeout(300);
  const sendText = `web-send-reconnect-${stamp}`;
  const tSend = Date.now();
  await sendMsg(page, sendText);
  const ok = await waitSentResolved(page, sendText, 30000);
  check("D1 send after the signer sockets dropped resolves within 20s, once", ok && Date.now() - tSend < 20000 && (await relayHas(sendText)).length === 1, `${Date.now() - tSend}ms`);
  const k9 = signer.log.timeline.filter((x) => x.kind === 9 && x.t >= tSend).length;
  check("D2 the request was delivered to the signer once (no duplicate publishes multiplying load)", k9 === 1, `${k9} kind-9 sign requests`);
  await r.ctx.close();
  await signer.close();
}

await browser.close();
const bad = results.filter((x) => !x.ok);
console.log(`\n${results.length - bad.length}/${results.length} checks passed`);
process.exit(bad.length ? 1 : 0);
