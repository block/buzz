import { profileValidOaOwner } from "./nipOa";
// TS port of the pure converters in desktop/src-tauri/src/nostr_convert.rs (S1 subset: channels, members, profiles).
import type { NostrEvent } from "./nostrTypes";

export const firstTag = (e: NostrEvent, name: string): string | undefined =>
  e.tags.find((t) => t.length >= 2 && t[0] === name)?.[1];
const hasTag = (e: NostrEvent, name: string) =>
  e.tags.some((t) => t[0] === name);
export const dTag = (e: NostrEvent) => firstTag(e, "d");

export const timestampToIso = (secs: number): string =>
  new Date(secs * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");

export type ChannelInfo = {
  id: string;
  name: string;
  channel_type: string;
  visibility: string;
  description: string;
  topic: string | null;
  purpose: string | null;
  member_count: number;
  member_pubkeys: string[];
  last_message_at: string | null;
  archived_at: string | null;
  participants: string[];
  participant_pubkeys: string[];
  is_member: boolean;
  ttl_seconds: number | null;
  ttl_deadline: string | null;
};

function channelType(e: NostrEvent) {
  return firstTag(e, "t") ?? (hasTag(e, "hidden") ? "dm" : "stream");
}
function visibility(e: NostrEvent) {
  const v = firstTag(e, "visibility");
  if (hasTag(e, "public") || v === "open") return "open";
  if (hasTag(e, "private") || v === "private") return "private";
  return "open";
}
const intOrNull = (s: string | undefined) =>
  s !== undefined && /^-?\d+$/.test(s) ? Number.parseInt(s, 10) : null;

export function channelInfoFromEvent(
  e: NostrEvent,
  isMember = true,
): ChannelInfo | null {
  const id = dTag(e);
  if (!id) return null;
  const participants = e.tags
    .filter((t) => t[0] === "p" && t[1])
    .map((t) => t[1]);
  return {
    id,
    name: firstTag(e, "name") ?? "",
    channel_type: channelType(e),
    visibility: visibility(e),
    description: firstTag(e, "about") ?? "",
    topic: firstTag(e, "topic") ?? null,
    purpose: firstTag(e, "purpose") ?? null,
    member_count: 0,
    member_pubkeys: [],
    last_message_at: null,
    archived_at:
      firstTag(e, "archived") === "true" ? timestampToIso(e.created_at) : null,
    participants,
    participant_pubkeys: [...participants],
    is_member: isMember,
    ttl_seconds: intOrNull(firstTag(e, "ttl")),
    ttl_deadline: firstTag(e, "ttl_deadline") ?? null,
  };
}

export function channelDetailFromEvent(e: NostrEvent) {
  const id = dTag(e);
  if (!id) throw new Error("kind:39000 missing required `d` tag");
  const iso = timestampToIso(e.created_at);
  return {
    id,
    name: firstTag(e, "name") ?? "",
    channel_type: channelType(e),
    visibility: visibility(e),
    description: firstTag(e, "about") ?? "",
    topic: firstTag(e, "topic") ?? null,
    topic_set_by: null,
    topic_set_at: null,
    purpose: firstTag(e, "purpose") ?? null,
    purpose_set_by: null,
    purpose_set_at: null,
    created_by: e.pubkey,
    created_at: iso,
    updated_at: iso,
    archived_at: firstTag(e, "archived") === "true" ? iso : null,
    member_count: 0,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    ttl_seconds: intOrNull(firstTag(e, "ttl")),
    ttl_deadline: firstTag(e, "ttl_deadline") ?? null,
  };
}

export type ChannelMember = {
  pubkey: string;
  role: string;
  is_agent: boolean;
  joined_at: string | null;
  display_name: string | null;
};
export function channelMembersFromEvent(e: NostrEvent): {
  members: ChannelMember[];
  next_cursor: null;
} {
  if (!dTag(e)) throw new Error("kind:39002 missing required `d` tag");
  const seen = new Set<string>();
  const members: ChannelMember[] = [];
  for (const t of e.tags) {
    if (t[0] !== "p" || !t[1] || seen.has(t[1])) continue;
    seen.add(t[1]);
    const role = t[3] ? t[3] : "member";
    members.push({
      pubkey: t[1],
      role,
      is_agent: role === "bot",
      joined_at: null,
      display_name: null,
    });
  }
  return { members, next_cursor: null };
}

function parseJson(s: string): Record<string, unknown> {
  try {
    const v = JSON.parse(s);
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
const str = (v: unknown) => (typeof v === "string" ? v : null);

/** owner_pubkey: verified NIP-OA owner (profile_valid_oa_owner_pubkey port, nipOa.ts). */
export function profileInfoFromEvent(e: NostrEvent) {
  const v = parseJson(e.content);
  return {
    pubkey: e.pubkey,
    display_name: str(v.display_name) ?? str(v.name),
    avatar_url: str(v.picture),
    about: str(v.about),
    nip05_handle: str(v.nip05),
    owner_pubkey: profileValidOaOwner(e) as string | null,
    has_profile_event: true,
  };
}
export const emptyProfile = (pubkey: string) => ({
  pubkey,
  display_name: null,
  avatar_url: null,
  about: null,
  nip05_handle: null,
  owner_pubkey: null,
  has_profile_event: false,
});

export function usersBatchFromEvents(
  events: NostrEvent[],
  requested: string[],
) {
  const latest = new Map<string, NostrEvent>();
  for (const ev of events) {
    const prev = latest.get(ev.pubkey);
    if (!prev || ev.created_at > prev.created_at) latest.set(ev.pubkey, ev);
  }
  const profiles: Record<string, unknown> = {};
  for (const [pk, ev] of latest) {
    const v = parseJson(ev.content);
    profiles[pk] = {
      display_name: str(v.display_name) ?? str(v.name),
      name: str(v.name),
      avatar_url: str(v.picture),
      nip05_handle: str(v.nip05),
      owner_pubkey: profileValidOaOwner(ev),
      is_agent: profileValidOaOwner(ev) !== null,
    };
  }
  return { profiles, missing: requested.filter((pk) => !(pk in profiles)) };
}
