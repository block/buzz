// Web reactions e2e: add_reaction / remove_reaction / delete_message web handlers + live render. Needs local relay (:3100) and a web dev server.
// Usage: node scripts/pwa/s11-reactions-e2e.mjs [appUrl]   (default http://127.0.0.1:5311/)
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5311/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext();
const { pk } = await installMockExtension(ctx, TYLER);
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

const inv = (cmd, args) => page.evaluate(([c, a]) => window.__buzzWebInvoke(c, a), [cmd, args]);
const stamp = Date.now();
const general = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  return (chs.find((c) => c.name === "general") ?? chs[0]).id;
});
const msg = await inv("send_channel_message", { channelId: general, content: `react me ${stamp}` });
const msgId = msg.event_id ?? msg.eventId ?? msg.id;
await page.goto(`${APP}#/channels/${general}`);
await page.locator(`text=react me ${stamp}`).first().waitFor({ timeout: 30000 });

// 1. add unicode reaction
await inv("add_reaction", { eventId: msgId, emoji: "🎉" });
check("add_reaction (unicode) resolves with no NativeOnlyError", true);
const row = page.locator('[data-testid="message-row"]', { hasText: `react me ${stamp}` }).first();
const pill = await row.getByText("🎉").first().waitFor({ timeout: 15000 }).then(() => true).catch(() => false);
check("reaction pill 🎉 appears live on the message", pill);

// 2. custom emoji
await inv("add_reaction", { eventId: msgId, emoji: ":Party_Parrot:", emojiUrl: "https://example.com/parrot.png" });
const custom = await page.waitForFunction(async ([id, ch]) => {
  const r = await window.__buzzWebInvoke("get_channel_window", { channelId: ch, limitRows: 100 });
  return r.some((e) => e.kind === 7 && e.content === ":party_parrot:" && e.tags.some((t) => t[0] === "emoji" && t[1] === "party_parrot" && t[2] === "https://example.com/parrot.png") && e.tags.some((t) => t[0] === "e" && t[1] === id));
}, [msgId, general], { timeout: 15000 }).then(() => true).catch(() => false);
check("custom emoji reaction published with normalized :shortcode: + emoji tag", custom);
const bad = await inv("add_reaction", { eventId: msgId, emoji: ":bad name:", emojiUrl: "https://x/y.png" }).then(() => false).catch((e) => String(e?.message ?? e).includes("invalid custom emoji"));
check("invalid custom shortcode rejected like desktop", bad);

// 3. remove the unicode reaction (kind:5 of our kind:7) and see the pill disappear live
await inv("remove_reaction", { eventId: msgId, emoji: "🎉" });
const pillGone = await row.getByText("🎉").first().waitFor({ state: "detached", timeout: 15000 }).then(() => true).catch(() => false);
check("remove_reaction removes the pill live", pillGone);
const removedErr = await inv("remove_reaction", { eventId: msgId, emoji: "🐙" }).then(() => false).catch((e) => String(e?.message ?? e).includes("could not find your reaction"));
check("remove_reaction of a reaction you never made errors like desktop", removedErr);

// 5. delete_message
const m2 = await inv("send_channel_message", { channelId: general, content: `delete me ${stamp}` });
await inv("delete_message", { channelId: general, eventId: m2.event_id ?? m2.eventId ?? m2.id });
check("delete_message resolves", true);

await page.screenshot({ path: "/tmp/s11-reactions.png" });
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
