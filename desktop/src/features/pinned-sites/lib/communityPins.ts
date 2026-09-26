import { KIND_COMMUNITY_PINNED_SITES } from "@/shared/constants/kinds";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

import { isPinnedSiteIconId } from "./icons";
import type { PinnedSite, PinnedSiteIconId } from "./types";
import { normalizePinnedSiteName, normalizePinnedSiteUrl } from "./url";

export const COMMUNITY_PINNED_SITES_D_TAG = "buzz:community-pins";

export type CommunityPinnedSitesPayload = {
  version: 1;
  pins: Array<{
    id: string;
    name: string;
    url: string;
    icon: PinnedSiteIconId;
    pollForChanges?: boolean;
    openMatchingLinks?: boolean;
  }>;
};

function parseCommunityPin(value: unknown): PinnedSite | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.id !== "string" || candidate.id.trim().length === 0) {
    return null;
  }
  const name = normalizePinnedSiteName(String(candidate.name ?? ""));
  const url = normalizePinnedSiteUrl(String(candidate.url ?? ""));
  if (!name || !url || !isPinnedSiteIconId(candidate.icon)) {
    return null;
  }
  return {
    id: candidate.id,
    name,
    url,
    icon: candidate.icon,
    pollForChanges: candidate.pollForChanges === true,
    openMatchingLinks: candidate.openMatchingLinks !== false,
    scope: "community",
  };
}

export function parseCommunityPinnedSitesPayload(
  content: string,
): PinnedSite[] {
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
    if (candidate.version !== 1 || !Array.isArray(candidate.pins)) {
      return [];
    }
    const pins: PinnedSite[] = [];
    const seen = new Set<string>();
    for (const entry of candidate.pins) {
      const pin = parseCommunityPin(entry);
      if (!pin || seen.has(pin.id)) continue;
      seen.add(pin.id);
      pins.push(pin);
    }
    return pins;
  } catch {
    return [];
  }
}

/** Latest admin-authored list wins (NIP-33 LWW by created_at). */
export function selectLatestCommunityPins(
  events: ReadonlyArray<RelayEvent>,
): PinnedSite[] {
  let latest: RelayEvent | null = null;
  for (const event of events) {
    if (event.kind !== KIND_COMMUNITY_PINNED_SITES) continue;
    if (!latest || event.created_at > latest.created_at) {
      latest = event;
    }
  }
  return latest ? parseCommunityPinnedSitesPayload(latest.content) : [];
}

export async function fetchCommunityPinnedSites(): Promise<PinnedSite[]> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_COMMUNITY_PINNED_SITES],
    "#d": [COMMUNITY_PINNED_SITES_D_TAG],
    limit: 50,
  });
  return selectLatestCommunityPins(events);
}

export async function publishCommunityPinnedSites(
  pins: ReadonlyArray<PinnedSite>,
): Promise<void> {
  const payload: CommunityPinnedSitesPayload = {
    version: 1,
    pins: pins.map((pin) => ({
      id: pin.id,
      name: pin.name,
      url: pin.url,
      icon: pin.icon,
      pollForChanges: pin.pollForChanges,
      openMatchingLinks: pin.openMatchingLinks !== false,
    })),
  };
  const event = await signRelayEvent({
    kind: KIND_COMMUNITY_PINNED_SITES,
    content: JSON.stringify(payload),
    tags: [["d", COMMUNITY_PINNED_SITES_D_TAG]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while saving community pins.",
    "Failed to save community pins.",
  );
}
