// Slice-1b end-to-end: NIP-46 remote signer (bunker:// + nostrconnect://) against the REAL local buzz-relay (:3100)
// with an in-process mock remote signer. No browser extension is installed in any context here.
// Usage: node scripts/pwa/s1b-nip46-e2e.mjs [appUrl]
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

// ── 1. no extension, bad bunker inputs give clear errors ───────────────────────────────
await page.goto(APP);
await page.waitForSelector('[data-testid="web-unlock-bunker-input"]', {
  timeout: 15000,
});
check(
  "1a NIP-46 panel is usable with no extension present",
  await page.$eval(
    '[data-testid="web-unlock-bunker-input"]',
    (i) => !i.disabled,
  ),
);
await page.fill(
  '[data-testid="web-unlock-bunker-input"]',
  "https://example.com",
);
await page.click('[data-testid="web-unlock-bunker-go"]');
check(
  "1b non-bunker URL → clear error, no hang",
  /bunker:\/\//.test(
    await page
      .locator('[data-testid="web-unlock-nip46-error"]')
      .innerText({ timeout: 5000 })
      .catch(() => ""),
  ),
);
await page.fill(
  '[data-testid="web-unlock-bunker-input"]',
  signer.bunkerUrl().replace(/secret=[^&]+/, "secret=WRONG"),
);
await page.click('[data-testid="web-unlock-bunker-go"]');
check(
  "1c wrong secret → signer's rejection shown",
  /invalid secret/.test(
    await page
      .locator('[data-testid="web-unlock-nip46-error"]')
      .innerText({ timeout: 10000 })
      .catch(() => ""),
  ),
);

// ── 2. bunker:// flow end to end ───────────────────────────────────────────────────────
const seed = `nip46-seed-${stamp}`;
await other.publish(9, [["h", generalId]], seed);
await page.fill('[data-testid="web-unlock-bunker-input"]', signer.bunkerUrl());
await page.click('[data-testid="web-unlock-bunker-go"]');
check(
  "2a bunker connect + get_public_key → app boots, channels load",
  await sidebar(page),
);
await page.waitForTimeout(1500);
const authFrame = wsFrames.find(
  ([d, p]) => d === ">" && p.startsWith('["AUTH"'),
);
const authOk = wsFrames.some(
  ([d, p]) => d === "<" && p.startsWith('["OK"') && p.includes(",true"),
);
check(
  "2b NIP-42 AUTH (22242) to local relay signed by REMOTE signer & accepted",
  authFrame &&
    JSON.parse(authFrame[1])[1].pubkey === pk &&
    authOk &&
    signer.log.signedKinds.includes(22242),
);
check(
  "2c signer saw connect + get_public_key + sign_event",
  ["connect", "get_public_key", "sign_event"].every((m) =>
    signer.log.requests.includes(m),
  ),
  signer.log.requests.join(","),
);
await page
  .locator('[data-testid="sidebar-channel-content"]')
  .getByText("general", { exact: true })
  .first()
  .click();
check(
  "2d channel history loads (REST/NIP-98 path via remote signer)",
  await page
    .waitForSelector(`text=${seed}`, { timeout: 20000 })
    .catch(() => null),
);
const sendText = `nip46-send-${stamp}`;
await page.locator('[data-testid="message-input"]').first().click();
await page.keyboard.type(sendText);
await page.keyboard.press("Enter");
check(
  "2e composer message rendered",
  await page
    .waitForSelector(`text=${sendText}`, { timeout: 20000 })
    .catch(() => null),
);
await page.waitForTimeout(1000);
const mine = (
  await other.query([
    { kinds: [9], "#h": [generalId], authors: [pk], limit: 20 },
  ])
).find((e) => e.content === sendText);
check(
  "2f message is on the relay, authored+signed via remote signer",
  mine && mine.pubkey === pk && mine.sig && signer.log.signedKinds.includes(9),
);
const rest = await inv(page, "send_channel_message", {
  channelId: generalId,
  content: `nip46-rest-${stamp}`,
});
check(
  "2g REST send (NIP-98 27235) signed remotely",
  rest?.event_id && signer.log.signedKinds.includes(27235),
  [...new Set(signer.log.signedKinds)].join(","),
);
const enc = await inv(page, "nip44_encrypt_to_self", { plaintext: "hello" });
check(
  "2h nip44 encrypt/decrypt round-trip via remote signer",
  (await inv(page, "nip44_decrypt_from_self", { ciphertext: enc })) === "hello",
);

// ── 3. storage: only the client key + public info, never the user's nsec ───────────────
const sess = await readIdb(page);
const ls = await page.evaluate(() => JSON.stringify({ ...localStorage }));
check(
  "3a IndexedDB holds {kind,clientSecret,remotePubkey,relays,userPubkey}",
  sess &&
    sess.kind === "nip46" &&
    sess.remotePubkey === signer.signerPk &&
    sess.userPubkey === pk &&
    sess.relays[0] === signer.url &&
    /^[0-9a-f]{64}$/.test(sess.clientSecret),
);
check(
  "3b stored client key ≠ user's identity key; user nsec nowhere in page storage",
  sess.clientSecret !== TYLER &&
    !ls.includes(TYLER) &&
    !JSON.stringify(sess).includes(TYLER) &&
    !ls.includes(sess.clientSecret),
);

// ── 4. reconnect after the signer relay socket dies (iOS background suspension analogue) ─
signer.dropAllSockets();
await page.waitForTimeout(500);
const t0 = Date.now();
const r4 = await inv(page, "send_channel_message", {
  channelId: generalId,
  content: `nip46-after-drop-${stamp}`,
}).catch((e) => ({ err: String(e) }));
check(
  "4 signer-relay disconnect → client reconnects and signing still works",
  r4?.event_id,
  `${Date.now() - t0}ms ${r4?.err ?? ""}`,
);

// ── 5. reload resumes without unlock ───────────────────────────────────────────────────
await page.reload();
check(
  "5a reload resumes NIP-46 session (no unlock screen)",
  (await page
    .waitForSelector("text=general", { timeout: 25000 })
    .catch(() => null)) && !(await page.$('[data-testid="web-unlock"]')),
);
const r5 = await inv(page, "send_channel_message", {
  channelId: generalId,
  content: `nip46-after-reload-${stamp}`,
}).catch((e) => ({ err: String(e) }));
check("5b signing works after resume", r5?.event_id, r5?.err ?? "");

// ── 6. signer timeout → visible waiting state, then an error (not a hang) ──────────────
signer.state.silent = true;
const t6 = Date.now();
const p6b = page.evaluate(
  (id) =>
    window
      .__buzzWebInvoke("send_channel_message", {
        channelId: id,
        content: "timeout-probe",
      })
      .then(
        () => "ok",
        (e) => String(e?.message ?? e),
      ),
  generalId,
);
check(
  "6a 'Waiting for your signer…' indicator visible",
  await page
    .waitForSelector('[data-testid="signer-waiting"]', { timeout: 5000 })
    .catch(() => null),
);
check(
  "6b UI thread stays responsive while waiting",
  (await page.evaluate(() => 1 + 1)) === 2,
);
const r6 = await p6b;
check(
  "6c timeout surfaces an error (not a hang)",
  /didn't respond|Waiting|timed out|signer/i.test(r6) &&
    Date.now() - t6 < 45000,
  `${Date.now() - t6}ms: ${r6.slice(0, 120)}`,
);
console.log("dropped while silent:", signer.log.dropped);
check(
  "6d indicator clears once all outstanding requests have timed out",
  await page
    .waitForSelector('[data-testid="signer-waiting"]', {
      state: "detached",
      timeout: 100000,
    })
    .then(() => true)
    .catch(() => false),
);
signer.state.silent = false;

// ── 7. forget wipes everything ─────────────────────────────────────────────────────────
await inv(page, "sign_out", {}).catch(() => {});
await page
  .waitForSelector('[data-testid="web-unlock"]', { timeout: 15000 })
  .catch(() => {});
check(
  "7a sign out → unlock screen",
  await page.$('[data-testid="web-unlock"]'),
);
check("7b IndexedDB session wiped", (await readIdb(page)) === null);
check(
  "7c localStorage session wiped",
  (await page.evaluate(() => localStorage.getItem("buzz-web-session"))) ===
    null,
);
check(
  "7d no uncaught page errors (flows 1-7)",
  pageErrors.length === 0,
  pageErrors.slice(0, 3).join(" | "),
);
await ctx.close();

// ── 8. nostrconnect:// (QR + deep link) with the mock signer "scanning" the URI ────────
{
  const c2 = await newCtx(pk);
  const p2 = await c2.newPage();
  await p2.goto(`${APP}?nip46relay=${encodeURIComponent(signer.url)}`);
  await p2.waitForSelector('[data-testid="web-unlock-nostrconnect"]', {
    timeout: 15000,
  });
  await p2.click('[data-testid="web-unlock-nostrconnect"]');
  await p2.waitForSelector('[data-testid="web-unlock-nostrconnect-panel"]', {
    timeout: 5000,
  });
  const uri = await p2.inputValue(
    '[data-testid="web-unlock-nostrconnect-uri"]',
  );
  const href = await p2.getAttribute(
    '[data-testid="web-unlock-open-signer"]',
    "href",
  );
  const u = new URL(uri);
  check(
    "8a nostrconnect URI well-formed (client pubkey, relay, secret, name, perms)",
    u.protocol === "nostrconnect:" &&
      /^[0-9a-f]{64}$/.test(u.hostname) &&
      u.searchParams.get("relay") === signer.url &&
      u.searchParams.get("secret") &&
      u.searchParams.get("name") === "Buzz" &&
      /sign_event/.test(u.searchParams.get("perms")),
  );
  check(
    "8b QR rendered and deep-link button points at the same URI",
    (await p2.$('[data-testid="web-unlock-nostrconnect-panel"] svg')) &&
      href === uri,
  );
  await p2.screenshot({
    path: "/Users/jm/Documents/buzz-pwa/pwa-shots/nip46-qr.png",
  });
  check(
    "8c user key not derivable from URI (client key ≠ user key)",
    u.hostname !== pk,
  );
  await new Promise((r) => setTimeout(r, 800)); // let the client subscribe on the signer relay
  await signer.scan(uri);
  check(
    "8d signer 'scans' URI → connect response → signed in, channels load",
    await sidebar(p2),
  );
  const s2 = await readIdb(p2);
  check(
    "8e session persisted with signer's pubkey from the connect response",
    s2 && s2.remotePubkey === signer.signerPk && s2.userPubkey === pk,
  );
  await c2.close();
}

// ── 9. auth_url challenge is opened ────────────────────────────────────────────────────
{
  const s9 = await startMockSigner({
    userSkHex: TYLER,
    authUrl: "http://127.0.0.1:59999/approve",
  });
  const c9 = await newCtx(pk);
  const hits = [];
  await c9.route("**/approve", (r) => {
    hits.push(r.request().url());
    return r.fulfill({ body: "approve" });
  });
  const p9 = await c9.newPage();
  await p9.goto(APP);
  await p9.waitForSelector('[data-testid="web-unlock-bunker-input"]');
  await p9.fill('[data-testid="web-unlock-bunker-input"]', s9.bunkerUrl());
  await p9.click('[data-testid="web-unlock-bunker-go"]');
  check(
    "9a auth_url challenge opened in a new tab",
    await new Promise((res) => {
      const t = setInterval(
        () => hits.length && (clearInterval(t), res(true)),
        100,
      );
      setTimeout(() => {
        clearInterval(t);
        res(false);
      }, 8000);
    }),
    hits.join(","),
  );
  check("9b connect completes after approval", await sidebar(p9));
  await c9.close();
  await s9.close();
}

// ── 10. NIP-04-only signer fallback ────────────────────────────────────────────────────
{
  const s10 = await startMockSigner({ userSkHex: TYLER, useNip04: true });
  const c10 = await newCtx(pk);
  const p10 = await c10.newPage();
  await p10.goto(APP);
  await p10.waitForSelector('[data-testid="web-unlock-bunker-input"]');
  await p10.fill('[data-testid="web-unlock-bunker-input"]', s10.bunkerUrl());
  await p10.click('[data-testid="web-unlock-bunker-go"]');
  const ok10 = await sidebar(p10);
  // Mock signer replies NIP-04 (and can read both); proves the client decrypts NIP-04 responses and keeps going.
  check("10 signer replying in NIP-04 is understood", ok10);
  await c10.close();
  await s10.close();
}

// ── 11. REAL-DEVICE SEQUENCE: reload (new document) mid-handshake, signer answers while page was gone ──
const readPending = (page) =>
  page.evaluate(
    () =>
      new Promise((res) => {
        const ls = localStorage.getItem("buzz-nip46-pending");
        const r = indexedDB.open("buzz-web-signer", 1);
        r.onupgradeneeded = () => r.result.createObjectStore("kv");
        r.onsuccess = () => {
          const g = r.result.transaction("kv").objectStore("kv").get("nip46-pending");
          g.onsuccess = () => {
            r.result.close();
            res({ ls: !!ls, idb: !!g.result });
          };
          g.onerror = () => res({ ls: !!ls, idb: false });
        };
        r.onerror = () => res({ ls: !!ls, idb: false });
      }),
  );
const openConnect = async (p, qs = "") => {
  await p.goto(`${APP}?nip46relay=${encodeURIComponent(signer.url)}${qs}`);
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect"]', { timeout: 15000 });
  await p.click('[data-testid="web-unlock-nostrconnect"]');
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect-uri"]', { timeout: 5000 });
  return p.inputValue('[data-testid="web-unlock-nostrconnect-uri"]');
};
{
  const c = await newCtx(pk);
  const p = await c.newPage();
  const logs = [];
  p.on("console", (m) => m.text().startsWith("[nip46]") && logs.push(m.text()));
  const uri1 = await openConnect(p, "&debug=1");
  const pend1 = await readPending(p);
  check("11a pending attempt persisted (localStorage + IndexedDB) before the link is used", pend1.ls && pend1.idb, JSON.stringify(pend1));
  await p.reload(); // iOS discards/reloads the tab while the user is in Primal
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect-uri"]', { timeout: 15000 });
  const uri2 = await p.inputValue('[data-testid="web-unlock-nostrconnect-uri"]');
  check("11b reload re-uses the SAME keypair/secret/URI (no new attempt)", uri1 === uri2, uri2.slice(0, 40));
  check(
    "11c unlock screen shows 'Finishing sign-in…' + Cancel (not a blank sign-in)",
    /Finishing sign-in/.test(await p.locator('[data-testid="web-unlock-finishing"]').innerText()) &&
      (await p.$('[data-testid="web-unlock-cancel"]')),
  );
  check("11d ?debug=1 shows relay/last-event line", /signer relays: connected/.test(await p.locator('[data-testid="web-unlock-debug"]').innerText()));
  await new Promise((r) => setTimeout(r, 800));
  await signer.scan(uri2); // signer answers; page takes no user action
  check("11e signer answers after reload → signed in with NO tap, channels load", await sidebar(p));
  await p.waitForTimeout(500);
  const pend2 = await readPending(p);
  check("11f pending attempt cleared on success", !pend2.ls && !pend2.idb, JSON.stringify(pend2));
  check("11g logs show resume + connect + get_public_key, no secrets", logs.some((l) => /RESUMING/.test(l)) && logs.some((l) => /connect response ok/.test(l)) && logs.some((l) => /get_public_key result/.test(l)) && !logs.some((l) => new RegExp(new URL(uri2).searchParams.get("secret")).test(l)), logs.length + " lines");
  await c.close();
}
// signer answers BEFORE the reloaded page exists at all (event only sits on the relay) → fetched via `since`
{
  const c = await newCtx(pk);
  const p = await c.newPage();
  const uri = await openConnect(p);
  await p.goto("about:blank"); // page gone entirely (same context/storage)
  await signer.scan(uri);
  await p.goto(`${APP}?nip46relay=${encodeURIComponent(signer.url)}`);
  check("11h answer that arrived while no page was open is picked up on next load (since window)", await sidebar(p));
  await c.close();
}
// Cancel clears the attempt; a fresh click mints a NEW uri
{
  const c = await newCtx(pk);
  const p = await c.newPage();
  const u1 = await openConnect(p);
  await p.click('[data-testid="web-unlock-cancel"]');
  const pe = await readPending(p);
  check("11i Cancel clears the pending attempt", !pe.ls && !pe.idb);
  await p.click('[data-testid="web-unlock-nostrconnect"]');
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect-uri"]');
  check("11j after Cancel a new attempt gets a new URI", u1 !== (await p.inputValue('[data-testid="web-unlock-nostrconnect-uri"]')));
  await c.close();
}

// ── 12. answer while page is hidden/suspended (sockets dead), then pageshow/visibilitychange ──
{
  const c = await newCtx(pk);
  const p = await c.newPage();
  const uri = await openConnect(p);
  await p.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
  });
  await new Promise((r) => setTimeout(r, 500));
  signer.dropAllSockets(); // iOS kills sockets in background
  await signer.scan(uri); // answer lands while suspended
  await p.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("pageshow"));
  });
  check("12a answer while hidden, then visibilitychange/pageshow → signed in without a tap", await sidebar(p));
  await c.close();
}

// ── 13. robustness: multi relay (one dead), 'ack' result, empty result ──
for (const [label, result] of [["ack", "ack"], ["empty result (verified via get_public_key)", ""]]) {
  const c = await newCtx(pk);
  const p = await c.newPage();
  await p.goto(`${APP}?nip46relay=${encodeURIComponent(signer.url + ", ws://127.0.0.1:1")}`);
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect"]');
  await p.click('[data-testid="web-unlock-nostrconnect"]');
  await p.waitForSelector('[data-testid="web-unlock-nostrconnect-uri"]');
  const uri = await p.inputValue('[data-testid="web-unlock-nostrconnect-uri"]');
  check(`13 URI carries every relay (${label})`, new URL(uri).searchParams.getAll("relay").length === 2);
  await new Promise((r) => setTimeout(r, 800));
  await signer.scan(uri, { result });
  check(`13 connect with result=${label} works even with a dead 2nd relay`, await sidebar(p));
  await c.close();
}

await signer.close();
await browser.close();
const failed = results.filter((r) => !r.ok);
console.log(
  `\n${results.length - failed.length}/${results.length} checks passed`,
);
process.exit(failed.length ? 1 : 0);
