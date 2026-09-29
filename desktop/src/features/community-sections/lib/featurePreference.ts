import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

const STORAGE_KEY_PREFIX = "buzz-community-sections-enabled.v1";
export const COMMUNITY_SECTIONS_ENABLED_EVENT =
  "buzz:community-sections-enabled-change";

export type CommunitySectionsEnabledChange = {
  pubkey: string;
  relayUrl: string;
  enabled: boolean;
};

/** Default ON so existing opt-in subscribe behavior stays active until the member turns the master switch off. */
export const DEFAULT_COMMUNITY_SECTIONS_ENABLED = true;

export function communitySectionsEnabledStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey.toLowerCase()}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function readCommunitySectionsEnabled(
  pubkey: string,
  relayUrl: string,
): boolean {
  if (!pubkey || !relayUrl) return DEFAULT_COMMUNITY_SECTIONS_ENABLED;
  try {
    const raw = getStorageItem(
      communitySectionsEnabledStorageKey(pubkey, relayUrl),
    );
    if (raw === null || raw === undefined || raw === "") {
      return DEFAULT_COMMUNITY_SECTIONS_ENABLED;
    }
    if (raw === "0" || raw === "false") return false;
    if (raw === "1" || raw === "true") return true;
    return DEFAULT_COMMUNITY_SECTIONS_ENABLED;
  } catch {
    return DEFAULT_COMMUNITY_SECTIONS_ENABLED;
  }
}

export function writeCommunitySectionsEnabled(
  pubkey: string,
  relayUrl: string,
  enabled: boolean,
): boolean {
  if (!pubkey || !relayUrl) return false;
  try {
    setStorageItem(
      communitySectionsEnabledStorageKey(pubkey, relayUrl),
      enabled ? "1" : "0",
    );
    return true;
  } catch {
    return false;
  }
}

export function notifyCommunitySectionsEnabledChange(
  detail: CommunitySectionsEnabledChange,
): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent(COMMUNITY_SECTIONS_ENABLED_EVENT, { detail }),
  );
}
