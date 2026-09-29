import type { Channel } from "@/shared/api/types";

const HUDDLE_NAME_SUFFIX = /\s+huddle$/i;
const HUDDLE_FALLBACK_NAME = /^huddle-[0-9a-f]{8}$/i;

const ARCHIVED_HUDDLE_DATE_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

export const ARCHIVED_HUDDLE_OTHER_GROUP_KEY = "__other__";
export const ARCHIVED_HUDDLE_OTHER_GROUP_LABEL = "Other";

type HuddleChannelShape = Pick<
  Channel,
  "id" | "name" | "ttlSeconds" | "channelType"
>;

/**
 * Huddle backing channels are ephemeral (TTL) streams named like
 * "parent huddle" or the create-fallback `huddle-<8 hex>`. Permanent
 * channels that happen to include "huddle" in the name stay ordinary.
 */
export function looksLikeHuddleBackingChannel(
  channel: HuddleChannelShape,
): boolean {
  if (channel.channelType === "forum") return false;
  if (channel.ttlSeconds === null) return false;
  const name = channel.name.trim();
  return HUDDLE_NAME_SUFFIX.test(name) || HUDDLE_FALLBACK_NAME.test(name);
}

export function isHuddleBackingChannel(
  channel: HuddleChannelShape,
  huddleBackingChannelIds: ReadonlySet<string>,
): boolean {
  return (
    huddleBackingChannelIds.has(channel.id) ||
    looksLikeHuddleBackingChannel(channel)
  );
}

export function isLiveHuddleSidebarChannel(
  channelId: string,
  revealedHuddleChannelIds: ReadonlySet<string>,
  activeHuddleChannelId: string | null | undefined,
): boolean {
  return (
    revealedHuddleChannelIds.has(channelId) ||
    channelId === activeHuddleChannelId
  );
}

/**
 * Primary left-nav lists (Channels, sections, starred): show ordinary
 * channels and explicitly revealed live huddle transcripts. Live companion
 * huddles stay hidden here; ended ones belong in Archived Huddles.
 */
export function shouldShowSidebarChannel(
  channel: HuddleChannelShape,
  huddleBackingChannelIds: ReadonlySet<string>,
  revealedHuddleChannelIds: ReadonlySet<string>,
): boolean {
  return (
    !isHuddleBackingChannel(channel, huddleBackingChannelIds) ||
    revealedHuddleChannelIds.has(channel.id)
  );
}

/**
 * Ended / abandoned huddle child channels for the "Archived Huddles" section.
 * Keeps transcripts reachable without cluttering Channels. Live sessions
 * (active or revealed) are excluded.
 */
export function isArchivedHuddleSidebarChannel(
  channel: HuddleChannelShape,
  huddleBackingChannelIds: ReadonlySet<string>,
  revealedHuddleChannelIds: ReadonlySet<string>,
  activeHuddleChannelId: string | null | undefined,
): boolean {
  if (!isHuddleBackingChannel(channel, huddleBackingChannelIds)) {
    return false;
  }
  return !isLiveHuddleSidebarChannel(
    channel.id,
    revealedHuddleChannelIds,
    activeHuddleChannelId,
  );
}

export function sortArchivedHuddleChannels(
  channels: readonly Channel[],
): Channel[] {
  return [...channels].sort((left, right) => {
    const leftAt = left.lastMessageAt ?? "";
    const rightAt = right.lastMessageAt ?? "";
    if (leftAt !== rightAt) return rightAt.localeCompare(leftAt);
    return left.name.localeCompare(right.name);
  });
}

/** Parent display name implied by a huddle backing-channel name, if any. */
export function parentLabelFromHuddleChannelName(name: string): string | null {
  const trimmed = name.trim();
  if (HUDDLE_FALLBACK_NAME.test(trimmed)) return null;
  const match = trimmed.match(/^(.*)\s+huddle$/i);
  const parent = match?.[1]?.trim() ?? "";
  return parent.length > 0 ? parent : null;
}

export type ArchivedHuddleGroup = {
  /** Parent channel id, or {@link ARCHIVED_HUDDLE_OTHER_GROUP_KEY}. */
  key: string;
  parentId: string | null;
  parentLabel: string;
  channels: Channel[];
};

function channelActivityAt(channel: Channel): string {
  return channel.lastMessageAt ?? "";
}

/**
 * Group archived huddles under their parent channel (matched by the
 * `"<parent> huddle"` naming convention). Unmatched / fallback
 * `huddle-<hex>` names land in Other. Groups and rows sort newest-first.
 */
export function groupArchivedHuddleChannels(
  archived: readonly Channel[],
  candidateParents: readonly Channel[],
): ArchivedHuddleGroup[] {
  const sorted = sortArchivedHuddleChannels(archived);
  if (sorted.length === 0) return [];

  const parentsByName = new Map<string, Channel>();
  for (const candidate of candidateParents) {
    if (candidate.ttlSeconds !== null) continue;
    const key = candidate.name.trim().toLowerCase();
    if (!key || parentsByName.has(key)) continue;
    parentsByName.set(key, candidate);
  }

  const groups = new Map<string, ArchivedHuddleGroup>();
  for (const huddle of sorted) {
    const inferred = parentLabelFromHuddleChannelName(huddle.name);
    const matched = inferred
      ? parentsByName.get(inferred.toLowerCase())
      : undefined;
    const key = matched?.id
      ? matched.id
      : inferred
        ? `name:${inferred.toLowerCase()}`
        : ARCHIVED_HUDDLE_OTHER_GROUP_KEY;
    const parentLabel = matched?.name.trim()
      ? matched.name.trim()
      : (inferred ?? ARCHIVED_HUDDLE_OTHER_GROUP_LABEL);
    const existing = groups.get(key);
    if (existing) {
      existing.channels.push(huddle);
      continue;
    }
    groups.set(key, {
      key,
      parentId: matched?.id ?? null,
      parentLabel,
      channels: [huddle],
    });
  }

  return [...groups.values()].sort((left, right) => {
    const leftAt = channelActivityAt(left.channels[0]!);
    const rightAt = channelActivityAt(right.channels[0]!);
    if (leftAt !== rightAt) return rightAt.localeCompare(leftAt);
    if (left.key === ARCHIVED_HUDDLE_OTHER_GROUP_KEY) return 1;
    if (right.key === ARCHIVED_HUDDLE_OTHER_GROUP_KEY) return -1;
    return left.parentLabel.localeCompare(right.parentLabel);
  });
}

/** Date-first row label so same-parent huddles stay distinguishable. */
export function archivedHuddleRowLabel(
  channel: Channel,
  nowMs = Date.now(),
): string {
  const raw = channel.lastMessageAt;
  if (!raw) return channel.name.trim() || "Huddle";
  const parsed = Date.parse(raw);
  if (Number.isNaN(parsed)) return channel.name.trim() || "Huddle";
  const formatted = ARCHIVED_HUDDLE_DATE_FORMATTER.format(new Date(parsed));
  const ageMs = nowMs - parsed;
  if (ageMs < 0) return formatted;
  if (ageMs < 60_000) return "Just now";
  if (ageMs < 60 * 60_000) {
    const minutes = Math.max(1, Math.floor(ageMs / 60_000));
    return `${minutes}m ago`;
  }
  if (ageMs < 24 * 60 * 60_000) {
    const hours = Math.max(1, Math.floor(ageMs / (60 * 60_000)));
    return `${hours}h ago`;
  }
  return formatted;
}
