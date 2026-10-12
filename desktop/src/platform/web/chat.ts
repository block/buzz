// Slice-1 chat path: TS ports of the Rust commands for channel list, channel open, history, profiles, presence, and send.
// Reads/writes go through relayHttp (NIP-98 signed by the extension). Live updates + history paging use the shared
// relayClientSession WebSocket (NIP-42 AUTH signed via create_auth_event → signer).
import { getSigner, signerPubkey } from "./signer";
import { buildObserverControlEvent, decryptObserverEvent } from "./observer";
import { markBootReady, noteSigner } from "./signer/instrument";
import { queryRelay, queryRelayHttp, submitSignedEvent } from "./relayHttp";
import { relayHttpBase, wsToHttp } from "./relayConfig";
import type { Filter, NostrEvent } from "./nostrTypes";
import {
  type ChannelInfo,
  channelDetailFromEvent,
  channelInfoFromEvent,
  channelMembersFromEvent,
  dTag,
  emptyProfile,
  profileInfoFromEvent,
  timestampToIso,
  usersBatchFromEvents,
  firstTag,
} from "./nostrConvert";

type Args = Record<string, any> | undefined;

const KIND_DM_VISIBILITY = 30622;
const KIND_HUDDLE_STARTED = 48100;
const DIRECTORY_PAGE_SIZE = 500;
const LAST_MESSAGE_BATCH = 128;
const RECENCY_KINDS = [9, 40002, 45001, 45003];
const TIMELINE_KINDS = [
  9,
  40002,
  40008,
  40099,
  43001,
  43002,
  43003,
  43004,
  43005,
  43006,
  KIND_HUDDLE_STARTED,
];
const GET_EVENT_KINDS = [
  0,
  1,
  3,
  5,
  7,
  9,
  30078,
  40002,
  40003,
  40008,
  40099,
  40100,
  45001,
  45003,
  KIND_HUDDLE_STARTED,
];

const me = () => {
  const pk = signerPubkey();
  if (!pk) throw new Error("Not signed in");
  return pk;
};

let selfProfileLookup: Promise<unknown> | null = null;
async function lookupSelfProfile() {
  const pk = me();
  for (let attempt = 1; ; attempt++) {
    const t0 = Date.now();
    try {
      const evs = await queryRelay([{ kinds: [0], authors: [pk], limit: 1 }]);
      noteSigner(
        `profile lookup #${attempt}: ${evs[0] ? "found kind:0" : "authenticated read returned NO kind:0"} (${Date.now() - t0}ms)`,
      );
      return evs[0] ? profileInfoFromEvent(evs[0]) : emptyProfile(pk);
    } catch (e) {
      noteSigner(
        `profile lookup #${attempt} FAILED (${Date.now() - t0}ms): ${e instanceof Error ? e.message : e} — retrying, NOT treating as "no profile"`,
      );
      await new Promise((r) => setTimeout(r, Math.min(15_000, 1500 * attempt)));
    }
  }
}

async function queryAll(filter: Filter): Promise<NostrEvent[]> {
  const f: Filter = { ...filter, limit: DIRECTORY_PAGE_SIZE };
  const all: NostrEvent[] = [];
  for (;;) {
    const page = await queryRelay([f]);
    all.push(...page);
    if (page.length < DIRECTORY_PAGE_SIZE) return all;
    const last = page[page.length - 1];
    f.until = last.created_at;
    f.before_id = last.id;
  }
}

function collectMembers(events: NostrEvent[]) {
  const map = new Map<string, string[]>();
  for (const ev of events) {
    const d = dTag(ev);
    if (!d) continue;
    try {
      map.set(
        d,
        channelMembersFromEvent(ev).members.map((m) => m.pubkey),
      );
    } catch {
      /* skip */
    }
  }
  return map;
}

async function fetchChannels(
  includeOpenDirectory: boolean,
): Promise<ChannelInfo[]> {
  // Every relay HTTP request needs its own NIP-98 signature (payload-bound, replay-checked). With a remote NIP-46 signer
  // (Primal on iOS) each signature is a multi-second round trip, so the channel list is built from as few requests as
  // possible: round 1 = my memberships + DM visibility, round 2 = channel metadata + missing member lists + recency.
  const my = me();
  const isDmVisibility = (e: NostrEvent) => e.kind === KIND_DM_VISIBILITY;
  const [round1, openMeta] = await Promise.all([
    queryRelay([
      { kinds: [39002], "#p": [my], limit: DIRECTORY_PAGE_SIZE },
      { kinds: [KIND_DM_VISIBILITY], "#p": [my], limit: 1 },
    ]),
    includeOpenDirectory
      ? queryAll({ kinds: [39000] })
      : Promise.resolve([] as NostrEvent[]),
  ]);
  let memberEvents = round1.filter((e) => e.kind === 39002);
  if (memberEvents.length >= DIRECTORY_PAGE_SIZE)
    memberEvents = await queryAll({ kinds: [39002], "#p": [my] }); // rare: page through the full membership set
  const latestVis = round1
    .filter(isDmVisibility)
    .reduce<NostrEvent | null>(
      (a, e) => (!a || e.created_at > a.created_at ? e : a),
      null,
    );
  const hiddenDms = new Set(
    latestVis
      ? latestVis.tags.filter((t) => t[0] === "h" && t[1]).map((t) => t[1])
      : [],
  );
  const membership = collectMembers(memberEvents);
  const ids = [
    ...new Set(memberEvents.map(dTag).filter((x): x is string => !!x)),
  ].sort();
  const openIds = openMeta
    .map(dTag)
    .filter((x): x is string => !!x && !ids.includes(x));
  const allIds = [...ids, ...openIds];
  const missing = allIds.filter((id) => !membership.has(id));

  const round2: NostrEvent[] = [];
  if (ids.length || allIds.length) {
    const recency = (id: string) => ({
      kinds: RECENCY_KINDS,
      "#h": [id],
      limit: 1,
    });
    // First request carries metadata + missing members + as many recency filters as fit one batch.
    const head: Filter[] = [];
    if (ids.length) head.push({ kinds: [39000], "#d": ids, limit: ids.length });
    if (missing.length)
      head.push({ kinds: [39002], "#d": missing, limit: missing.length });
    const room = Math.max(0, LAST_MESSAGE_BATCH - head.length);
    const first = [...head, ...allIds.slice(0, room).map(recency)];
    round2.push(...(await queryRelay(first)));
    for (let i = room; i < allIds.length; i += LAST_MESSAGE_BATCH)
      round2.push(
        ...(await queryRelay(
          allIds.slice(i, i + LAST_MESSAGE_BATCH).map(recency),
        ).catch(() => [])),
      );
  }
  const meta = round2.filter((e) => e.kind === 39000);
  for (const [k, v] of collectMembers(round2.filter((e) => e.kind === 39002)))
    membership.set(k, v);
  const messages = round2.filter((e) => e.kind !== 39000 && e.kind !== 39002);

  const memberIds = new Set(meta.map(dTag));
  const channels: ChannelInfo[] = [];
  for (const ev of meta) {
    const c = channelInfoFromEvent(ev, true);
    if (c) channels.push(c);
  }
  for (const ev of openMeta) {
    const d = dTag(ev);
    if (d && memberIds.has(d)) continue;
    const c = channelInfoFromEvent(ev, false);
    if (c) channels.push(c);
  }
  const lastBy = new Map<string, number>();
  for (const ev of messages) {
    const h = firstTag(ev, "h");
    if (h && ev.created_at > (lastBy.get(h) ?? 0)) lastBy.set(h, ev.created_at);
  }
  for (const c of channels) {
    const pk = membership.get(c.id);
    if (pk) {
      c.member_count = pk.length;
      c.member_pubkeys = pk;
    }
    const ts = lastBy.get(c.id);
    if (ts) c.last_message_at = timestampToIso(ts);
  }
  return hiddenDms.size
    ? channels.filter((c) => c.channel_type !== "dm" || !hiddenDms.has(c.id))
    : channels;
}

function fnv1a(s: string): string {
  let h = 0xcbf29ce484222325n;
  for (const b of new TextEncoder().encode(s)) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h.toString(16).padStart(16, "0");
}

const getChannels = async (a: Args) => {
  const channels = await fetchChannels(false);
  markBootReady(); // first screen has data: release deferred settings decrypts / presence / other background signer work
  const lastMessages: Record<string, string> = {};
  for (const c of channels)
    if (c.last_message_at) lastMessages[c.id] = c.last_message_at;
  const hash = fnv1a(
    JSON.stringify(
      [...channels]
        .sort((x, y) => (x.id < y.id ? -1 : 1))
        .map(({ last_message_at: _l, ...rest }) => rest),
    ),
  );
  return {
    hash,
    channels: a?.knownHash === hash ? null : channels,
    last_messages: lastMessages,
  };
};

function channelWindowFilter(
  channelId: string,
  cap: number,
  cursor?: { created_at: number; event_id: string } | null,
): Filter {
  const f: Filter = {
    "#h": [channelId],
    kinds: TIMELINE_KINDS,
    limit: cap,
    top_level: true,
    include_summaries: true,
    include_aux: true,
  };
  if (cursor) {
    f.until = cursor.created_at;
    f.before_id = cursor.event_id;
  }
  return f;
}

async function getChannelMembers(a: Args) {
  const f: Filter = { kinds: [39002], "#d": [a?.channelId], limit: 1 };
  if (a?.readYourWrites) f.consistency = "strong";
  const evs = await queryRelay([f]);
  if (!evs[0]) throw new Error("channel members not found");
  const resp = channelMembersFromEvent(evs[0]);
  const pks = resp.members.slice(0, 500).map((m) => m.pubkey);
  if (pks.length) {
    const profs = await queryRelay([
      { kinds: [0], authors: pks, limit: pks.length },
    ]).catch(() => []);
    const names = new Map(
      profs.map((p) => [p.pubkey, profileInfoFromEvent(p).display_name]),
    );
    for (const m of resp.members) {
      if (m.role === "bot") m.is_agent = true;
      if (m.display_name === null && names.has(m.pubkey))
        m.display_name = names.get(m.pubkey) ?? null;
    }
  }
  return resp;
}

// ── Writes ────────────────────────────────────────────────────────────────────
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const HEX64 = /^[0-9a-fA-F]{64}$/;
const MAX_CONTENT = 64 * 1024;

async function resolveThreadRef(
  parent: string,
): Promise<{ root: string; parent: string }> {
  if (!HEX64.test(parent)) throw new Error("invalid parent event ID");
  const evs = await queryRelay([
    {
      ids: [parent],
      kinds: [9, 40002, 45001, 45003, KIND_HUDDLE_STARTED],
      limit: 1,
    },
  ]);
  if (!evs[0]) throw new Error("parent event not found");
  let root: string | undefined;
  let reply: string | undefined;
  for (const t of evs[0].tags)
    if (t.length >= 4 && t[0] === "e") {
      if (t[3] === "root") root = t[1];
      if (t[3] === "reply") reply = t[1];
    }
  const r = root ?? reply;
  return { root: r && r !== parent ? r : parent, parent };
}

function threadTags(root: string, parent: string): string[][] {
  return root === parent
    ? [["e", root, "", "reply"]]
    : [
        ["e", root, "", "root"],
        ["e", parent, "", "reply"],
      ];
}

async function sendChannelMessage(a: Args) {
  const channelId = String(a?.channelId ?? "");
  if (!UUID.test(channelId))
    throw new Error(`invalid channel UUID: ${channelId}`);
  const kind: number = a?.kind ?? 9;
  const content = String(a?.content ?? "").trim();
  if (new TextEncoder().encode(content).length > MAX_CONTENT)
    throw new Error("content exceeds maximum size");
  if (
    a?.expectedRelayUrl &&
    a.expectedRelayUrl.replace(/\/+$/, "").replace(/^wss?:/, "") !==
      relayHttpBase().replace(/^https?:/, "")
  )
    throw new Error(
      "active community changed before the message was submitted; not sent",
    );
  if (a?.expectedSignerPubkey && a.expectedSignerPubkey !== me())
    throw new Error(
      "active identity changed before the message was submitted; not sent",
    );
  if (a?.rootEventId && !a?.parentEventId)
    throw new Error("root_event_id requires parent_event_id");
  if (a?.sentFromThreadTag && kind !== 9)
    throw new Error("sent-from-thread provenance requires a stream message");
  const mentions: string[] = a?.mentionPubkeys ?? [];
  if (mentions.length > 50) throw new Error("too many mentions (max 50)");
  for (const m of mentions)
    if (!HEX64.test(m))
      throw new Error("pubkey must be a 64-character hex string");

  const tags: string[][] = [["h", channelId]];
  let resolvedRoot: string | null = null;
  if (kind === 45001) {
    // forum post: no thread tags
  } else if (a?.parentEventId) {
    const tr = a.rootEventId
      ? { root: a.rootEventId, parent: a.parentEventId }
      : await resolveThreadRef(a.parentEventId);
    if (!HEX64.test(tr.root) || !HEX64.test(tr.parent))
      throw new Error("invalid event ID");
    resolvedRoot = tr.root;
    tags.push(...threadTags(tr.root, tr.parent));
  } else if (kind === 45003) {
    throw new Error("forum comment requires parent_event_id");
  }
  const seen = new Set<string>();
  for (const m of mentions) {
    const l = m.toLowerCase();
    if (!seen.has(l)) {
      seen.add(l);
      tags.push(["p", l]);
    }
  }
  // imeta (attachments) pass through so uploaded images carry dim/blurhash/thumb; emoji and link-preview tags are still dropped on web.
  for (const t of a?.mediaTags ?? [])
    if (Array.isArray(t) && t[0] === "imeta") tags.push(t);
  if (a?.emojiTags?.length || a?.linkPreviewTags?.length)
    console.warn(
      "[web] send_channel_message: emoji/link-preview tags not supported yet; dropped",
    );
  for (const t of a?.mentionTags ?? []) tags.push(t);
  if (a?.sentFromThreadTag) tags.push(a.sentFromThreadTag);

  const signed = (await getSigner().signEvent({
    kind,
    content,
    tags,
  })) as unknown as NostrEvent;
  const res = await submitSignedEvent(signed);
  const pid = a?.parentEventId ?? null;
  const depth = !pid
    ? 0
    : resolvedRoot && pid === resolvedRoot
      ? 1
      : resolvedRoot
        ? 2
        : 1;
  return {
    event_id: res.event_id,
    root_event_id: resolvedRoot,
    parent_event_id: pid,
    depth,
    created_at: signed.created_at,
  };
}

async function publish(kind: number, tags: string[][], content = "") {
  const signed = (await getSigner().signEvent({
    kind,
    content,
    tags,
  })) as unknown as NostrEvent;
  return submitSignedEvent(signed);
}

async function updateProfile(a: Args) {
  const my = me();
  const prior = (
    await queryRelay([{ kinds: [0], authors: [my], limit: 1 }])
  )[0];
  let cur: Record<string, unknown> = {};
  try {
    cur = prior ? JSON.parse(prior.content) : {};
  } catch {
    cur = {};
  }
  const pick = (override: unknown, key: string) =>
    typeof override === "string"
      ? override
      : typeof cur[key] === "string"
        ? (cur[key] as string)
        : undefined;
  const next: Record<string, string> = {};
  const set = (k: string, v: string | undefined) => {
    if (v !== undefined) next[k] = v;
  };
  set("display_name", pick(a?.displayName, "display_name"));
  set("name", typeof cur.name === "string" ? cur.name : undefined);
  set("picture", pick(a?.avatarUrl, "picture"));
  set("about", pick(a?.about, "about"));
  set("nip05", pick(a?.nip05Handle, "nip05"));
  await publish(0, [], JSON.stringify(next));
  const fresh = (
    await queryRelay([{ kinds: [0], authors: [my], limit: 1 }])
  )[0];
  return fresh ? profileInfoFromEvent(fresh) : emptyProfile(my);
}

function relayMembersFromEvent(e: NostrEvent) {
  const seen = new Set<string>();
  const members: { pubkey: string; role: string }[] = [];
  for (const t of e.tags)
    if (t[0] === "member" && t[1] && !seen.has(t[1])) {
      seen.add(t[1]);
      members.push({ pubkey: t[1], role: t[2] || "member" });
    }
  for (const t of e.tags)
    if (t[0] === "p" && t[1] && !seen.has(t[1])) {
      seen.add(t[1]);
      members.push({ pubkey: t[1], role: t[3] || "member" });
    }
  return { members };
}

// ── Reactions / delete (mirror of src-tauri add_reaction, remove_reaction, delete_message) ──
// Signed at user-action priority (0) so a queue of settings decrypts can't delay a click.
const USER_ACTION = { priority: 0 } as const;
const SHORTCODE = /^[A-Za-z0-9_-]+$/;

async function publishNow(kind: number, tags: string[][], content = "") {
  const signed = (await getSigner().signEvent(
    { kind, content, tags },
    USER_ACTION,
  )) as unknown as NostrEvent;
  return submitSignedEvent(signed);
}

function eventIdArg(v: unknown): string {
  const id = String(v ?? "").trim();
  if (!HEX64.test(id)) throw new Error(`invalid event ID: ${id}`);
  return id.toLowerCase();
}

async function addReaction(a: Args) {
  const target = eventIdArg(a?.eventId);
  const emoji = String(a?.emoji ?? "").trim();
  const url: string | undefined = a?.emojiUrl ?? undefined;
  if (url != null) {
    // NIP-30 custom emoji: `:shortcode:` content + ["emoji", shortcode, url] (same validation as buzz-sdk)
    const code = emoji.replace(/^:+|:+$/g, "");
    if (!code)
      throw new Error(
        "invalid custom emoji reaction: emoji shortcode must not be empty",
      );
    if (code.length > 64 || !SHORTCODE.test(code))
      throw new Error(
        "invalid custom emoji reaction: emoji shortcode may only contain ASCII letters, digits, hyphens, and underscores (max 64)",
      );
    if (!url || url.length > 2048 || !/^https?:\/\//.test(url))
      throw new Error(
        "invalid custom emoji reaction: emoji image URL must be http(s) and at most 2048 bytes",
      );
    const sc = code.toLowerCase();
    await publishNow(
      7,
      [
        ["e", target],
        ["emoji", sc, url],
      ],
      `:${sc}:`,
    );
    return;
  }
  if ([...emoji].length > 64)
    throw new Error("emoji exceeds maximum length of 64 characters");
  await publishNow(7, [["e", target]], emoji);
}

async function removeReaction(a: Args) {
  const target = eventIdArg(a?.eventId);
  const emoji = String(a?.emoji ?? "").trim();
  const mine = await queryRelay([
    { kinds: [7], "#e": [target], authors: [me()] },
  ]);
  const ev = mine.find((e) => e.content.trim() === emoji);
  if (!ev) throw new Error("could not find your reaction event for this emoji");
  await publishNow(5, [["e", ev.id]]);
}

async function deleteMessage(a: Args) {
  const channelId = String(a?.channelId ?? "");
  if (!UUID.test(channelId))
    throw new Error(`invalid channel UUID: ${channelId}`);
  await publishNow(5, [
    ["h", channelId],
    ["e", eventIdArg(a?.eventId)],
  ]);
}

// open_dm: mirror of src-tauri open_dm. Kind 41010 with one ["p", pubkey] per participant; the relay finds the
// existing DM for that participant set or creates it and replies "response:{"channel_id":...}". Then re-read kind 39000.
async function openDm(a: Args) {
  const pubkeys = (a?.pubkeys as string[] | undefined) ?? [];
  if (!pubkeys.length) throw new Error("dm_open requires at least one pubkey");
  const tags = pubkeys.map((pk) => {
    if (!HEX64.test(String(pk))) throw new Error(`invalid pubkey: ${pk}`);
    return ["p", String(pk).toLowerCase()];
  });
  let res = await publishNow(41010, tags);
  if (res.message.startsWith("duplicate:")) {
    // Identical event (same participants, same second) already ingested: no channel id in the ack.
    // Re-sign one second later so the id differs; the relay then returns the existing DM's id.
    await new Promise((r) => setTimeout(r, 1100));
    res = await publishNow(41010, tags);
  }
  const raw = res.message.startsWith("response:")
    ? res.message.slice(9)
    : res.message;
  let channelId: string;
  try {
    channelId = JSON.parse(raw).channel_id;
  } catch (e) {
    throw new Error(
      `response parse failed: ${e instanceof Error ? e.message : e}`,
    );
  }
  const evs = await queryRelay([
    { kinds: [39000], "#d": [channelId], limit: 1 },
  ]);
  const info = evs[0] ? channelInfoFromEvent(evs[0], true) : null;
  if (!info)
    throw new Error("DM channel created but metadata not yet available");
  return info;
}

/** Channels the signed-in member belongs to (used by Web Push to scope DM lease filters). */

export const chatHandlers: Record<
  string,
  (a: Args) => unknown | Promise<unknown>
> = {
  get_channels: getChannels,
  get_open_channel_directory: () => fetchChannels(true),
  get_channel_details: async (a) => {
    const evs = await queryRelay([
      { kinds: [39000], "#d": [a?.channelId], limit: 1 },
    ]);
    if (!evs[0]) throw new Error("channel not found");
    return channelDetailFromEvent(evs[0]);
  },
  get_channel_members: getChannelMembers,
  get_channel_window: async (a) =>
    queryRelay([
      channelWindowFilter(
        a?.channelId,
        Math.min(a?.limitRows ?? 50, 200),
        a?.cursor,
      ),
    ]),
  get_channel_messages_before: async (a) => {
    const cap = Math.min(a?.limit ?? 200, 500);
    const f: Filter = {
      "#h": [a?.channelId],
      kinds: TIMELINE_KINDS,
      until: a?.before,
      limit: cap,
    };
    if (a?.beforeId) f.before_id = a.beforeId;
    const events = await queryRelay([f]);
    const last = events[events.length - 1];
    return {
      events,
      next_cursor:
        events.length >= cap && last
          ? { created_at: last.created_at, event_id: last.id }
          : null,
    };
  },
  // Thread subtree read (mirror of src-tauri get_thread_replies). `depth_limit` is a bridge-only field, so this always goes over
  // HTTP /query (one NIP-98 token at read priority P1, which a settings decrypt (P2) can never queue ahead of).
  get_thread_replies: async (a) => {
    const cap = Math.min(a?.limit ?? 200, 500);
    const f: Filter = {
      "#e": [a?.rootEventId],
      kinds: TIMELINE_KINDS,
      depth_limit: a?.depthLimit ?? 64,
      limit: cap,
      include_aux: true,
    };
    if (a?.channelId) f["#h"] = [a.channelId];
    if (a?.cursor) {
      f.thread_cursor = a.cursor.created_at;
      f.thread_cursor_id = a.cursor.event_id;
    }
    const events = await queryRelayHttp([f]);
    const replies = events.filter((e) => TIMELINE_KINDS.includes(e.kind));
    const last = replies[replies.length - 1];
    return {
      events,
      next_cursor:
        replies.length >= cap && last
          ? { created_at: last.created_at, event_id: last.id }
          : null,
    };
  },
  get_event: async (a) => {
    const evs = await queryRelay([
      { ids: [a?.eventId], kinds: GET_EVENT_KINDS, limit: 1 },
    ]);
    if (!evs[0]) throw new Error("event not found");
    return JSON.stringify(evs[0]);
  },
  get_events: async (a) => {
    const ids = [
      ...new Set<string>(
        (a?.eventIds ?? [])
          .map((x: string) => x.trim().toLowerCase())
          .filter((x: string) => HEX64.test(x)),
      ),
    ];
    const out = new Map<string, NostrEvent>();
    for (let i = 0; i < ids.length; i += 1000) {
      const chunk = ids.slice(i, i + 1000);
      for (const e of await queryRelay([
        { ids: chunk, kinds: GET_EVENT_KINDS, limit: chunk.length },
      ]))
        out.set(e.id, e);
    }
    return [...out.values()];
  },
  get_profile: async () => {
    // The first-run onboarding gate treats ANY settled answer without a kind:0 as "new user, show create-profile". So this
    // must only ever return "no profile" after a definitive, authenticated empty read. A failed/timed-out/unauthenticated read
    // is "unknown": keep retrying (splash + stuck hint show the reason) rather than risk publishing over a real profile.
    selfProfileLookup ??= lookupSelfProfile().finally(() => {
      selfProfileLookup = null;
    });
    return selfProfileLookup;
  },
  get_user_profile: async (a) => {
    const target = a?.pubkey ?? me();
    const evs = await queryRelay([{ kinds: [0], authors: [target], limit: 1 }]);
    return evs[0] ? profileInfoFromEvent(evs[0]) : emptyProfile(target);
  },
  get_users_batch: async (a) => {
    const pks: string[] = a?.pubkeys ?? [];
    if (!pks.length) return { profiles: {}, missing: [] };
    return usersBatchFromEvents(
      await queryRelay([{ kinds: [0], authors: pks }]),
      pks,
    );
  },
  get_presence: async (a) => {
    const pks: string[] = a?.pubkeys ?? [];
    if (!pks.length) return {};
    const evs = await queryRelay([{ kinds: [20001], authors: pks }]);
    // Presence expires after three heartbeats: an agent that died without saying "offline" must not stay online forever.
    const cutoff = Math.floor(Date.now() / 1000) - 3 * 60;
    const latest = new Map<string, [number, string]>();
    for (const ev of evs) {
      const pk = ev.tags.find((t) => t[0] === "p" && t[1])?.[1] ?? ev.pubkey;
      const s = ev.content.trim();
      if (s !== "online" && s !== "away" && s !== "offline") continue;
      const prev = latest.get(pk);
      if (!prev || prev[0] < ev.created_at) latest.set(pk, [ev.created_at, s]);
    }
    return Object.fromEntries(
      [...latest]
        .filter(([, [ts, s]]) => ts >= cutoff && s !== "offline")
        .map(([k, [, s]]) => [k, s]),
    );
  },
  update_profile: updateProfile,
  ensure_starter_channels: async () => ({
    channels: await fetchChannels(true),
    changed_channel_ids: [],
    error: null,
  }), // S1: read-only; starter-channel creation is S2
  relay_requires_membership: async (a) => {
    const base = a?.relayUrl ? wsToHttp(a.relayUrl) : relayHttpBase();
    let res: Response;
    try {
      res = await fetch(`${base}/info`, {
        headers: { Accept: "application/nostr+json" },
      });
    } catch {
      throw new Error("relay unreachable: could not connect to relay");
    }
    if (!res.ok) throw new Error(`relay returned ${res.status}`);
    const info = (await res.json()) as { supported_nips?: number[] };
    return (info.supported_nips ?? []).includes(43);
  },
  list_relay_members: async () => {
    const ev = (await queryRelay([{ kinds: [13534], limit: 1 }]))[0];
    return ev ? relayMembersFromEvent(ev) : { members: [] };
  },
  get_my_relay_membership: async () => {
    const ev = (await queryRelay([{ kinds: [13534], limit: 1 }]))[0];
    if (!ev) return { member: null };
    return {
      member:
        relayMembersFromEvent(ev).members.find((m) => m.pubkey === me()) ??
        null,
    };
  },
  send_channel_message: sendChannelMessage,
  open_dm: openDm,
  decrypt_observer_event: decryptObserverEvent,
  build_observer_control_event: buildObserverControlEvent,
  add_reaction: addReaction,
  remove_reaction: removeReaction,
  delete_message: deleteMessage,
  // local head cache is a desktop SQLite optimisation; web starts cold (IndexedDB cache = later slice)
  channel_head_cache_load: () => [],
  channel_head_cache_store: () => undefined,
  channel_head_cache_clear: () => undefined,
  join_channel: (a) =>
    publish(9021, [["h", a?.channelId]]).then(() => undefined),
  leave_channel: (a) =>
    publish(9022, [["h", a?.channelId]]).then(() => undefined),
  // hide_dm: mirror of src-tauri hide_dm (events::build_dm_hide). Kind 41012 with ["h", channelId]; the
  // relay records it and fetchChannels already filters hidden DMs, so this persists across devices like desktop.
  hide_dm: (a) =>
    publishNow(41012, [["h", String(a?.channelId ?? "")]]).then(
      () => undefined,
    ),
};
