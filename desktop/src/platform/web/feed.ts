// Web get_feed: port of desktop src-tauri/src/commands/messages.rs `get_feed` (mentions + needs-action approvals,
// with kind-40003 edits applying link-preview suppression). activity / agent_activity stay empty, as on desktop.
// Batched for remote signers: one relay query carries both the mention and approval filters, and a second one
// fetches edits for the found mentions — at most 2 signed requests, each well under the 10-filter cap.
// Edits by the author or the author's verified NIP-OA owner suppress previews (owner check: nipOa.ts), as on desktop.

type Ev = { id: string; kind: number; pubkey: string; content: string; created_at: number; tags: string[][] };
type Filter = Record<string, unknown>;
export type FeedArgs = { since?: number; limit?: number; types?: string } | undefined;

export const MENTION_KINDS = [9, 40002, 1, 45001, 45003, 1618, 1619, 1621, 1630, 1631, 1632, 1633];
export const APPROVAL_KINDS = [46010, 46011, 46012];
const EDIT_KIND = 40003;

function feedItem(ev: Ev, category: "mention" | "needs_action", suppressed: Set<string>) {
  const tags = ev.tags.map((t) => [...t]);
  if (suppressed.has(ev.id) && !tags.some((t) => t[0] === "link-preview" && t[1] === "none" && t.length === 2))
    tags.push(["link-preview", "none"]);
  return {
    id: ev.id,
    kind: ev.kind,
    pubkey: ev.pubkey,
    content: ev.content,
    created_at: ev.created_at,
    channel_id: ev.tags.find((t) => t[0] === "h" && t.length >= 2)?.[1] ?? null,
    channel_name: "",
    channel_type: null,
    tags,
    category,
  };
}

export async function getFeed(
  args: FeedArgs,
  myPubkey: string,
  query: (filters: Filter[]) => Promise<Ev[]>,
  ownerOf: (profiles: Ev[]) => Map<string, string> = () => new Map(),
) {
  const since = args?.since;
  const cap = Math.min(args?.limit ?? 50, 100);
  const types = args?.types;
  const wants = (t: string) => (types == null ? true : types.split(",").some((s) => s.trim() === t));
  const wantMentions = wants("mentions");
  const wantNeedsAction = wants("needs_action");
  const withSince = (f: Filter) => (since != null ? { ...f, since } : f);

  const filters: Filter[] = [];
  if (wantMentions) filters.push(withSince({ kinds: MENTION_KINDS, "#p": [myPubkey], limit: cap }));
  if (wantNeedsAction) filters.push(withSince({ kinds: APPROVAL_KINDS, "#p": [myPubkey], limit: 20 }));
  const events = filters.length ? await query(filters).catch(() => [] as Ev[]) : [];

  const seen = new Set<string>();
  const unique = events.filter((e) => !seen.has(e.id) && seen.add(e.id));
  const byNewest = (a: Ev, b: Ev) => b.created_at - a.created_at;
  const mentionEvents = wantMentions
    ? unique.filter((e) => MENTION_KINDS.includes(e.kind) && e.tags.some((t) => t[0] === "p" && t[1] === myPubkey)).sort(byNewest).slice(0, cap)
    : [];
  const approvalEvents = wantNeedsAction
    ? unique.filter((e) => APPROVAL_KINDS.includes(e.kind)).sort(byNewest).slice(0, 20)
    : [];

  const mentionIds = mentionEvents.map((e) => e.id);
  const authors = [...new Set(mentionEvents.map((e) => e.pubkey))];
  // One request: edits of the mentions + the mention authors' profiles (for NIP-OA owner edits).
  const extra = mentionIds.length
    ? await query([{ kinds: [EDIT_KIND], "#e": mentionIds }, { kinds: [0], authors }]).catch(() => [] as Ev[])
    : [];
  const edits = extra.filter((e) => e.kind === EDIT_KIND);
  const owners = ownerOf(extra.filter((e) => e.kind === 0));
  const byId = new Map(mentionEvents.map((e) => [e.id, e]));
  const suppressed = new Set<string>();
  for (const edit of edits) {
    if (edit.kind !== EDIT_KIND) continue;
    if (!edit.tags.some((t) => t.length === 2 && t[0] === "link-preview" && t[1] === "none")) continue;
    const target = edit.tags.find((t) => t[0] === "e")?.[1];
    const original = target ? byId.get(target) : undefined;
    if (original && (original.pubkey === edit.pubkey || owners.get(original.pubkey) === edit.pubkey)) suppressed.add(original.id);
  }

  const mentions = mentionEvents.map((e) => feedItem(e, "mention", suppressed));
  const needs_action = approvalEvents.map((e) => feedItem(e, "needs_action", suppressed));
  return {
    feed: { mentions, needs_action, activity: [], agent_activity: [] },
    meta: { since: since ?? 0, total: mentions.length + needs_action.length, generated_at: Math.floor(Date.now() / 1000) },
  };
}
