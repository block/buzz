// Slice-1 end-to-end: web app + mock NIP-07 extension + REAL local buzz-relay (http://localhost:3100, ws://localhost:3100).
// Usage: node scripts/pwa/s1-e2e.mjs [appUrl]   (relay + seeded fixture data must be running; see docs/pwa-plan.md)
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
import { client } from "./relay-client.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const RELAY_WS = "ws://localhost:3100",
  RELAY_HTTP = "http://localhost:3100";
const TYLER =
  "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const GENERAL = "4b2a8ed2-b8cd-5aa6-9a44-8c6b3a0a0f0e"; // replaced below by uuid5(dns, buzz.channel.general)
const SHOTS = "/Users/jm/Documents/buzz-pwa/pwa-shots";
const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok, detail });
  console.log(
    `${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`,
  );
};

const { createHash } = await import("node:crypto");
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

const browser = await chromium.launch();
const stamp = Date.now();

// ── A. No extension → clear unlock screen, nothing stored ──────────────────────────────
{
  const ctx = await browser.newContext({
    viewport: { width: 1280, height: 800 },
  });
  const page = await ctx.newPage();
  await page.goto(APP);
  await page
    .waitForSelector('[data-testid="web-unlock-no-extension"]', {
      timeout: 10000,
    })
    .catch(() => {});
  check(
    "A1 no extension: unlock screen says none found",
    await page.$('[data-testid="web-unlock-no-extension"]'),
  );
  check(
    "A2 NIP-46 remote signer option shown and usable",
    await page
      .$eval('[data-testid="web-unlock-bunker-input"]', (i) => !i.disabled)
      .catch(() => false),
  );
  const keys = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => /nsec|secret|private/i.test(k)),
  );
  check(
    "A3 no secret-key material in localStorage",
    keys.length === 0,
    JSON.stringify(keys),
  );
  await page.screenshot({ path: `${SHOTS}/a-no-extension.png` });
  await ctx.close();
}

// ── B. Extension flow against the real local relay ─────────────────────────────────────
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
});
const { pk, log } = await installMockExtension(ctx, TYLER);
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
const page = await ctx.newPage();
const invoked = new Set(),
  wsFrames = [],
  pageErrors = [],
  nativeOnly = new Set(),
  httpAuthed = [];
page.on("pageerror", (e) => pageErrors.push(String(e).slice(0, 200)));
page.on("console", (m) => {
  const t = m.text();
  const nm = t.match(/native-only command: (\S+)/);
  if (nm) nativeOnly.add(nm[1]);
  const im = t.match(/^\[invoke\] (\S+)/);
  if (im) invoked.add(im[1]);
});
page.on("websocket", (ws) => {
  if (!ws.url().includes("localhost:3100")) return;
  ws.on("framesent", (f) => wsFrames.push([">", String(f.payload)]));
  ws.on("framereceived", (f) => wsFrames.push(["<", String(f.payload)]));
});
page.on("request", async (r) => {
  if (
    (r.url().startsWith(RELAY_HTTP) || r.url().includes("/__relay/")) &&
    r.method() !== "OPTIONS"
  ) {
    const h = await r.allHeaders().catch(() => r.headers());
    if (h.authorization?.startsWith("Nostr "))
      httpAuthed.push(
        r.method() +
          " " +
          new URL(r.url()).pathname.replace(/^\/__relay\/[^/]+/, ""),
      );
  }
});

const other = client(RELAY_HTTP, TYLER); // second client (same identity) posts "from another device"
const seedText = `seed-${stamp}`;
await other.publish(9, [["h", generalId]], seedText);

await page.goto(APP);
check(
  "B1 unlock screen detects NIP-07",
  await page
    .waitForSelector('[data-testid="web-unlock-nip07"]', { timeout: 10000 })
    .catch(() => null),
);
await page.click('[data-testid="web-unlock-nip07"]');
await page
  .waitForSelector('[data-testid="sidebar-channel-content"] >> text=general', {
    timeout: 20000,
  })
  .catch(() => {});
await page.waitForTimeout(1500);

const authFrame = wsFrames.find(
  ([d, p]) => d === ">" && p.startsWith('["AUTH"'),
);
const authOk = wsFrames.some(
  ([d, p]) => d === "<" && p.startsWith('["OK"') && p.includes(",true"),
);
check("B2 relay WebSocket connected to relay URL", wsFrames.length > 0);
check(
  "B3 NIP-42 AUTH sent (kind 22242) and accepted",
  authFrame &&
    JSON.parse(authFrame[1])[1].kind === 22242 &&
    JSON.parse(authFrame[1])[1].pubkey === pk &&
    authOk,
);
check(
  "B4 AUTH signed by the extension (not locally)",
  log.signed.includes(22242),
);
check(
  "B5 channel list loaded from relay (general, engineering, DMs in sidebar)",
  /general[\s\S]*engineering|engineering[\s\S]*general/.test(
    await page
      .locator('[data-testid="sidebar-channel-content"]')
      .innerText()
      .catch(() => ""),
  ),
);
await page.screenshot({ path: `${SHOTS}/b-channels.png` });

await page
  .locator('[data-testid="sidebar-channel-content"]')
  .getByText("general", { exact: true })
  .first()
  .click();
const seedVisible = await page
  .waitForSelector(`text=${seedText}`, { timeout: 15000 })
  .catch(() => null);
check(
  "B6 opening #general loads message history (seed message visible)",
  seedVisible,
);
await page.screenshot({ path: `${SHOTS}/b-general.png` });

const liveText = `live-${stamp}`;
await other.publish(9, [["h", generalId]], liveText);
check(
  "B7 live update over websocket (message from other client appears without reload)",
  await page
    .waitForSelector(`text=${liveText}`, { timeout: 15000 })
    .catch(() => null),
);

const sendText = `web-send-${stamp}`;
const input = page.locator('[data-testid="message-input"]').first();
await input.click();
await page.keyboard.type(sendText);
await page.keyboard.press("Enter");
check(
  "B8 sent message renders in timeline",
  await page
    .waitForSelector(`text=${sendText}`, { timeout: 15000 })
    .catch(() => null),
);
await page.waitForTimeout(1000);
const onRelay = await other.query([
  { kinds: [9], "#h": [generalId], authors: [pk], limit: 20 },
]);
const mine = onRelay.find((e) => e.content === sendText);
check(
  "B9 message exists on relay, authored by the extension key",
  mine &&
    mine.pubkey === pk &&
    mine.sig &&
    mine.tags.some((t) => t[0] === "h" && t[1] === generalId),
);
check(
  "B10 extension signed kind 9 and NIP-98 (27235)",
  log.signed.includes(9) && log.signed.includes(27235),
  `signed kinds: ${[...new Set(log.signed)].join(",")}`,
);
check(
  "B10b composer send used the app websocket (EVENT kind 9, extension-signed)",
  wsFrames.some(
    ([d, p]) =>
      d === ">" &&
      p.startsWith('["EVENT"') &&
      JSON.parse(p)[1].kind === 9 &&
      JSON.parse(p)[1].content === sendText,
  ),
);
// Registry path: send_channel_message (REST, NIP-98) — top-level + threaded reply + mention
const inv = (c, a) =>
  page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [c, a]);
const before = log.signed.length;
const topText = `rest-top-${stamp}`;
const r1 = await inv("send_channel_message", {
  channelId: generalId,
  content: topText,
  mentionPubkeys: [
    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
  ],
});
const r2 = await inv("send_channel_message", {
  channelId: generalId,
  content: `rest-reply-${stamp}`,
  parentEventId: r1.event_id,
});
const r3 = await inv("send_channel_message", {
  channelId: generalId,
  content: `rest-reply2-${stamp}`,
  parentEventId: r2.event_id,
});
const evs = await other.query([
  { ids: [r1.event_id, r2.event_id, r3.event_id], limit: 10 },
]);
const e1 = evs.find((e) => e.id === r1.event_id),
  e2 = evs.find((e) => e.id === r2.event_id),
  e3 = evs.find((e) => e.id === r3.event_id);
check(
  "B11 send_channel_message top-level via NIP-98 POST /events (h + p tags, extension-signed)",
  e1 &&
    e1.pubkey === pk &&
    e1.tags.some((t) => t[0] === "h" && t[1] === generalId) &&
    e1.tags.some((t) => t[0] === "p" && t[1].startsWith("953d")) &&
    r1.depth === 0,
);
check(
  "B12 threaded replies carry NIP-10 root/reply tags and correct depth",
  e2 &&
    e3 &&
    e2.tags.some(
      (t) => t[0] === "e" && t[1] === r1.event_id && t[3] === "reply",
    ) &&
    e3.tags.some(
      (t) => t[0] === "e" && t[1] === r1.event_id && t[3] === "root",
    ) &&
    e3.tags.some(
      (t) => t[0] === "e" && t[1] === r2.event_id && t[3] === "reply",
    ) &&
    r2.depth === 1 &&
    r3.depth === 2,
  `depths ${r1.depth},${r2.depth},${r3.depth}`,
);
check(
  "B13 REST sends were authorised with NIP-98 headers",
  httpAuthed.filter((x) => x === "POST /events").length >= 3,
  [...new Set(httpAuthed)].join(" | "),
);
const dm = await inv("get_channel_details", { channelId: generalId });
check(
  "B14 get_channel_details / get_channel_members registry reads",
  dm.name === "general" &&
    (await inv("get_channel_members", { channelId: generalId })).members
      .length >= 3,
);
await page.screenshot({ path: `${SHOTS}/b-sent.png` });

// ── C. Session resume on reload (no unlock click) ──────────────────────────────────────
await page.reload();
check(
  "C1 reload resumes session without unlock screen",
  (await page
    .waitForSelector("text=general", { timeout: 20000 })
    .catch(() => null)) && !(await page.$('[data-testid="web-unlock"]')),
);
const stored = await page.evaluate(() =>
  JSON.stringify(
    Object.fromEntries(
      Object.entries(localStorage).filter(([k]) => k.startsWith("buzz-web-")),
    ),
  ),
);
check(
  "C2 only public session info persisted",
  /pubkey/.test(stored) && !/nsec|secret/i.test(stored),
  stored.slice(0, 160),
);

check(
  "D1 no uncaught page errors",
  pageErrors.length === 0,
  pageErrors.slice(0, 3).join(" | "),
);
console.log(
  "native-only commands touched:",
  [...nativeOnly].join(", ") || "(none)",
);
console.log(
  "signed kinds:",
  [...new Set(log.signed)].join(","),
  " total signatures:",
  log.signed.length,
);

// ── E. Mobile viewport ─────────────────────────────────────────────────────────────────
const mctx = await browser.newContext({
  viewport: { width: 390, height: 844 },
  deviceScaleFactor: 3,
  isMobile: true,
  hasTouch: true,
});
await installMockExtension(mctx, TYLER);
await mctx.addInitScript(
  ({ relay, pk }) => {
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
  },
  { relay: RELAY_WS, pk },
);
const mp = await mctx.newPage();
await mp.goto(APP);
await mp.waitForSelector('[data-testid="web-unlock-nip07"]');
await mp.screenshot({ path: `${SHOTS}/e-mobile-unlock.png` });
await mp.click('[data-testid="web-unlock-nip07"]');
await mp.waitForTimeout(8000);
await mp.screenshot({ path: `${SHOTS}/e-mobile-app.png` });
const m = await mp.evaluate(() => ({
  sw: document.documentElement.scrollWidth,
  cw: document.documentElement.clientWidth,
  sh: document.documentElement.scrollHeight,
  ch: document.documentElement.clientHeight,
  vp: document.querySelector("meta[name=viewport]")?.content,
}));
check(
  "E1 mobile 390x844: no horizontal overflow",
  m.sw <= m.cw,
  JSON.stringify(m),
);
await browser.close();

const failed = results.filter((r) => !r.ok);
console.log(
  `\n${results.length - failed.length}/${results.length} checks passed`,
);
process.exit(failed.length ? 1 : 0);
