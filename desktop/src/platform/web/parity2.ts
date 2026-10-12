// Web parity round 2: approvals, NIP-OA owner, workflows, canvas, forum, notes, team catalog.
// TS ports of desktop src-tauri commands (workflows.rs, canvas.rs, messages/forum.rs, social.rs, identity_archive.rs,
// team_catalog.rs). Same kinds/tags; writes are one signature at user-action priority.
import { parse as parseYaml } from "yaml";
import { verifyEvent } from "nostr-tools/pure";
import { getSigner, signerPubkey } from "./signer";
import { getRelayJson, queryRelay, submitSignedEvent } from "./relayHttp";
import type { NostrEvent } from "./nostrTypes";
import { applySuppression, linkPreviewSuppressionTargets, ownersFromProfiles, profileValidOaOwner } from "./nipOa";

type Args = Record<string, any> | undefined;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const HEX64 = /^[0-9a-fA-F]{64}$/;
const MAX_CONTENT = 64 * 1024;
const USER_ACTION = { priority: 0 } as const;
const me = () => {
  const pk = signerPubkey();
  if (!pk) throw new Error("Not signed in");
  return pk;
};
const nowSecs = () => Math.floor(Date.now() / 1000);
const tagValue = (ev: NostrEvent, name: string) => ev.tags.find((t) => t[0] === name && t.length >= 2)?.[1];
function checkContent(c: string) {
  if (new TextEncoder().encode(c).length > MAX_CONTENT) throw new Error(`content exceeds maximum size of ${MAX_CONTENT} bytes`);
}
async function sign(kind: number, tags: string[][], content = "", created_at?: number) {
  return (await getSigner().signEvent({ kind, content, tags, created_at }, USER_ACTION)) as unknown as NostrEvent;
}
async function publishNow(kind: number, tags: string[][], content = "", created_at?: number) {
  return submitSignedEvent(await sign(kind, tags, content, created_at));
}
export function parseCommandResponse<T>(message: string): T {
  if (message.startsWith("response:")) {
    try {
      return JSON.parse(message.slice(9)) as T;
    } catch (e) {
      throw new Error(`response parse failed: ${e instanceof Error ? e.message : e}`);
    }
  }
  try {
    return JSON.parse(message) as T;
  } catch (e) {
    throw new Error(`expected 'response:' prefix or valid JSON, got: ${message} (${e instanceof Error ? e.message : e})`);
  }
}

// ── 6. NIP-OA owner ─────────────────────────────────────────────────────────
async function resolveOaOwner(a: Args) {
  const target = String(a?.targetPubkey ?? "").toLowerCase();
  const ev = (await queryRelay([{ kinds: [0], authors: [target], limit: 1 }]))[0];
  const owner = ev ? profileValidOaOwner(ev) : null;
  return owner ? { owner, is_me: owner === me().toLowerCase() } : null;
}
async function ownersFor(events: NostrEvent[]) {
  const authors = [...new Set(events.map((e) => e.pubkey))];
  if (!authors.length) return new Map<string, string>();
  return ownersFromProfiles(await queryRelay([{ kinds: [0], authors }]).catch(() => []));
}

// ── 5 + 7. Workflows and approvals ──────────────────────────────────────────
function parseDefinition(yaml: string): Record<string, unknown> {
  try {
    const v = parseYaml(yaml);
    return v && typeof v === "object" && !Array.isArray(v) ? v : {};
  } catch {
    return {};
  }
}
export function workflowRecord(id: string, revision: string, channelId: string | null, owner: string, yaml: string, createdAt: number, updatedAt: number) {
  const definition = parseDefinition(yaml);
  const n = typeof definition.name === "string" && definition.name.trim() ? definition.name : id;
  return { id, revision, name: n, owner_pubkey: owner, channel_id: channelId, definition, status: "active", created_at: createdAt, updated_at: updatedAt };
}
const workflowFromEvent = (ev: NostrEvent) =>
  workflowRecord(tagValue(ev, "d") ?? "", ev.id, tagValue(ev, "h") ?? null, ev.pubkey, ev.content, ev.created_at, ev.created_at);
function uuidArg(v: unknown, what: string) {
  const s = String(v ?? "").trim();
  if (!UUID.test(s)) throw new Error(`invalid ${what}`);
  return s.toLowerCase();
}
async function getChannelsWorkflows(a: Args) {
  const ids: string[] = (a?.channelIds ?? []).map((c: string) => uuidArg(c, "channel id"));
  const seen = new Set<string>();
  const out = [];
  for (let i = 0; i < ids.length; i += 10) {
    // WS max 10 filters: one single-channel filter each (old relays narrowed multi-value #h to its first value).
    const evs = await queryRelay(ids.slice(i, i + 10).map((id) => ({ kinds: [30620], "#h": [id] })));
    for (const ev of evs) if (!seen.has(ev.id)) (seen.add(ev.id), out.push(workflowFromEvent(ev)));
  }
  return out;
}
function workflowDefinitionTags(workflowId: string, channelId: string, yaml: string, expected?: string) {
  checkContent(yaml);
  const tags = [["d", workflowId], ["h", channelId]];
  if (expected) {
    if (!HEX64.test(expected)) throw new Error("invalid workflow revision");
    tags.push(["expected-revision", expected]);
  }
  return tags;
}
async function createWorkflow(a: Args) {
  const channelId = String(a?.channelId ?? "");
  const yaml = String(a?.yamlDefinition ?? "");
  const id = crypto.randomUUID();
  const res = await publishNow(30620, workflowDefinitionTags(id, channelId, yaml), yaml);
  let secret: string | undefined;
  try {
    const v = parseCommandResponse<Record<string, unknown>>(res.message);
    if (typeof v.webhook_secret === "string") secret = v.webhook_secret;
  } catch {
    /* no secret */
  }
  const t = nowSecs();
  return { ...workflowRecord(id, res.event_id, channelId, me(), yaml, t, t), ...(secret ? { webhook_secret: secret } : {}) };
}
async function updateWorkflow(a: Args) {
  const id = String(a?.workflowId ?? "");
  const yaml = String(a?.yamlDefinition ?? "");
  const expected = String(a?.expectedRevision ?? "");
  const prior = (await queryRelay([{ kinds: [30620], "#d": [id], limit: 1 }]))[0];
  if (!prior) throw new Error("workflow not found");
  if (prior.id !== expected) throw new Error("workflow changed since it was loaded; refresh and try again");
  const channelId = tagValue(prior, "h");
  if (!channelId) throw new Error("workflow not found");
  // Replaceable event: a same-second update would not sort after the prior revision, so stamp it at least one second later.
  const res = await publishNow(30620, workflowDefinitionTags(id, channelId, yaml, expected), yaml, Math.max(nowSecs(), prior.created_at + 1));
  return workflowRecord(id, res.event_id, channelId, me(), yaml, prior.created_at, nowSecs());
}
async function triggerWorkflow(a: Args) {
  const id = String(a?.workflowId ?? "");
  const res = await publishNow(46020, [["d", id]]);
  const ack = parseCommandResponse<{ run_id?: string }>(res.message);
  if (!ack.run_id || !String(ack.run_id).trim()) throw new Error("workflow trigger response contained an empty run_id");
  return { run_id: ack.run_id, workflow_id: id, status: "pending" };
}
const approvalAction = (kind: number) => async (a: Args) => {
  const token = String(a?.token ?? "");
  if (!token) throw new Error("approval token is required");
  // Desktop sends ["t", token]; the relay looks the approval up by ["d", <token hash hex>] or ["e", <request id>]. Carry both
  // so a hash-shaped token resolves on the relay while staying byte-compatible with desktop.
  const tags = [["t", token]];
  if (/^[0-9a-f]{64}$/i.test(token)) tags.push(["d", token.toLowerCase()]);
  const res = await publishNow(kind, tags, a?.note ?? "");
  return { event_id: res.event_id };
};

// ── 8. Canvas ───────────────────────────────────────────────────────────────
const CANVAS_CHANGED = "conflict: canvas changed since it was loaded";
const CANVAS_REVISION_MISSING = "conflict: canvas revision does not exist";
const CANVAS_SUPERSEDED = "conflict: canvas save was superseded by a concurrent write";
const CANVAS_ANCESTRY_WALK_MAX = 256;
export function checkCanvasPrecondition(expected: string | null | undefined, head: [string, number] | null): number | null {
  if (expected == null) return null;
  if (expected === "none") {
    if (head) throw new Error(CANVAS_CHANGED);
    return null;
  }
  if (!head) throw new Error(CANVAS_REVISION_MISSING);
  if (head[0].toLowerCase() !== expected.toLowerCase()) throw new Error(CANVAS_CHANGED);
  return head[1];
}
export function canvasWriteSurvived(ourId: string, revisions: [string, string | null][]): boolean {
  if (!revisions.length) return false;
  const our = ourId.toLowerCase();
  if (revisions[0][0].toLowerCase() === our) return true;
  const byId = new Map(revisions.map(([id, exp]) => [id.toLowerCase(), exp]));
  const seen = new Set<string>();
  let cursor = revisions[0][0].toLowerCase();
  for (let i = 0; i < CANVAS_ANCESTRY_WALK_MAX; i++) {
    if (seen.has(cursor)) return false;
    seen.add(cursor);
    const exp = byId.get(cursor);
    if (!exp) return false;
    const next = exp.toLowerCase();
    if (next === our) return true;
    if (!byId.has(next)) return false;
    cursor = next;
  }
  return false;
}
async function getCanvas(a: Args) {
  const ev = (await queryRelay([{ kinds: [40100], "#h": [a?.channelId], limit: 1 }]))[0];
  return ev
    ? { content: ev.content, event_id: ev.id, updated_at: ev.created_at, author: ev.pubkey }
    : { content: "", event_id: null, updated_at: null, author: null };
}
async function setCanvas(a: Args) {
  const channelId = String(a?.channelId ?? "");
  if (!UUID.test(channelId)) throw new Error(`invalid channel UUID: ${channelId}`);
  const content = String(a?.content ?? "");
  checkContent(content);
  const expected: string | null = a?.expectedRevision ?? null;
  const headEv = (await queryRelay([{ kinds: [40100], "#h": [channelId], limit: 1, consistency: "strong" }]))[0];
  const floor = checkCanvasPrecondition(expected, headEv ? [headEv.id, headEv.created_at] : null);
  let createdAt: number | undefined;
  if (floor != null) {
    if (floor > nowSecs() + 60) throw new Error("canvas head is timestamped too far in the future; refusing to extend it");
    createdAt = Math.max(nowSecs(), floor + 1);
  }
  const tags = [["h", channelId]];
  if (expected) tags.push(["expected-revision", expected]);
  const res = await publishNow(40100, tags, content, createdAt);
  let verified = true;
  if (expected) {
    try {
      const evs = await queryRelay([{ kinds: [40100], "#h": [channelId], limit: CANVAS_ANCESTRY_WALK_MAX, consistency: "strong" }]);
      if (!canvasWriteSurvived(res.event_id, evs.map((e) => [e.id, tagValue(e, "expected-revision") ?? null]))) throw new Error(CANVAS_SUPERSEDED);
    } catch (e) {
      if (e instanceof Error && e.message === CANVAS_SUPERSEDED) throw e;
      verified = false;
    }
  }
  return { ok: true, event_id: res.event_id, verified };
}
async function getCanvasHistory(a: Args) {
  if (a?.beforeId != null && a?.until == null) throw new Error("before_id requires until");
  const size = a?.limit ?? 100;
  if (!(size >= 1 && size <= 1000)) throw new Error("limit must be between 1 and 1000");
  const f: Record<string, unknown> = { kinds: [40100], "#h": [a?.channelId], limit: size };
  if (a?.until != null) f.until = a.until;
  if (a?.beforeId != null) {
    if (!HEX64.test(a.beforeId)) throw new Error("before_id must be a 64-character hex event id");
    f.before_id = a.beforeId;
  }
  const evs = await queryRelay([f]);
  const last = evs[evs.length - 1];
  return {
    revisions: evs.map((e) => ({ event_id: e.id, content: e.content, created_at: e.created_at, author: e.pubkey })),
    next_cursor: evs.length === size && last ? { created_at: last.created_at, event_id: last.id } : null,
  };
}

// ── 8. Forum ────────────────────────────────────────────────────────────────
const emptySummary = () => ({ reply_count: 0, descendant_count: 0, last_reply_at: null, participants: [] });
function forumMessage(ev: NostrEvent, channelId: string) {
  return {
    event_id: ev.id, pubkey: ev.pubkey, sig: ev.sig, content: ev.content, kind: ev.kind, created_at: ev.created_at,
    channel_id: channelId, tags: ev.tags.map((t) => [...t]), thread_summary: emptySummary(), reactions: null,
  };
}
function forumReply(ev: NostrEvent, channelId: string, rootId: string) {
  let parent: string | undefined;
  let root: string | undefined;
  for (const t of ev.tags)
    if (t.length >= 2 && t[0] === "e") {
      if (t[3] === "root") root = t[1];
      else if (t[3] === "reply") parent = t[1];
      else if (!parent) parent = t[1];
    }
  const p = parent ?? rootId;
  const r = root ?? rootId;
  return {
    event_id: ev.id, pubkey: ev.pubkey, sig: ev.sig, content: ev.content, kind: ev.kind, created_at: ev.created_at,
    channel_id: channelId, tags: ev.tags.map((t) => [...t]), parent_event_id: p, root_event_id: r, depth: p === r ? 1 : 2,
    broadcast: false, reactions: null,
  };
}
async function suppressionFor(events: NostrEvent[]) {
  if (!events.length) return new Set<string>();
  const authors = [...new Set(events.map((e) => e.pubkey))];
  // One round trip: edits + author profiles (owner verification).
  const extra = await queryRelay([{ kinds: [40003], "#e": events.map((e) => e.id) }, { kinds: [0], authors }]).catch(() => []);
  return linkPreviewSuppressionTargets(events, extra.filter((e) => e.kind === 40003), ownersFromProfiles(extra.filter((e) => e.kind === 0)));
}
async function getForumPosts(a: Args) {
  const channelId = String(a?.channelId ?? "");
  const f: Record<string, unknown> = { kinds: [45001], "#h": [channelId], limit: Math.min(a?.limit ?? 20, 100) };
  if (a?.before != null) f.until = a.before;
  const evs = await queryRelay([f]);
  const sup = await suppressionFor(evs);
  const messages = evs.map((ev) => {
    const m = forumMessage(ev, channelId);
    applySuppression(m.tags, m.event_id, sup);
    return m;
  });
  return { messages, next_cursor: messages.length ? messages[messages.length - 1].created_at : null };
}
async function getForumThread(a: Args) {
  const channelId = String(a?.channelId ?? "");
  const eventId = String(a?.eventId ?? "");
  const evs = await queryRelay([
    { ids: [eventId], kinds: [9, 40002, 45001, 45003] },
    { kinds: [9, 45003], "#e": [eventId], "#h": [channelId] },
  ]);
  const sup = await suppressionFor(evs);
  let root: ReturnType<typeof forumMessage> | null = null;
  const replies = [];
  const seen = new Set<string>();
  for (const ev of evs) {
    if (seen.has(ev.id)) continue;
    seen.add(ev.id);
    if (ev.id === eventId) {
      root = forumMessage(ev, channelId);
      applySuppression(root.tags, root.event_id, sup);
    } else if (ev.kind !== 40003) {
      const r = forumReply(ev, channelId, eventId);
      applySuppression(r.tags, r.event_id, sup);
      replies.push(r);
    }
  }
  if (!root) throw new Error("forum thread root event not found");
  return { root, replies, total_replies: replies.length, next_cursor: null };
}

// ── 8. Notes (kind 1) ───────────────────────────────────────────────────────
const noteInfo = (ev: NostrEvent) => ({ id: ev.id, pubkey: ev.pubkey, created_at: ev.created_at, content: ev.content, tags: ev.tags.map((t) => [...t]) });
function userNotes(evs: NostrEvent[]) {
  const notes = evs.map(noteInfo);
  const last = notes[notes.length - 1];
  return { notes, next_cursor: last ? { before: last.created_at, before_id: last.id } : null };
}
function noteId(v: unknown) {
  const s = String(v ?? "");
  if (!HEX64.test(s)) throw new Error("invalid note id");
  return s;
}
const eTags = (ev: NostrEvent) => ev.tags.filter((t) => t[0] === "e" && t[1]).map((t) => t[1]);
async function publishNote(a: Args) {
  const content = String(a?.content ?? "");
  checkContent(content);
  const tags: string[][] = [];
  if (a?.replyTo) tags.push(["e", noteId(a.replyTo).toLowerCase(), "", "reply"]);
  const seen = new Set<string>();
  const mentions: string[] = a?.mentionPubkeys ?? [];
  if (mentions.length > 50) throw new Error("too many mentions (max 50)");
  for (const pk of mentions) {
    if (!HEX64.test(pk)) throw new Error("pubkey must be a 64-character hex string");
    const l = pk.toLowerCase();
    if (!seen.has(l)) (seen.add(l), tags.push(["p", l]));
  }
  for (const t of a?.mediaTags ?? []) {
    if (!Array.isArray(t) || t[0] !== "imeta") throw new Error("media tags must use 'imeta' prefix");
    tags.push(t.map(String));
  }
  return publishNow(1, tags, content);
}
async function getNoteReactions(a: Args) {
  const ids: string[] = (a?.noteIds ?? []).map(noteId);
  if (!ids.length) return [];
  if (ids.length > 200) throw new Error(`too many note ids (max 200, got ${ids.length})`);
  const reactions = await queryRelay([{ kinds: [7], "#e": ids, limit: 500 }]);
  const dels = reactions.length ? await queryRelay([{ kinds: [5], "#e": reactions.map((r) => r.id), limit: 500 }]) : [];
  const deleted = new Set(dels.flatMap(eTags));
  const targets = new Set(ids);
  const by = new Map<string, Set<string>>();
  for (const r of reactions) {
    if (deleted.has(r.id)) continue;
    const target = [...eTags(r)].reverse().find((id) => targets.has(id));
    if (!target) continue;
    const emoji = r.content || "+";
    const k = `${target}\0${emoji}`;
    if (!by.has(k)) by.set(k, new Set());
    by.get(k)?.add(r.pubkey);
  }
  return [...by]
    .map(([k, s]) => {
      const [note_id, emoji] = k.split("\0");
      const pubkeys = [...s].sort();
      return { note_id, emoji, count: pubkeys.length, pubkeys };
    })
    .sort((x, y) => (x.note_id < y.note_id ? -1 : x.note_id > y.note_id ? 1 : x.emoji < y.emoji ? -1 : x.emoji > y.emoji ? 1 : 0));
}
async function getLikedNotes(a: Args) {
  const author = String(a?.authorPubkey ?? "");
  const cap = Math.min(a?.limit ?? 50, 200);
  const reactions = (await queryRelay([{ kinds: [7], authors: [author], limit: Math.min(cap * 4, 1000) }])).sort((x, y) => y.created_at - x.created_at);
  const dels = reactions.length ? await queryRelay([{ kinds: [5], authors: [author], "#e": reactions.map((r) => r.id), limit: 500 }]) : [];
  const deleted = new Set(dels.flatMap(eTags));
  const targets: string[] = [];
  const likedAt = new Map<string, number>();
  for (const r of reactions) {
    if (targets.length >= cap) break;
    if (deleted.has(r.id)) continue;
    const t = eTags(r).pop();
    if (t && !likedAt.has(t)) (likedAt.set(t, r.created_at), targets.push(t));
  }
  if (!targets.length) return { notes: [], next_cursor: null };
  const r = userNotes(await queryRelay([{ kinds: [1], ids: targets, limit: cap }]));
  r.notes.sort((x, y) => (likedAt.get(y.id) ?? 0) - (likedAt.get(x.id) ?? 0));
  r.notes = r.notes.slice(0, cap);
  return r;
}
async function getNotesTimeline(a: Args) {
  const pks: string[] = a?.pubkeys ?? [];
  if (!pks.length) return { notes: [], next_cursor: null };
  if (pks.length > 100) throw new Error(`too many pubkeys (max 100, got ${pks.length})`);
  const per = Math.min(a?.limitPerUser ?? 10, 50);
  const evs = await queryRelay([{ kinds: [1], authors: pks, limit: Math.min(per * pks.length, 200) }]);
  const notes = evs.map(noteInfo).sort((x, y) => y.created_at - x.created_at).slice(0, 200);
  return { notes, next_cursor: null };
}

// ── 10. Team catalog (kind 30178, read-only projection) ─────────────────────
const KIND_TEAM_CATALOG = 30178;
const CATALOG_PAGE = 500;
const MAX_PAGES = 40;
function isShared(ev: NostrEvent) {
  let n = 0;
  for (const t of ev.tags)
    if (t[0] === "shared") {
      if (t.length !== 2 || t[1] !== "true") return false;
      n++;
    }
  return n === 1;
}
const CONCEALED = /[\u200B-\u200F\u202A-\u202E\u2060-\u2064\u2066-\u2069\uFEFF]/;
export function teamCatalogContent(ev: NostrEvent) {
  const c = JSON.parse(ev.content);
  if (!c || c.v !== 1) throw new Error("unsupported team catalog schema version");
  const s = (v: unknown) => (typeof v === "string" ? v : null);
  if (typeof c.name !== "string" || !c.name.trim() || CONCEALED.test(c.name) || /[\r\n\t]/.test(c.name)) throw new Error("invalid team name");
  if (!Array.isArray(c.members)) throw new Error("invalid members");
  const keys = new Set<string>();
  const members = c.members.map((m: Record<string, unknown>) => {
    if (typeof m?.member_key !== "string" || typeof m.display_name !== "string" || !m.display_name.trim()) throw new Error("invalid member");
    if (keys.has(m.member_key)) throw new Error("duplicate member key");
    keys.add(m.member_key);
    if (CONCEALED.test(m.display_name)) throw new Error("invalid member name");
    return {
      memberKey: m.member_key, displayName: m.display_name, systemPrompt: s(m.system_prompt) ?? "", avatarUrl: s(m.avatar_url),
      runtime: s(m.runtime), model: s(m.model), provider: s(m.provider),
    };
  });
  return { name: c.name as string, description: s(c.description), instructions: s(c.instructions), members };
}
export function publicationsFromEvents(events: NostrEvent[]) {
  const sorted = [...events].sort((x, y) => y.created_at - x.created_at || (x.id < y.id ? -1 : 1));
  const claimed = new Set<string>();
  const out = [];
  for (const ev of sorted) {
    if (ev.kind !== KIND_TEAM_CATALOG) continue;
    const d = ev.tags.filter((t) => t[0] === "d" && t.length >= 2);
    if (d.length !== 1 || !d[0][1]) continue;
    const owner = ev.pubkey.toLowerCase();
    const coord = `${owner}\0${d[0][1]}`;
    if (claimed.has(coord)) continue;
    claimed.add(coord);
    if (!isShared(ev)) continue;
    try {
      const c = teamCatalogContent(ev);
      out.push({ eventId: ev.id, ownerPubkey: owner, teamDTag: d[0][1], ...c });
    } catch {
      /* skip invalid publication */
    }
  }
  return out;
}
async function fetchTeamCatalog() {
  const owner = me();
  const byId = new Map<string, NostrEvent>();
  let until: number | undefined;
  for (let i = 0; i < MAX_PAGES; i++) {
    const f: Record<string, unknown> = { kinds: [KIND_TEAM_CATALOG], limit: CATALOG_PAGE };
    if (until != null) f.until = until;
    const page = await queryRelay([f]);
    const verified = page.filter((e) => verifyEvent(e as never));
    for (const e of verified) byId.set(e.id, e);
    if (page.length < CATALOG_PAGE) {
      if (signerPubkey() !== owner) throw new Error("team catalog scope changed while fetching");
      return publicationsFromEvents([...byId.values()]);
    }
    if (!verified.length) throw new Error("team catalog returned a full page with no verifiable events; cannot safely advance the cursor");
    const oldest = Math.min(...verified.map((e) => e.created_at));
    if (until != null && oldest >= until) throw new Error(`team catalog has more than one page of events at created_at ${oldest}`);
    until = oldest;
  }
  throw new Error(`team catalog exceeds the ${MAX_PAGES} page fetch budget`);
}

export const parity2Handlers: Record<string, (a: Args) => unknown> = {
  resolve_oa_owner: resolveOaOwner,
  // workflows
  get_channel_workflows: async (a) => (await queryRelay([{ kinds: [30620], "#h": [a?.channelId] }])).map(workflowFromEvent),
  get_channels_workflows: getChannelsWorkflows,
  get_workflow: async (a) => {
    const ev = (await queryRelay([{ kinds: [30620], "#d": [a?.workflowId], limit: 1 }]))[0];
    if (!ev) throw new Error("workflow not found");
    return workflowFromEvent(ev);
  },
  get_workflow_runs: async (a) => {
    const id = uuidArg(a?.workflowId, "workflow id");
    const limit = Math.max(1, Math.min(a?.limit ?? 20, 100));
    return getRelayJson(`/workflows/${id}/runs?limit=${limit}`);
  },
  create_workflow: createWorkflow,
  update_workflow: updateWorkflow,
  delete_workflow: async (a) => {
    await publishNow(5, [["a", `30620:${me()}:${String(a?.workflowId ?? "")}`]]);
  },
  trigger_workflow: triggerWorkflow,
  get_run_approvals: async (a) =>
    getRelayJson(`/workflows/${uuidArg(a?.workflowId, "workflow id")}/runs/${uuidArg(a?.runId, "workflow run id")}/approvals`),
  grant_approval: approvalAction(46030),
  deny_approval: approvalAction(46031),
  // canvas
  get_canvas: getCanvas,
  set_canvas: setCanvas,
  get_canvas_history: getCanvasHistory,
  // forum
  get_forum_posts: getForumPosts,
  get_forum_thread: getForumThread,
  // notes
  publish_note: publishNote,
  get_global_notes: async (a) => {
    const f: Record<string, unknown> = { kinds: [1], limit: Math.min(a?.limit ?? 50, 200) };
    if (a?.before != null) f.until = a.before;
    return userNotes(await queryRelay([f]));
  },
  get_user_notes: async (a) => {
    const f: Record<string, unknown> = { kinds: [1], authors: [a?.pubkey], limit: Math.min(a?.limit ?? 20, 100) };
    if (a?.before != null) f.until = a.before;
    return userNotes(await queryRelay([f]));
  },
  get_note: async (a) => {
    const ev = (await queryRelay([{ kinds: [1], ids: [noteId(a?.noteId)], limit: 1 }]))[0];
    return ev ? noteInfo(ev) : null;
  },
  get_note_reactions: getNoteReactions,
  get_liked_notes: getLikedNotes,
  get_notes_timeline: getNotesTimeline,
  get_contact_list: async (a) => {
    const ev = (await queryRelay([{ kinds: [3], authors: [a?.pubkey], limit: 1 }]))[0];
    return ev
      ? { id: ev.id, pubkey: ev.pubkey, created_at: ev.created_at, tags: ev.tags, content: ev.content }
      : { id: "", pubkey: a?.pubkey, created_at: 0, tags: [], content: "" };
  },
  // team catalog (display only; adding a team writes the desktop's local store → over Direct)
  fetch_team_catalog: fetchTeamCatalog,
};
export { ownersFor };
