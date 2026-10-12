// Web parity round 1: search, edit, channel management, downloads. TS ports of the desktop Rust commands
// (src-tauri/src/commands/{messages,profile,channels}.rs + events.rs), same event kinds and tags.
// Every write is one signature at user-action priority; reads that need bridge-only fields go over HTTP /query.
import { getSigner, signerPubkey } from "./signer";
import { queryRelay, queryRelayHttp, submitSignedEvent } from "./relayHttp";
import { relayHttpBase } from "./relayConfig";
import { getRelayMediaBlobUrl, isRelayMediaUrl } from "./relayMedia";
import { channelDetailFromEvent, channelInfoFromEvent, firstTag } from "./nostrConvert";
import type { NostrEvent } from "./nostrTypes";
import { profileValidOaOwner } from "./nipOa";

type Args = Record<string, any> | undefined;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const HEX64 = /^[0-9a-fA-F]{64}$/;
const MAX_CONTENT = 64 * 1024;
const MAX_MENTIONS = 50;
const USER_ACTION = { priority: 0 } as const;

const me = () => {
  const pk = signerPubkey();
  if (!pk) throw new Error("Not signed in");
  return pk;
};
function channelUuid(v: unknown): string {
  const id = String(v ?? "");
  if (!UUID.test(id)) throw new Error(`invalid channel UUID: ${id}`);
  return id;
}
function checkPubkey(pk: string) {
  if (!HEX64.test(pk)) throw new Error(`pubkey must be a 64-character hex string (got ${pk.length} chars)`);
}
async function publishNow(kind: number, tags: string[][], content = "") {
  const signed = (await getSigner().signEvent({ kind, content, tags }, USER_ACTION)) as unknown as NostrEvent;
  return submitSignedEvent(signed);
}

// ── 1. Search ────────────────────────────────────────────────────────────────
export function buildSearchMessagesFilter(a: Args) {
  const f: Record<string, unknown> = {
    kinds: [9, 40002, 45001, 45003],
    search: String(a?.q ?? "").trim(),
    search_mode: "prefix",
    limit: Math.min(a?.limit ?? 20, 500),
  };
  if (a?.channelId) f["#h"] = [a.channelId];
  const authors = ((a?.authors as string[] | undefined) ?? []).map((x) => x.trim()).filter(Boolean);
  if (authors.length) f.authors = authors;
  if (a?.since != null) f.since = a.since;
  if (a?.until != null) f.until = a.until;
  return f;
}
export function searchResponseFromEvents(events: NostrEvent[]) {
  const total = events.length;
  const hits = events.map((ev, idx) => ({
    event_id: ev.id,
    content: ev.content,
    kind: ev.kind,
    pubkey: ev.pubkey,
    channel_id: firstTag(ev, "h") ?? null,
    channel_name: null,
    created_at: ev.created_at,
    score: total <= 1 ? 1 : 1 - idx / total,
  }));
  return { hits, found: hits.length };
}
async function searchMessages(a: Args) {
  // search + search_mode are bridge-only extensions: HTTP /query (one NIP-98 signature per search).
  return searchResponseFromEvents(await queryRelayHttp([buildSearchMessagesFilter(a) as never]));
}

type UserHit = {
  pubkey: string;
  display_name: string | null;
  avatar_url: string | null;
  nip05_handle: string | null;
  owner_pubkey: string | null;
  is_agent: boolean;
};
function userHit(ev: NostrEvent): UserHit {
  let v: Record<string, unknown> = {};
  try {
    v = JSON.parse(ev.content) ?? {};
  } catch {
    /* bad kind:0 */
  }
  const s = (k: string) => (typeof v[k] === "string" ? (v[k] as string) : null);
  const owner = profileValidOaOwner(ev);
  return {
    pubkey: ev.pubkey,
    display_name: s("display_name") ?? s("name"),
    avatar_url: s("picture"),
    nip05_handle: s("nip05"),
    owner_pubkey: owner,
    is_agent: owner !== null,
  };
}
function matchScore(q: string, display: string, nip05: string, pk: string) {
  const f = (x: string, e: number, p: number, c: number) =>
    !x ? 0 : x === q ? e : x.startsWith(q) ? p : x.includes(q) ? c : 0;
  return Math.max(f(display, 1000, 900, 800), f(nip05, 700, 600, 500), pk && pk.startsWith(q) ? 400 : 0);
}
export function rankUserSearchResults(events: NostrEvent[], query: string, limit: number) {
  const q = query.trim().toLowerCase();
  if (!q || !limit) return { users: [], next_cursor: null };
  const scored: [number, number, UserHit][] = [];
  events.forEach((ev, i) => {
    if (ev.kind !== 0) return;
    const u = userHit(ev);
    const sc = matchScore(q, (u.display_name ?? "").toLowerCase(), (u.nip05_handle ?? "").toLowerCase(), u.pubkey.toLowerCase());
    if (sc) scored.push([sc, i, u]);
  });
  scored.sort((x, y) => y[0] - x[0] || x[1] - y[1]);
  const seen = new Set<string>();
  const users: UserHit[] = [];
  for (const [, , u] of scored) {
    if (seen.has(u.pubkey)) continue;
    seen.add(u.pubkey);
    users.push(u);
    if (users.length >= limit) break;
  }
  return { users, next_cursor: null as string | null };
}
export function listUserSearchResults(events: NostrEvent[], limit: number) {
  const latest = new Map<string, [number, UserHit]>();
  for (const ev of events) {
    if (ev.kind !== 0) continue;
    const k = ev.pubkey.toLowerCase();
    const prev = latest.get(k);
    if (!prev || ev.created_at > prev[0]) latest.set(k, [ev.created_at, userHit(ev)]);
  }
  const label = (u: UserHit) => (u.display_name ?? u.nip05_handle ?? u.pubkey).toLowerCase();
  const users = [...latest.values()]
    .map(([, u]) => u)
    .sort((x, y) => (label(x) < label(y) ? -1 : label(x) > label(y) ? 1 : x.pubkey < y.pubkey ? -1 : 1))
    .slice(0, limit);
  return { users, next_cursor: null as string | null };
}
async function searchUsers(a: Args) {
  const q = String(a?.query ?? "").trim();
  const max = Math.min(a?.limit ?? 8, 500);
  const n = Number.parseInt(String(a?.cursor ?? ""), 10);
  const page = Number.isFinite(n) && n > 0 ? n : 1;
  if (!max) return { users: [], next_cursor: null };
  if (!q) {
    const evs = await queryRelayHttp([{ kinds: [0], limit: max, page } as never]);
    const r = listUserSearchResults(evs, max);
    if (evs.length >= max) r.next_cursor = String(page + 1);
    return r;
  }
  const evs = await queryRelayHttp([{ kinds: [0], search: q, search_mode: "prefix", limit: max, page } as never]);
  const r = rankUserSearchResults(evs, q, max);
  if (evs.length >= max) r.next_cursor = String(page + 1);
  return r;
}

// ── 2. Edit message (kind 40003, events::build_message_edit) ─────────────────
function prefixed(list: unknown, prefix: string): string[][] {
  const out: string[][] = [];
  for (const t of (list as string[][] | undefined) ?? []) {
    if (!Array.isArray(t) || t[0] !== prefix) throw new Error(`${prefix} tags must use '${prefix}' prefix (got ${JSON.stringify(t?.[0])})`);
    out.push(t.map(String));
  }
  return out;
}
function mentionPTags(list: unknown): string[][] {
  const pks = (list as string[] | undefined) ?? [];
  if (pks.length > MAX_MENTIONS) throw new Error(`too many mentions (max ${MAX_MENTIONS})`);
  const seen = new Set<string>();
  const out: string[][] = [];
  for (const pk of pks) {
    checkPubkey(pk);
    const l = pk.toLowerCase();
    if (!seen.has(l)) {
      seen.add(l);
      out.push(["p", l]);
    }
  }
  return out;
}
function mentionRefTags(list: unknown): string[][] {
  return prefixed(list, "mention").map((m) => {
    if (!m[1]) throw new Error("mention reference tag missing pubkey");
    if (m.length > 3) throw new Error("mention reference tag has invalid display metadata");
    checkPubkey(m[1]);
    return m.length === 3 ? ["mention", m[1].toLowerCase(), m[2]] : ["mention", m[1].toLowerCase()];
  });
}
export function buildEditTags(input: Record<string, any>): { content: string; tags: string[][] } {
  const channelId = channelUuid(input.channelId);
  const target = String(input.eventId ?? "");
  if (!HEX64.test(target)) throw new Error(`invalid event ID: ${target}`);
  const content = String(input.content ?? "").trim();
  const media = prefixed(input.mediaTags, "imeta");
  if (!content && !media.length) throw new Error("edit must have content or attachments");
  if (new TextEncoder().encode(content).length > MAX_CONTENT) throw new Error("content exceeds maximum size");
  const tags: string[][] = [["h", channelId], ["e", target.toLowerCase()]];
  tags.push(...mentionPTags(input.mentionPubkeys));
  tags.push(...media);
  tags.push(...prefixed(input.emojiTags, "emoji"));
  if (Array.isArray(input.mentionTags)) {
    tags.push(...mentionRefTags(input.mentionTags));
    tags.push(["buzz:mention-snapshot"]);
  }
  if (input.suppressLinkPreviews) tags.push(["link-preview", "none"]);
  return { content, tags };
}
async function editMessage(a: Args) {
  const { content, tags } = buildEditTags(a?.input ?? {});
  await publishNow(40003, tags, content);
}

// ── 3. Channel management (kinds 9007/9002/9008/9000/9001) ───────────────────
const canonicalName = (n: string) => n.replace(/^[#\s]+/, "").trimEnd();
async function channelMeta(id: string) {
  return queryRelay([{ kinds: [39000], "#d": [id], limit: 1, consistency: "strong" } as never]);
}
async function createChannel(a: Args) {
  const vis = String(a?.visibility ?? "");
  if (vis !== "open" && vis !== "private") throw new Error(`invalid visibility: ${vis}`);
  const ct = String(a?.channelType ?? "");
  if (ct !== "stream" && ct !== "forum") throw new Error(`invalid channel_type: ${ct}`);
  const name = canonicalName(String(a?.name ?? ""));
  if (!name.trim()) throw new Error("channel name is required");
  const id = crypto.randomUUID();
  const tags = [["h", id], ["name", name], ["visibility", vis], ["channel_type", ct]];
  if (a?.description != null) tags.push(["about", String(a.description)]);
  if (a?.ttlSeconds != null) tags.push(["ttl", String(a.ttlSeconds)]);
  await publishNow(9007, tags);
  // The owner membership (kind 39002) is provisioned asynchronously; we created it, so we are a member.
  for (let i = 0; i < 4; i++) {
    const ev = (await channelMeta(id))[0];
    const info = ev ? channelInfoFromEvent(ev, true) : null;
    if (info) return info;
    await new Promise((r) => setTimeout(r, 300));
  }
  throw new Error("channel created but metadata not yet available");
}
async function updateChannel(a: Args) {
  const input = a?.input ?? {};
  const id = channelUuid(input.channelId);
  const hasTtl = "ttlSeconds" in input;
  if (input.name == null && input.description == null && input.visibility == null && !hasTtl)
    throw new Error("at least one of name, about, visibility, or ttl must be provided");
  if (input.visibility != null && input.visibility !== "open" && input.visibility !== "private")
    throw new Error('visibility must be "open" or "private"');
  const tags = [["h", id]];
  if (input.name != null) {
    const n = canonicalName(String(input.name));
    if (!n.trim()) throw new Error("channel name is required");
    tags.push(["name", n]);
  }
  if (input.description != null) tags.push(["about", String(input.description)]);
  if (input.visibility != null) tags.push(["visibility", input.visibility]);
  if (hasTtl) tags.push(["ttl", input.ttlSeconds == null ? "" : String(input.ttlSeconds)]);
  await publishNow(9002, tags);
  const ev = (await channelMeta(id))[0];
  if (!ev) throw new Error("channel updated but metadata not yet available");
  return channelDetailFromEvent(ev);
}
function assertScope(a: Args) {
  if (
    a?.expectedRelayUrl &&
    String(a.expectedRelayUrl).replace(/\/+$/, "").replace(/^wss?:/, "") !== relayHttpBase().replace(/^https?:/, "")
  )
    throw new Error("active community changed before the change was submitted; not applied");
  if (a?.expectedSignerPubkey && a.expectedSignerPubkey !== me())
    throw new Error("active identity changed before the change was submitted; not applied");
}
async function addChannelMembers(a: Args) {
  const id = channelUuid(a?.channelId);
  assertScope(a);
  const role = a?.role ?? null;
  if (role != null && !["admin", "bot", "guest", "member"].includes(role)) throw new Error(`invalid role: ${role}`);
  const roleTag = role && role !== "member" ? role : null;
  const added: string[] = [];
  const errors: { pubkey: string; error: string }[] = [];
  for (const pk of (a?.pubkeys as string[] | undefined) ?? []) {
    try {
      checkPubkey(pk);
      const tags = [["h", id], ["p", pk.toLowerCase()]];
      if (roleTag) tags.push(["role", roleTag]);
      await publishNow(9000, tags);
      added.push(pk);
    } catch (e) {
      errors.push({ pubkey: pk, error: e instanceof Error ? e.message : String(e) });
    }
  }
  return { added, errors };
}
const simple9002 = (tag: (a: Args) => string[]) => async (a: Args) => {
  await publishNow(9002, [["h", channelUuid(a?.channelId)], tag(a)]);
};

// ── 4. Downloads ─────────────────────────────────────────────────────────────
const isIos = () =>
  /iP(hone|ad|od)/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
export function filenameFor(url: string, given?: string | null, mime = ""): string {
  const clean = (s: string) => s.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, "_").trim();
  if (given && clean(given)) return clean(given);
  let base = "";
  try {
    base = decodeURIComponent(new URL(url, location.href).pathname.split("/").pop() ?? "");
  } catch {
    /* ignore */
  }
  base = clean(base) || "download";
  if (!/\.[a-z0-9]{2,5}$/i.test(base) && mime.includes("/")) {
    const ext = mime.split("/")[1].split(/[;+]/)[0].replace("jpeg", "jpg");
    if (/^[a-z0-9]{2,5}$/i.test(ext)) base += `.${ext}`;
  }
  return base;
}
async function downloadUrl(url: string, filename?: string | null) {
  if (!url) throw new Error("Nothing to download");
  const src = isRelayMediaUrl(url) ? await getRelayMediaBlobUrl(url) : url;
  let blob: Blob;
  try {
    const res = await fetch(src);
    if (!res.ok) throw new Error(`Download failed (${res.status})`);
    blob = await res.blob();
  } catch (e) {
    // Cross-origin file without CORS: let the browser open it directly.
    if (!isRelayMediaUrl(url)) {
      window.open(url, "_blank", "noopener");
      return;
    }
    throw e;
  }
  const name = filenameFor(url, filename, blob.type);
  // iOS (Safari tab and installed app): the share sheet offers Save Image / Save to Files.
  if (isIos() && typeof navigator.share === "function") {
    const file = new File([blob], name, { type: blob.type || "application/octet-stream" });
    if (!navigator.canShare || navigator.canShare({ files: [file] })) {
      try {
        await navigator.share({ files: [file] });
        return;
      } catch (e) {
        if (e instanceof DOMException && e.name === "AbortError") return;
      }
    }
  }
  const href = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = href;
  a.download = name;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(href), 60_000);
}

export const parityHandlers: Record<string, (a: Args) => unknown> = {
  search_messages: searchMessages,
  search_users: searchUsers,
  edit_message: editMessage,
  create_channel: createChannel,
  update_channel: updateChannel,
  set_channel_topic: simple9002((a) => ["topic", String(a?.topic ?? "")]),
  set_channel_purpose: simple9002((a) => ["purpose", String(a?.purpose ?? "")]),
  archive_channel: simple9002(() => ["archived", "true"]),
  unarchive_channel: simple9002(() => ["archived", "false"]),
  delete_channel: async (a) => {
    await publishNow(9008, [["h", channelUuid(a?.channelId)]]);
  },
  add_channel_members: addChannelMembers,
  remove_channel_member: async (a) => {
    const pk = String(a?.pubkey ?? "");
    checkPubkey(pk);
    await publishNow(9001, [["h", channelUuid(a?.channelId)], ["p", pk.toLowerCase()]]);
  },
  change_channel_member_role: async (a) => {
    const role = String(a?.role ?? "");
    if (role === "owner") throw new Error("cannot assign owner role — use transfer ownership");
    if (!["admin", "member", "guest", "bot"].includes(role)) throw new Error(`invalid role: ${role}`);
    const pk = String(a?.pubkey ?? "");
    checkPubkey(pk);
    await publishNow(9000, [["h", channelUuid(a?.channelId)], ["p", pk.toLowerCase()], ["role", role]]);
  },
  download_file: (a) => downloadUrl(String(a?.url ?? ""), a?.filename),
  download_image: (a) => downloadUrl(String(a?.url ?? ""), a?.filename),
};
