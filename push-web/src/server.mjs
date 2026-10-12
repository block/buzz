// Hula Buzz Web Push sender.
//   GET    /vapid-public-key           -> { key }
//   PUT    /subscriptions  (NIP-98)    body { subscription, prefs, relay }
//   DELETE /subscriptions  (NIP-98)    body { endpoint }
//   GET    /healthz
// Env: PORT, PUBLIC_URL (exact external base URL used in NIP-98 `u`), ALLOWED_ORIGIN, RELAY_URL,
//      VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY (or VAPID_PRIVATE_KEY_FILE), VAPID_SUBJECT (mailto:/https:),
//      SERVICE_SECRET_KEY_FILE (hex secret of the service's OWN nostr identity for relay AUTH), DATA_DIR.
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import webpush from "web-push";
import { hexToBytes } from "nostr-tools/utils";
import { verifyNip98 } from "./nip98.mjs";
import { Store } from "./store.mjs";
import { RelayWatcher } from "./relay.mjs";
import { APPROVAL_KINDS, DM_MESSAGE_KINDS, MENTION_KINDS, buildPayload, recipientsFor, threadRootOf } from "./classify.mjs";

const env = process.env;
const PORT = Number(env.PORT || 8787);
const PUBLIC_URL = (env.PUBLIC_URL || `http://localhost:${PORT}`).replace(/\/+$/, "");
const ALLOWED_ORIGIN = env.ALLOWED_ORIGIN || "https://hulabuzz.huladesk.com";
const readSecret = (v, f) => (v ? v.trim() : f ? readFileSync(f, "utf8").trim() : "");
const VAPID_PUBLIC_KEY = env.VAPID_PUBLIC_KEY || "";
const VAPID_PRIVATE_KEY = readSecret(env.VAPID_PRIVATE_KEY, env.VAPID_PRIVATE_KEY_FILE);
if (!VAPID_PUBLIC_KEY || !VAPID_PRIVATE_KEY) throw new Error("VAPID keys not configured");
webpush.setVapidDetails(env.VAPID_SUBJECT || "mailto:admin@huladesk.com", VAPID_PUBLIC_KEY, VAPID_PRIVATE_KEY);
const serviceSk = env.SERVICE_SECRET_KEY_FILE ? hexToBytes(readSecret("", env.SERVICE_SECRET_KEY_FILE)) : null;

const store = new Store(env.DATA_DIR || "./data");
const relay = env.RELAY_URL ? new RelayWatcher({ url: env.RELAY_URL, serviceSecretKey: serviceSk }) : null;

// ---- caches ----
const names = new Map(); // pubkey -> display name
const channels = new Map(); // channel id -> { name, type }
const rootAuthors = new Map(); // event id -> pubkey
const dmChannelMembers = new Map(); // dm channel id -> Set(registered member pubkeys)
const delivered = new Set();

async function senderName(pk) {
  if (names.has(pk)) return names.get(pk);
  const [ev] = (await relay.query([{ kinds: [0], authors: [pk], limit: 1 }])) ?? [];
  let n = "";
  try {
    const c = JSON.parse(ev?.content || "{}");
    n = String(c.display_name || c.name || "").slice(0, 60);
  } catch {}
  names.set(pk, n);
  return n;
}
async function channelInfo(id) {
  if (!id) return null;
  if (channels.has(id)) return channels.get(id);
  const [ev] = (await relay.query([{ kinds: [39000], "#d": [id], limit: 1 }])) ?? [];
  const tag = (k) => ev?.tags.find((t) => t[0] === k)?.[1];
  const info = ev ? { name: String(tag("name") || "").slice(0, 60), type: tag("t") || (ev.tags.some((t) => t[0] === "hidden") ? "dm" : "stream") } : null;
  channels.set(id, info);
  return info;
}
async function rootAuthor(rootId) {
  if (!rootId) return null;
  if (rootAuthors.has(rootId)) return rootAuthors.get(rootId);
  const [ev] = (await relay.query([{ ids: [rootId], limit: 1 }])) ?? [];
  rootAuthors.set(rootId, ev?.pubkey ?? null);
  return ev?.pubkey ?? null;
}
async function refreshDmMembership() {
  const pks = store.pubkeys();
  dmChannelMembers.clear();
  if (!pks.length) return;
  const evs = await relay.query([{ kinds: [39002], "#p": pks }]);
  const reg = new Set(pks);
  for (const ev of evs) {
    const id = ev.tags.find((t) => t[0] === "d")?.[1];
    if (!id) continue;
    const info = await channelInfo(id);
    if (info?.type !== "dm") continue;
    dmChannelMembers.set(id, new Set(ev.tags.filter((t) => t[0] === "p" && reg.has(t[1])).map((t) => t[1])));
  }
}
function refreshLiveFilters() {
  const pks = store.pubkeys();
  if (!relay) return;
  if (!pks.length) return relay.setLiveFilters([]);
  relay.setLiveFilters([
    { kinds: [...new Set([...MENTION_KINDS, ...APPROVAL_KINDS])], "#p": pks },
    { kinds: DM_MESSAGE_KINDS }, // DM + thread-reply detection; channel visibility is whatever the service identity may read
  ]);
}

async function deliver(ev) {
  if (delivered.has(ev.id)) return;
  delivered.add(ev.id);
  if (delivered.size > 5000) delivered.delete(delivered.values().next().value);
  const registered = new Set(store.pubkeys());
  const root = threadRootOf(ev);
  const targets = recipientsFor(ev, { registered, dmChannelMembers, rootAuthor: root ? await rootAuthor(root) : null });
  if (!targets.length) return;
  const h = ev.tags.find((t) => t[0] === "h")?.[1];
  const [name, ch] = await Promise.all([senderName(ev.pubkey), channelInfo(h)]);
  for (const { recipient, category } of targets) {
    const user = store.user(recipient);
    if (!user || user.prefs[category] === false) continue;
    const payload = JSON.stringify(buildPayload(ev, category, { senderName: name, channelName: ch?.type === "dm" ? "" : ch?.name }));
    for (const sub of [...user.subs]) {
      try {
        await webpush.sendNotification(sub, payload, { TTL: 3600, urgency: category === "dm" || category === "mention" ? "high" : "normal" });
      } catch (e) {
        if (e.statusCode === 404 || e.statusCode === 410) store.removeEndpoint(sub.endpoint);
        else console.warn("push failed", e.statusCode ?? e.message);
      }
    }
  }
}

// ---- HTTP ----
function readBody(req, limit = 16 * 1024) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let n = 0;
    req.on("data", (c) => {
      n += c.length;
      if (n > limit) reject(new Error("body too large"));
      else chunks.push(c);
    });
    req.on("end", () => resolve(Buffer.concat(chunks)));
    req.on("error", reject);
  });
}
const cors = {
  "access-control-allow-origin": ALLOWED_ORIGIN,
  "access-control-allow-methods": "GET, PUT, DELETE, OPTIONS",
  "access-control-allow-headers": "authorization, content-type",
  "access-control-max-age": "600",
  vary: "origin",
};
function send(res, code, obj) {
  res.writeHead(code, { "content-type": "application/json", ...cors });
  res.end(JSON.stringify(obj));
}

export const server = createServer(async (req, res) => {
  const path = new URL(req.url, "http://x").pathname;
  try {
    if (req.method === "OPTIONS") return send(res, 204, {});
    if (req.method === "GET" && path === "/healthz") return send(res, 200, { ok: true, users: store.pubkeys().length });
    if (req.method === "GET" && path === "/vapid-public-key") return send(res, 200, { key: VAPID_PUBLIC_KEY });
    if (path === "/subscriptions" && (req.method === "PUT" || req.method === "DELETE")) {
      const body = await readBody(req);
      let pubkey;
      try {
        pubkey = verifyNip98(req.headers.authorization, { method: req.method, url: `${PUBLIC_URL}/subscriptions`, body });
      } catch (e) {
        return send(res, 401, { error: e.message });
      }
      const json = JSON.parse(body.toString("utf8") || "{}");
      if (req.method === "PUT") store.upsert(pubkey, json.subscription, json.prefs, json.relay);
      else store.remove(pubkey, json.endpoint);
      refreshLiveFilters();
      if (relay) void refreshDmMembership();
      return send(res, 200, { ok: true });
    }
    send(res, 404, { error: "not found" });
  } catch (e) {
    send(res, 400, { error: e.message });
  }
});

if (process.argv[1] && import.meta.url.endsWith(process.argv[1].split("/").pop())) {
  server.listen(PORT, () => console.log(`buzz web push sender on :${PORT} (public ${PUBLIC_URL})`));
  if (relay) {
    relay.onEvent = (ev) => void deliver(ev).catch((e) => console.warn("deliver", e.message));
    relay.start();
    refreshLiveFilters();
    setTimeout(() => void refreshDmMembership(), 3000);
    setInterval(() => void refreshDmMembership(), 5 * 60 * 1000);
  }
}
