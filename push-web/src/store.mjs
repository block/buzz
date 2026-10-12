// Subscription store: one JSON file (mode 0600), atomic writes. Holds push endpoints + their p256dh/auth keys and per-user prefs.
// No message content and no user private keys are ever stored.
import { mkdirSync, readFileSync, renameSync, writeFileSync, chmodSync } from "node:fs";
import { join } from "node:path";

const MAX_SUBS_PER_USER = 10;
const PREF_KEYS = ["dm", "mention", "thread_reply", "needs_action"];

export function sanitizePrefs(p) {
  const out = {};
  for (const k of PREF_KEYS) out[k] = p?.[k] !== false;
  return out;
}

export function sanitizeSubscription(s) {
  const endpoint = s?.endpoint;
  if (typeof endpoint !== "string" || !/^https:\/\//.test(endpoint) || endpoint.length > 2048) throw new Error("bad endpoint");
  const host = new URL(endpoint).hostname;
  // Only real browser push services (blocks SSRF via arbitrary endpoints).
  const ok = [/\.push\.apple\.com$/, /^fcm\.googleapis\.com$/, /\.googleapis\.com$/, /\.push\.services\.mozilla\.com$/, /\.notify\.windows\.com$/];
  if (!ok.some((re) => re.test(host))) throw new Error("unsupported push service");
  const { p256dh, auth } = s.keys ?? {};
  if (typeof p256dh !== "string" || typeof auth !== "string" || p256dh.length > 200 || auth.length > 100) throw new Error("bad keys");
  return { endpoint, keys: { p256dh, auth } };
}

export class Store {
  constructor(dir) {
    mkdirSync(dir, { recursive: true, mode: 0o700 });
    this.file = join(dir, "subscriptions.json");
    try {
      this.data = JSON.parse(readFileSync(this.file, "utf8"));
    } catch {
      this.data = { users: {} };
    }
  }
  save() {
    const tmp = `${this.file}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.data), { mode: 0o600 });
    renameSync(tmp, this.file);
    chmodSync(this.file, 0o600);
  }
  upsert(pubkey, subscription, prefs, relay) {
    const sub = sanitizeSubscription(subscription);
    const u = (this.data.users[pubkey] ??= { subs: [], prefs: sanitizePrefs(prefs) });
    u.prefs = sanitizePrefs(prefs);
    if (typeof relay === "string") u.relay = relay.slice(0, 200);
    u.subs = u.subs.filter((s) => s.endpoint !== sub.endpoint);
    u.subs.push({ ...sub, updatedAt: Date.now() });
    if (u.subs.length > MAX_SUBS_PER_USER) u.subs.splice(0, u.subs.length - MAX_SUBS_PER_USER);
    this.save();
  }
  remove(pubkey, endpoint) {
    const u = this.data.users[pubkey];
    if (!u) return;
    u.subs = u.subs.filter((s) => s.endpoint !== endpoint);
    if (!u.subs.length) delete this.data.users[pubkey];
    this.save();
  }
  removeEndpoint(endpoint) {
    for (const [pk, u] of Object.entries(this.data.users)) if (u.subs.some((s) => s.endpoint === endpoint)) this.remove(pk, endpoint);
  }
  pubkeys() {
    return Object.keys(this.data.users);
  }
  user(pubkey) {
    return this.data.users[pubkey];
  }
}
