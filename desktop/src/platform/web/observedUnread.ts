// Web port of the desktop observed-unread store (src-tauri/src/observed_unread.rs) and unread_catch_up
// (src-tauri/src/unread_catch_up.rs). Desktop keeps these in SQLite; web keeps the same projection model in
// localStorage per (pubkey, relay) scope. Read markers themselves sync with desktop through the shared read-state
// manager (encrypted relay event), so "clear on read" matches desktop.

type Args = Record<string, any> | undefined;
type Scope = { pubkey: string; relayUrl: string };
type StoredEvent = {
  channelId: string;
  id: string;
  createdAt: number;
  rootId: string | null;
  highPriority: boolean;
  countsTowardBadge: boolean;
  countsTowardAppBadge: boolean;
};
type Store = {
  generation: string;
  revision: number;
  lastSequence: number;
  events: Record<string, StoredEvent>;
  channelLatest: Record<string, number>;
  markers: Record<string, number>;
  membership: Record<string, string[]>;
};
type Projection = {
  channelId: string;
  latest: number;
  count: number;
  badgeCount: number;
  appBadgeCount: number;
  topLevelUnread: boolean;
  highPriorityCount: number;
};

const MAX_EVENTS = 5_000;
const keyOf = (s: Scope) => `buzz-web-observed-unread.v1:${s.pubkey.toLowerCase()}:${s.relayUrl.toLowerCase().replace(/\/$/, "")}`;
const cache = new Map<string, Store>();

function load(s: Scope): Store {
  const key = keyOf(s);
  const hit = cache.get(key);
  if (hit) return hit;
  let store: Store | null = null;
  try {
    store = JSON.parse(localStorage.getItem(key) || "null");
  } catch {
    store = null;
  }
  if (!store || typeof store !== "object" || !store.events) {
    store = {
      generation: crypto.randomUUID(),
      revision: 0,
      lastSequence: 0,
      events: {},
      channelLatest: {},
      markers: {},
      membership: {},
    };
  }
  store.membership ??= {};
  cache.set(key, store);
  return store;
}

function persist(s: Scope, store: Store) {
  const ids = Object.keys(store.events);
  if (ids.length > MAX_EVENTS) {
    ids
      .sort((a, b) => store.events[a].createdAt - store.events[b].createdAt)
      .slice(0, ids.length - MAX_EVENTS)
      .forEach((id) => delete store.events[id]);
  }
  try {
    localStorage.setItem(keyOf(s), JSON.stringify(store));
  } catch {
    /* quota: keep in memory */
  }
}

export function projections(store: Store): Projection[] {
  const channels = new Map<string, Projection>();
  const blank = (channelId: string, latest = 0): Projection => ({
    channelId,
    latest,
    count: 0,
    badgeCount: 0,
    appBadgeCount: 0,
    topLevelUnread: false,
    highPriorityCount: 0,
  });
  for (const [channelId, latest] of Object.entries(store.channelLatest)) channels.set(channelId, blank(channelId, latest));
  for (const event of Object.values(store.events)) {
    let readAt = Math.max(store.markers[event.channelId] ?? 0, store.markers[`msg:${event.id}`] ?? 0);
    if (event.rootId) readAt = Math.max(readAt, store.markers[`thread:${event.rootId}`] ?? 0);
    if (event.createdAt <= readAt) continue;
    const p = channels.get(event.channelId) ?? blank(event.channelId);
    p.latest = Math.max(p.latest, event.createdAt);
    p.count += 1;
    p.badgeCount += event.countsTowardBadge ? 1 : 0;
    p.appBadgeCount += event.countsTowardAppBadge ? 1 : 0;
    p.topLevelUnread ||= event.rootId === null;
    p.highPriorityCount += event.highPriority ? 1 : 0;
    channels.set(event.channelId, p);
  }
  return [...channels.values()].sort((a, b) => a.channelId.localeCompare(b.channelId));
}

const SEED_KINDS: Array<[string, string]> = [
  ["participatedRootIds", "participated"],
  ["authoredRootIds", "authored"],
  ["mentionedRootIds", "mentioned"],
  ["followedRootIds", "followed"],
  ["mutedRootIds", "muted_root"],
  ["mutedChannelIds", "muted_channel"],
];

function setMembership(store: Store, kind: string, value: string, present: boolean) {
  const set = new Set(store.membership[kind] ?? []);
  if (present) set.add(value);
  else set.delete(value);
  store.membership[kind] = [...set];
}

function openScope(a: Args) {
  const req = a?.request ?? {};
  const scope: Scope = req.scope;
  const store = load(scope);
  const seed = req.membershipSeed;
  if (seed) for (const [field, kind] of SEED_KINDS) for (const v of seed[field] ?? []) setMembership(store, kind, String(v), true);
  persist(scope, store);
  return {
    kind: "snapshot",
    scope,
    generation: store.generation,
    revision: store.revision,
    lastAckedSequence: store.lastSequence,
    migrationComplete: true,
    membershipSeeded: true,
    channels: projections(store),
  };
}

function ingest(a: Args) {
  const req = a?.request ?? {};
  const scope: Scope = req.scope;
  const store = load(scope);
  const before = new Map(projections(store).map((p) => [p.channelId, JSON.stringify(p)]));
  if (req.clearAll) {
    store.events = {};
    store.channelLatest = {};
  }
  for (const channelId of req.clearChannels ?? []) {
    delete store.channelLatest[channelId];
    for (const [id, ev] of Object.entries(store.events)) if (ev.channelId === channelId) delete store.events[id];
  }
  for (const l of req.channelLatest ?? []) store.channelLatest[l.channelId] = Math.max(store.channelLatest[l.channelId] ?? 0, l.createdAt);
  for (const ev of req.events ?? []) if (!store.events[ev.id]) store.events[ev.id] = { ...ev, rootId: ev.rootId ?? null };
  for (const m of req.markers ?? []) {
    if (m.readAt === null) delete store.markers[m.contextId];
    else store.markers[m.contextId] = Math.max(store.markers[m.contextId] ?? 0, m.readAt);
  }
  for (const m of req.membership ?? []) setMembership(store, String(m.kind), String(m.value), m.present === true);
  const after = projections(store);
  const afterIds = new Set(after.map((p) => p.channelId));
  const baseRevision = store.revision;
  store.revision += 1;
  store.lastSequence = req.sequence ?? store.lastSequence;
  persist(scope, store);
  return {
    kind: "delta",
    scope,
    generation: store.generation,
    baseRevision,
    revision: store.revision,
    ackedSequence: store.lastSequence,
    upserts: after.filter((p) => before.get(p.channelId) !== JSON.stringify(p)),
    removed: [...before.keys()].filter((id) => !afterIds.has(id)),
  };
}

// ── unread_catch_up (port of unread_catch_up.rs) ─────────────────────────────
type Ev = { id: string; kind: number; pubkey: string; content: string; created_at: number; tags: string[][] };
const CATCH_UP_LIMIT = 1000;
const ACTIVITY_LIMIT = 100;
const KIND_STREAM_MESSAGE = 9;
const KIND_STREAM_MESSAGE_V2 = 40002;
const KIND_FORUM_POST = 45001;
const KIND_FORUM_COMMENT = 45003;
const KIND_HUDDLE_STARTED = 48100;

const hasExact = (tags: string[][], n: string, v: string) => tags.some((t) => t[0] === n && t[1] === v);
const hasValue = (tags: string[][], n: string, v: string) => tags.some((t) => t[0] === n && typeof t[1] === "string" && t[1].toLowerCase() === v);

export function threadReference(tags: string[][]) {
  const e = tags.filter((t) => t[0] === "e" && t[1] != null);
  const root = e.find((t) => t[3] === "root");
  const reply = [...e].reverse().find((t) => t[3] === "reply");
  if (!reply) return { parentId: null as string | null, rootId: null as string | null };
  const parentId = reply[1];
  return { parentId, rootId: root?.[1] ?? parentId };
}

let huddleKind = KIND_HUDDLE_STARTED;
export function _setHuddleKindForTests(k: number) {
  huddleKind = k;
}

async function catchUp(a: Args, fetchChannel?: (filter: Record<string, unknown>) => Promise<Ev[]>) {
  const req = a?.request ?? {};
  const self = String(req.selfPubkey ?? "").toLowerCase();
  const channels: Array<{ id: string; type: string; name: string; readAt: number | null }> = req.channels ?? [];
  const muted = new Set<string>(req.mutedChannelIds ?? []);
  // Membership comes from the open scope for this pubkey (seeded/updated via open_scope + ingest).
  const store = [...cache.entries()].find(([k]) => k.includes(`:${self}:`))?.[1];
  const membership = (k: string) => new Set<string>(store?.membership[k] ?? []);
  const participated = membership("participated");
  const authored = membership("authored");
  const mentioned = membership("mentioned");
  const followed = membership("followed");
  const mutedRoots = membership("muted_root");
  const fetchOne =
    fetchChannel ??
    (async (f: Record<string, unknown>) => {
      const { queryRelay } = await import("./relayHttp");
      return (await queryRelay([f as never])) as unknown as Ev[];
    });

  const fetched: Array<{ channel: (typeof channels)[number]; events: Ev[] }> = [];
  const failures: unknown[] = [];
  const queue = channels.map((c, i) => ({ c, i }));
  const results: Array<{ channel: (typeof channels)[number]; events: Ev[] } | null> = new Array(channels.length).fill(null);
  const worker = async () => {
    for (let item = queue.shift(); item; item = queue.shift()) {
      const { c, i } = item;
      const kinds =
        c.type === "dm"
          ? [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2, KIND_FORUM_POST, KIND_FORUM_COMMENT, huddleKind]
          : [KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2, KIND_FORUM_POST, KIND_FORUM_COMMENT];
      try {
        const events = await fetchOne({ kinds, "#h": [c.id], since: c.readAt == null ? 0 : c.readAt + 1, limit: CATCH_UP_LIMIT });
        results[i] = { channel: c, events: events.slice(0, CATCH_UP_LIMIT) };
      } catch (e) {
        failures.push({ status: "error", channelId: c.id, error: e instanceof Error ? e.message : String(e) });
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(4, channels.length) }, worker));
  for (const r of results) if (r) fetched.push(r);

  const discoveries = fetched.map((item) => {
    const d = { participated: [] as string[], authored: [] as string[], mentioned: [] as string[] };
    for (const ev of item.events) {
      if (ev.pubkey.toLowerCase() === self) {
        const root = threadReference(ev.tags).rootId;
        if (root) {
          if (!participated.has(root)) (participated.add(root), d.participated.push(root));
        } else if (!authored.has(ev.id)) (authored.add(ev.id), d.authored.push(ev.id));
      } else if (hasValue(ev.tags, "p", self)) {
        const root = threadReference(ev.tags).rootId;
        if (root && !mentioned.has(root)) (mentioned.add(root), d.mentioned.push(root));
      }
    }
    return d;
  });

  const shouldNotify = (ev: Ev) => {
    if (hasExact(ev.tags, "broadcast", "1") || hasValue(ev.tags, "p", self)) return true;
    const h = ev.tags.find((t) => t[0] === "h")?.[1];
    if (h && muted.has(h)) return false;
    const ref = threadReference(ev.tags);
    if (ref.parentId == null) return true;
    if (!ref.rootId) return false;
    if (mutedRoots.has(ref.rootId)) return false;
    return participated.has(ref.rootId) || followed.has(ref.rootId) || authored.has(ref.rootId);
  };

  const outputs = fetched.map((item, idx) => {
    const observedEvents: unknown[] = [];
    const activityRows: Array<Record<string, unknown> & { id: string; created_at: number }> = [];
    let maxTrigger = 0;
    for (const ev of item.events) {
      if (ev.pubkey.toLowerCase() === self) continue;
      if (item.channel.readAt != null && ev.created_at <= item.channel.readAt) continue;
      if (!shouldNotify(ev)) continue;
      const ref = threadReference(ev.tags);
      const broadcast = hasExact(ev.tags, "broadcast", "1");
      const threaded = ref.parentId != null && !broadcast;
      const dm = item.channel.type === "dm";
      const high = dm || threaded || broadcast || hasValue(ev.tags, "p", self);
      maxTrigger = Math.max(maxTrigger, ev.created_at);
      observedEvents.push({
        id: ev.id,
        createdAt: ev.created_at,
        rootId: broadcast ? null : ref.rootId,
        highPriority: high,
        countsTowardBadge: dm || threaded || high,
        countsTowardAppBadge: dm || (!threaded && high),
      });
      if (threaded)
        activityRows.push({
          id: ev.id,
          kind: ev.kind,
          pubkey: ev.pubkey,
          content: ev.content,
          created_at: ev.created_at,
          channel_id: item.channel.id,
          channel_name: item.channel.name,
          tags: ev.tags,
        });
    }
    return { channelId: item.channel.id, observedEvents, maxTrigger, activityRows, discovered: discoveries[idx] };
  });

  const all = outputs.flatMap((o) => o.activityRows).sort((x, y) => x.created_at - y.created_at);
  const seen = new Set<string>();
  const uniq = all.filter((r) => !seen.has(r.id) && seen.add(r.id));
  const allowed = new Set(uniq.slice(Math.max(0, uniq.length - ACTIVITY_LIMIT)).map((r) => r.id));
  return {
    channels: [
      ...outputs.map((o) => ({ status: "success", ...o, activityRows: o.activityRows.filter((r) => allowed.has(r.id)) })),
      ...failures,
    ],
  };
}

export const observedUnreadHandlers: Record<string, (a: Args) => unknown> = {
  observed_unread_open_scope: openScope,
  observed_unread_ingest: ingest,
  unread_catch_up: (a) => catchUp(a),
};
export { catchUp as _catchUpForTests };
