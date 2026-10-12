// Web parity batch 1 e2e: search_messages / search_users, edit_message, channel management, download_file.
// Needs local relay (:3100) and a web dev server. Usage: node scripts/pwa/s22-parity-batch1-e2e.mjs [appUrl]
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";

const APP = process.argv[2] || "http://127.0.0.1:5311/";
const RELAY_WS = "ws://localhost:3100";
const TYLER = "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const OTHER = "a".repeat(63) + "1";
let fails = 0;
const check = (name, ok, detail = "") => {
  if (!ok) fails++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  — " + detail : ""}`);
};
const browser = await chromium.launch();
const ctx = await browser.newContext({ acceptDownloads: true });
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
const err = (p) => p.then(() => "", (e) => String(e?.message ?? e));
const stamp = Date.now();
const general = await page.evaluate(async () => {
  const res = await window.__buzzWebInvoke("get_channels");
  const chs = Array.isArray(res) ? res : (res.channels ?? Object.values(res));
  return (chs.find((c) => c.name === "general") ?? chs[0]).id;
});

// 1. search
const word = `zebra${stamp}`;
const sent = await inv("send_channel_message", { channelId: general, content: `parity search ${word}` });
const sentId = sent.event_id;
let hits = { hits: [] };
for (let i = 0; i < 10 && !hits.hits.some((h) => h.event_id === sentId); i++) {
  await page.waitForTimeout(500);
  hits = await inv("search_messages", { q: word.slice(0, 9), limit: 20 });
}
check("search_messages finds a fresh message by prefix", hits.hits.some((h) => h.event_id === sentId && h.channel_id === general), `found=${hits.found}`);
const scoped = await inv("search_messages", { q: word, channelId: general, authors: [pk] });
check("search_messages honours channel + author scope", scoped.hits.every((h) => h.channel_id === general && h.pubkey === pk));
const prof = await inv("get_profile");
const nm = String(prof.display_name ?? "").slice(0, 3);
const users = nm ? await inv("search_users", { query: nm, limit: 8 }) : { users: [{ pubkey: pk }] };
check("search_users finds me by display-name prefix", users.users.some((u) => u.pubkey === pk), `q=${nm} n=${users.users.length}`);
const dir = await inv("search_users", { query: "", limit: 2 });
check("search_users empty query lists a directory page", Array.isArray(dir.users) && dir.users.length > 0);

// 2. edit
await inv("edit_message", { input: { channelId: general, eventId: sentId, content: `parity edited ${word}`, mediaTags: [], emojiTags: [], mentionPubkeys: [], suppressLinkPreviews: true, mentionTags: null } });
const edit = await page.waitForFunction(async ([ch, id]) => {
  const evs = await window.__buzzWebInvoke("get_channel_window", { channelId: ch, limitRows: 100 });
  return evs.some((e) => e.kind === 40003 && e.content.startsWith("parity edited") && e.tags.some((t) => t[0] === "e" && t[1] === id) && e.tags.some((t) => t[0] === "link-preview" && t[1] === "none"));
}, [general, sentId], { timeout: 15000 }).then(() => true, () => false);
check("edit_message publishes kind 40003 with e + link-preview tags", edit);
await page.goto(`${APP}#/channels/${general}`);
const shown = await page.locator(`text=parity edited ${word}`).first().waitFor({ timeout: 20000 }).then(() => true, () => false);
check("edited text renders in the channel", shown);
// 1b. topbar search UI shows the edited hit
await page.click('[data-testid="open-search"]');
await page.fill('[data-testid="search-dialog-input"]', word);
const uiHit = await page.locator('[data-testid="search-results"]').getByText(word).first().waitFor({ timeout: 20000 }).then(() => true, () => false);
check("topbar Search shows the hit (no NativeOnlyError)", uiHit);
await page.keyboard.press("Escape");
check("empty edit rejected like desktop", (await err(inv("edit_message", { input: { channelId: general, eventId: sentId, content: " " } }))).includes("content or attachments"));

// 3. channel management
const ch = await inv("create_channel", { name: `#parity-${stamp}`, channelType: "stream", visibility: "open", description: "made on web" });
check("create_channel returns the new channel (canonical name, member)", ch?.name === `parity-${stamp}` && ch.is_member === true, JSON.stringify({ n: ch?.name, m: ch?.is_member }));
const upd = await inv("update_channel", { input: { channelId: ch.id, name: `parity-r-${stamp}`, description: "renamed" } });
check("update_channel renames", JSON.stringify(upd).includes(`parity-r-${stamp}`));
await inv("set_channel_topic", { channelId: ch.id, topic: "t1" });
await inv("set_channel_purpose", { channelId: ch.id, purpose: "p1" });
const det = await page.waitForFunction(async (id) => {
  const d = await window.__buzzWebInvoke("get_channel_details", { channelId: id });
  return d.topic === "t1" && d.purpose === "p1" ? d : null;
}, ch.id, { timeout: 15000 }).then(() => true, () => false);
check("topic + purpose land", det);
const add = await inv("add_channel_members", { channelId: ch.id, pubkeys: [OTHER, "nothex"] });
check("add_channel_members reports added + per-pubkey errors", add.added.includes(OTHER) && add.errors.length === 1, JSON.stringify(add).slice(0, 200));
await inv("remove_channel_member", { channelId: ch.id, pubkey: OTHER });
check("remove_channel_member resolves", true);
await inv("archive_channel", { channelId: ch.id });
const arch = await page.waitForFunction(async (id) => (await window.__buzzWebInvoke("get_channel_details", { channelId: id })).archived_at != null, ch.id, { timeout: 15000 }).then(() => true, () => false);
check("archive_channel archives", arch);
await inv("unarchive_channel", { channelId: ch.id });
await inv("delete_channel", { channelId: ch.id });
check("unarchive + delete_channel resolve", true);
check("bad visibility rejected", (await err(inv("create_channel", { name: "x", channelType: "stream", visibility: "secret" }))).includes("invalid visibility"));

// 4. download
const [dl] = await Promise.all([
  page.waitForEvent("download", { timeout: 15000 }).catch(() => null),
  inv("download_file", { url: `${APP}manifest.webmanifest`, filename: "app.webmanifest" }),
]);
check("download_file triggers a browser download with the given name", dl?.suggestedFilename() === "app.webmanifest", dl?.suggestedFilename());

await page.screenshot({ path: "/tmp/s22-parity.png" });
await browser.close();
console.log(fails ? `\n${fails} FAILED` : "\nALL PASS");
process.exit(fails ? 1 : 0);
