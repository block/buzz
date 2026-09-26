import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

import { isPinnedSiteIconId } from "./icons";
import {
  WAYFINDER_PIN,
  WAYFINDER_PIN_ID,
  type PinnedSite,
  type PinnedSiteIconId,
} from "./types";
import { normalizePinnedSiteName, normalizePinnedSiteUrl } from "./url";

const STORAGE_KEY_PREFIX = "buzz-pinned-sites.v1";

export type PersonalPinnedSitesBlob = {
  version: 1;
  wayfinderSeeded: boolean;
  pins: PinnedSite[];
};

function emptyBlob(): PersonalPinnedSitesBlob {
  return {
    version: 1,
    wayfinderSeeded: false,
    pins: [],
  };
}

export function personalPinnedSitesStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

function parsePin(value: unknown): PinnedSite | null {
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
    icon: candidate.icon as PinnedSiteIconId,
    pollForChanges: candidate.pollForChanges === true,
    // Missing field (legacy blobs) defaults to on.
    openMatchingLinks: candidate.openMatchingLinks !== false,
    scope: "personal",
  };
}

export function parsePersonalPinnedSitesBlob(
  value: unknown,
): PersonalPinnedSitesBlob | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (candidate.version !== 1 || !Array.isArray(candidate.pins)) {
    return null;
  }
  const pins: PinnedSite[] = [];
  const seen = new Set<string>();
  for (const entry of candidate.pins) {
    const pin = parsePin(entry);
    if (!pin || seen.has(pin.id)) continue;
    seen.add(pin.id);
    pins.push(pin);
  }
  return {
    version: 1,
    wayfinderSeeded: candidate.wayfinderSeeded === true,
    pins,
  };
}

export function seedWayfinderIfNeeded(
  blob: PersonalPinnedSitesBlob,
): PersonalPinnedSitesBlob {
  if (blob.wayfinderSeeded) {
    return blob;
  }
  const hasWayfinder = blob.pins.some((pin) => pin.id === WAYFINDER_PIN_ID);
  return {
    version: 1,
    wayfinderSeeded: true,
    pins: hasWayfinder ? blob.pins : [...blob.pins, { ...WAYFINDER_PIN }],
  };
}

export function loadPersonalPinnedSites(
  pubkey: string,
  relayUrl: string,
): PersonalPinnedSitesBlob {
  const raw = getStorageItem(personalPinnedSitesStorageKey(pubkey, relayUrl));
  if (!raw) {
    return seedWayfinderIfNeeded(emptyBlob());
  }
  try {
    const parsed = parsePersonalPinnedSitesBlob(JSON.parse(raw));
    if (!parsed) {
      return seedWayfinderIfNeeded(emptyBlob());
    }
    return seedWayfinderIfNeeded(parsed);
  } catch {
    return seedWayfinderIfNeeded(emptyBlob());
  }
}

export function savePersonalPinnedSites(
  pubkey: string,
  relayUrl: string,
  blob: PersonalPinnedSitesBlob,
): void {
  setStorageItem(
    personalPinnedSitesStorageKey(pubkey, relayUrl),
    JSON.stringify({
      version: 1,
      wayfinderSeeded: true,
      pins: blob.pins.map((pin) => ({
        id: pin.id,
        name: pin.name,
        url: pin.url,
        icon: pin.icon,
        pollForChanges: pin.pollForChanges,
        openMatchingLinks: pin.openMatchingLinks !== false,
      })),
    }),
  );
}

export function upsertPersonalPin(
  blob: PersonalPinnedSitesBlob,
  pin: PinnedSite,
): PersonalPinnedSitesBlob {
  const next = blob.pins.filter((entry) => entry.id !== pin.id);
  next.push({ ...pin, scope: "personal" });
  return {
    version: 1,
    wayfinderSeeded: true,
    pins: next,
  };
}

export function removePersonalPin(
  blob: PersonalPinnedSitesBlob,
  pinId: string,
): PersonalPinnedSitesBlob {
  return {
    version: 1,
    wayfinderSeeded: true,
    pins: blob.pins.filter((pin) => pin.id !== pinId),
  };
}
