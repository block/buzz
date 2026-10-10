// Slice-1 chat path: TS ports of the Rust commands for channel list, channel open, history, profiles, presence, and send.
// Reads/writes go through relayHttp (NIP-98 signed by the extension). Live updates + history paging use the shared
// relayClientSession WebSocket (NIP-42 AUTH signed via create_auth_event → signer).
import { getSigner, signerPubkey } from "./signer";
import { markBootReady, noteSigner } from "./signer/instrument";
import { queryRelay, submitSignedEvent } from "./relayHttp";
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
  const my = me();
  const [chain, openMeta, hiddenDms] = await Promise.all([
    (async () => {
      const memberEvents = await queryAll({ kinds: [39002], "#p": [my] });
      const ids = [
        ...new Set(memberEvents.map(dTag).filter((x): x is string => !!x)),
      ].sort();
      const meta = ids.length
        ? await queryRelay([{ kinds: [39000], "#d": ids, limit: ids.length }])
        : [];
      return { meta, memberEvents };
    })(),
    includeOpenDirectory
      ? queryAll({ kinds: [39000] })
      : Promise.resolve([] as NostrEvent[]),
    queryRelay([{ kinds: [KIND_DM_VISIBILITY], "#p": [my], limit: 1 }])
      .then((evs) => {
        const latest = evs.reduce<NostrEvent | null>(
          (a, e) => (!a || e.created_at > a.created_at ? e : a),
          null,
        );
        return new Set(
          latest
            ? latest.tags.filter((t) => t[0] === "h" && t[1]).map((t) => t[1])
            : [],
        );
      })
      .catch(() => new Set<string>()),
  ]);

  const membership = collectMembers(chain.memberEvents);
  const memberIds = new Set(chain.meta.map(dTag));
  const channels: ChannelInfo[] = [];
  for (const ev of chain.meta) {
    const c = channelInfoFromEvent(ev, true);
    if (c) channels.push(c);
  }
  for (const ev of openMeta) {
    const d = dTag(ev);
    if (d && memberIds.has(d)) continue;
    const c = channelInfoFromEvent(ev, false);
    if (c) channels.push(c);
  }

  if (channels.length) {
    const ids = channels.map((c) => c.id);
    const missing = ids.filter((id) => !membership.has(id));
    const lastFilters = ids.map((id) => ({
      kinds: RECENCY_KINDS,
      "#h": [id],
      limit: 1,
    }));
    const [members, messages] = await Promise.all([
      missing.length
        ? queryRelay([
            { kinds: [39002], "#d": missing, limit: missing.length },
          ]).catch(() => [])
        : Promise.resolve([]),
      (async () => {
        const out: NostrEvent[] = [];
        for (let i = 0; i < lastFilters.length; i += LAST_MESSAGE_BATCH)
          out.push(
            ...(await queryRelay(lastFilters.slice(i, i + LAST_MESSAGE_BATCH))),
          );
        return out;
      })(),
    ]);
    for (const [k, v] of collectMembers(members)) membership.set(k, v);
    const lastBy = new Map<string, number>();
    for (const ev of messages) {
      const h = firstTag(ev, "h");
      if (h && ev.created_at > (lastBy.get(h) ?? 0))
        lastBy.set(h, ev.created_at);
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
  // S1: media (imeta), custom-emoji, mention-ref, link-preview and sent-from-thread tags are not ported yet (text/mentions/threads only).
  if (
    a?.mediaTags?.length ||
    a?.emojiTags?.length ||
    a?.linkPreviewTags?.length
  )
    console.warn(
      "[web] send_channel_message: media/emoji/link-preview tags not supported in S1; dropped",
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
    const latest = new Map<string, [number, string]>();
    for (const ev of evs) {
      const pk = ev.tags.find((t) => t[0] === "p" && t[1])?.[1] ?? ev.pubkey;
      const s = ev.content.trim();
      if (s !== "online" && s !== "away" && s !== "offline") continue;
      const prev = latest.get(pk);
      if (!prev || prev[0] < ev.created_at) latest.set(pk, [ev.created_at, s]);
    }
    return Object.fromEntries([...latest].map(([k, [, s]]) => [k, s]));
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
  // local head cache is a desktop SQLite optimisation; web starts cold (IndexedDB cache = later slice)
  channel_head_cache_load: () => [],
  channel_head_cache_store: () => undefined,
  channel_head_cache_clear: () => undefined,
  join_channel: (a) =>
    publish(9021, [["h", a?.channelId]]).then(() => undefined),
  leave_channel: (a) =>
    publish(9022, [["h", a?.channelId]]).then(() => undefined),
};
