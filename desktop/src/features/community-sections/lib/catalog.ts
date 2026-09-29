import { KIND_COMMUNITY_SECTIONS } from "@/shared/constants/kinds";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

import { ensureExclusiveChannelMembership } from "./sectionAdmin";
import type { CommunitySection, CommunitySectionsPayload } from "./types";
import {
  MAX_COMMUNITY_SECTIONS,
  MAX_SECTION_CHANNELS,
  MAX_SECTION_NAME_LEN,
} from "./types";
import {
  COMMUNITY_SECTIONS_D_TAG,
  COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE,
  isSafeCommunitySectionId,
  isUnknownCommunitySectionsKindError,
  mergeCommunitySections,
  parseCommunitySectionsPayload,
  selectLatestCommunitySections,
} from "./catalogParse";

export {
  COMMUNITY_SECTIONS_D_TAG,
  COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE,
  isUnknownCommunitySectionsKindError,
  mergeCommunitySections,
  parseCommunitySectionsPayload,
  selectLatestCommunitySections,
} from "./catalogParse";

const LOCAL_KEY_PREFIX = "buzz-community-sections.v1";

export function communitySectionsStorageKey(relayUrl: string): string {
  return `${LOCAL_KEY_PREFIX}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function loadLocalCommunitySections(
  relayUrl: string,
): CommunitySection[] {
  const raw = getStorageItem(communitySectionsStorageKey(relayUrl));
  if (!raw) return [];
  return parseCommunitySectionsPayload(raw);
}

export function saveLocalCommunitySections(
  relayUrl: string,
  sections: ReadonlyArray<CommunitySection>,
): void {
  const payload: CommunitySectionsPayload = {
    version: 1,
    sections: sections.map((section) => ({
      id: section.id,
      name: section.name,
      ...(section.icon ? { icon: section.icon } : {}),
      order: section.order,
      channelIds: [...section.channelIds],
    })),
  };
  setStorageItem(
    communitySectionsStorageKey(relayUrl),
    JSON.stringify(payload),
  );
}

export async function fetchCommunitySections(
  relayUrl?: string,
): Promise<CommunitySection[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_COMMUNITY_SECTIONS],
    "#d": [COMMUNITY_SECTIONS_D_TAG],
    limit: 50,
  });
  const remote = selectLatestCommunitySections(events);
  if (relayUrl) {
    const merged = mergeCommunitySections(
      remote,
      loadLocalCommunitySections(relayUrl),
    );
    if (remote.length > 0) {
      saveLocalCommunitySections(relayUrl, remote);
    }
    return merged;
  }
  return remote;
}

export async function publishCommunitySections(
  sections: ReadonlyArray<CommunitySection>,
  relayUrl?: string,
): Promise<void> {
  if (sections.length > MAX_COMMUNITY_SECTIONS) {
    throw new Error(`At most ${MAX_COMMUNITY_SECTIONS} community sections.`);
  }
  const exclusive = ensureExclusiveChannelMembership(sections);
  const payload: CommunitySectionsPayload = {
    version: 1,
    sections: exclusive.map((section, index) => ({
      id: section.id,
      name: section.name.trim(),
      ...(section.icon?.trim() ? { icon: section.icon.trim() } : {}),
      order: Number.isFinite(section.order) ? section.order : index,
      channelIds: section.channelIds
        .filter(isSafeCommunitySectionId)
        .slice(0, MAX_SECTION_CHANNELS),
    })),
  };
  for (const section of payload.sections) {
    if (!isSafeCommunitySectionId(section.id)) {
      throw new Error("Section id is invalid.");
    }
    if (!section.name || section.name.length > MAX_SECTION_NAME_LEN) {
      throw new Error(
        `Section name is required (max ${MAX_SECTION_NAME_LEN} characters).`,
      );
    }
  }
  const event = await signRelayEvent({
    kind: KIND_COMMUNITY_SECTIONS,
    content: JSON.stringify(payload),
    tags: [["d", COMMUNITY_SECTIONS_D_TAG]],
  });
  try {
    await relayClient.publishEvent(
      event,
      "Timed out while saving community sections.",
      "Failed to save community sections.",
    );
  } catch (error) {
    if (isUnknownCommunitySectionsKindError(error)) {
      throw new Error(COMMUNITY_SECTIONS_RELAY_UNSUPPORTED_MESSAGE);
    }
    throw error;
  }
  if (relayUrl) {
    saveLocalCommunitySections(relayUrl, payload.sections);
  }
}
