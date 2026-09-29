import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

import type { CommunitySectionSubscriptionStore } from "./types";

const STORAGE_KEY_PREFIX = "buzz-community-section-subs.v1";
export const COMMUNITY_SECTION_SUBS_EVENT =
  "buzz:community-section-subscriptions-change";

export type CommunitySectionSubsChange = {
  relayUrl: string;
  pubkey: string;
  subscribedIds: string[];
};

export const EMPTY_SUBSCRIPTION_STORE: CommunitySectionSubscriptionStore =
  Object.freeze({
    version: 1,
    subscribed: {},
  });

export function communitySectionSubsStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey.toLowerCase()}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function parseCommunitySectionSubsPayload(
  value: unknown,
): CommunitySectionSubscriptionStore | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const candidate = value as Record<string, unknown>;
  if (candidate.version !== 1) return null;
  if (
    !candidate.subscribed ||
    typeof candidate.subscribed !== "object" ||
    Array.isArray(candidate.subscribed)
  ) {
    return null;
  }
  const subscribed: Record<string, boolean> = {};
  for (const [sectionId, flag] of Object.entries(
    candidate.subscribed as Record<string, unknown>,
  )) {
    if (!sectionId || typeof flag !== "boolean") continue;
    subscribed[sectionId] = flag;
  }
  return { version: 1, subscribed };
}

export function readCommunitySectionSubs(
  pubkey: string,
  relayUrl: string,
): CommunitySectionSubscriptionStore {
  if (!pubkey || !relayUrl) return EMPTY_SUBSCRIPTION_STORE;
  try {
    const raw = getStorageItem(
      communitySectionSubsStorageKey(pubkey, relayUrl),
    );
    if (!raw) return EMPTY_SUBSCRIPTION_STORE;
    const parsed = parseCommunitySectionSubsPayload(JSON.parse(raw));
    return parsed ?? EMPTY_SUBSCRIPTION_STORE;
  } catch {
    return EMPTY_SUBSCRIPTION_STORE;
  }
}

export function writeCommunitySectionSubs(
  pubkey: string,
  relayUrl: string,
  store: CommunitySectionSubscriptionStore,
): boolean {
  if (!pubkey || !relayUrl) return false;
  try {
    setStorageItem(
      communitySectionSubsStorageKey(pubkey, relayUrl),
      JSON.stringify(store),
    );
    return true;
  } catch {
    return false;
  }
}

export function isCommunitySectionSubscribed(
  store: CommunitySectionSubscriptionStore,
  sectionId: string,
): boolean {
  return store.subscribed[sectionId] === true;
}

export function setCommunitySectionSubscribed(
  store: CommunitySectionSubscriptionStore,
  sectionId: string,
  subscribed: boolean,
): CommunitySectionSubscriptionStore {
  const next = { ...store.subscribed };
  if (subscribed) {
    next[sectionId] = true;
  } else {
    delete next[sectionId];
  }
  return { version: 1, subscribed: next };
}

export function subscribedCommunitySectionIds(
  store: CommunitySectionSubscriptionStore,
): string[] {
  return Object.entries(store.subscribed)
    .filter(([, flag]) => flag)
    .map(([id]) => id);
}

export function notifyCommunitySectionSubsChange(
  detail: CommunitySectionSubsChange,
): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent(COMMUNITY_SECTION_SUBS_EVENT, { detail }),
  );
}
