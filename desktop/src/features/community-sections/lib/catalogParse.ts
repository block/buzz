import { KIND_COMMUNITY_SECTIONS } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

import type { CommunitySection } from "./types";
import {
  MAX_COMMUNITY_SECTIONS,
  MAX_SECTION_CHANNELS,
  MAX_SECTION_NAME_LEN,
} from "./types";

export const COMMUNITY_SECTIONS_D_TAG = "buzz:community-sections";

export function isSafeCommunitySectionId(value: string): boolean {
  return (
    value.length > 0 && value.length <= 80 && /^[A-Za-z0-9_-]+$/.test(value)
  );
}

function parseSection(value: unknown): CommunitySection | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const id = typeof candidate.id === "string" ? candidate.id.trim() : "";
  const name = typeof candidate.name === "string" ? candidate.name.trim() : "";
  if (
    !isSafeCommunitySectionId(id) ||
    !name ||
    name.length > MAX_SECTION_NAME_LEN
  ) {
    return null;
  }
  const order =
    typeof candidate.order === "number" && Number.isFinite(candidate.order)
      ? candidate.order
      : 0;
  const icon =
    typeof candidate.icon === "string" && candidate.icon.trim().length > 0
      ? candidate.icon.trim().slice(0, 80)
      : undefined;
  const rawIds = Array.isArray(candidate.channelIds)
    ? candidate.channelIds
    : [];
  const channelIds: string[] = [];
  const seen = new Set<string>();
  for (const entry of rawIds) {
    if (typeof entry !== "string") continue;
    const channelId = entry.trim();
    if (!isSafeCommunitySectionId(channelId) || seen.has(channelId)) continue;
    seen.add(channelId);
    channelIds.push(channelId);
    if (channelIds.length >= MAX_SECTION_CHANNELS) break;
  }
  return { id, name, ...(icon ? { icon } : {}), order, channelIds };
}

export function parseCommunitySectionsPayload(
  content: string,
): CommunitySection[] {
  try {
    const parsed = JSON.parse(content) as unknown;
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return [];
    }
    const candidate = parsed as Record<string, unknown>;
    if (candidate.version !== 1 || !Array.isArray(candidate.sections)) {
      return [];
    }
    const sections: CommunitySection[] = [];
    const seen = new Set<string>();
    for (const entry of candidate.sections) {
      const section = parseSection(entry);
      if (!section || seen.has(section.id)) continue;
      seen.add(section.id);
      sections.push(section);
      if (sections.length >= MAX_COMMUNITY_SECTIONS) break;
    }
    return sections.sort((a, b) => a.order - b.order);
  } catch {
    return [];
  }
}

/** Latest admin-authored catalog wins (NIP-33 LWW by created_at). */
export function selectLatestCommunitySections(
  events: ReadonlyArray<RelayEvent>,
): CommunitySection[] {
  let latest: RelayEvent | null = null;
  for (const event of events) {
    if (event.kind !== KIND_COMMUNITY_SECTIONS) continue;
    if (!latest || event.created_at > latest.created_at) {
      latest = event;
    }
  }
  return latest ? parseCommunitySectionsPayload(latest.content) : [];
}

export function mergeCommunitySections(
  remote: ReadonlyArray<CommunitySection>,
  local: ReadonlyArray<CommunitySection>,
): CommunitySection[] {
  if (remote.length > 0) return [...remote].sort((a, b) => a.order - b.order);
  return [...local].sort((a, b) => a.order - b.order);
}

export function createCommunitySectionId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `section-${Date.now().toString(36)}`;
}

/**
 * Official Buzz relays that predate kind:30625 reject with
 * `restricted: unknown event kind` (same gap as community bots 30624 /
 * pins 30623). Surface a deploy-oriented message instead of the raw OK.
 */
export function isUnknownCommunitySectionsKindError(error: unknown): boolean {
  const message = (
    error instanceof Error ? error.message : String(error)
  ).toLowerCase();
  return (
    message.includes("unknown event kind") ||
    message.includes("unknown kind") ||
    message.includes("kind-not-allowed") ||
    message.includes("kind not allowed")
  );
}

export const COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE =
  "This community relay does not accept community sections yet (kind 30625). Deploy a relay build that includes KIND_COMMUNITY_SECTIONS — see docs/community-sections.md (Deploy).";
