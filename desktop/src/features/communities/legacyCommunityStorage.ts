import { invokeTauri } from "@/shared/api/tauri";
import { getStorageItem } from "@/shared/lib/safeStorage";
import { migrateLegacyCommunityStorage } from "./communityStorage";

const BUZZ_COMMUNITIES_KEY = "buzz-communities";
const BUZZ_ACTIVE_COMMUNITY_KEY = "buzz-active-community-id";
// Bump this marker whenever the disposable community cache set changes. A
// marker bump is required because older installs may already have recorded a
// successful repair while retaining a cache that later proved unsafe.
const BUZZ_STORAGE_REPAIR_KEY = "buzz-storage-repair-v2";
const COMMUNITY_CACHE_PREFIXES = [
  "buzz-channels.v1:",
  "buzz-channel-messages.v1:",
  "buzz-home-feed-",
  "buzz-observed-",
  "buzz-active-agent-turns:",
  "buzz-relay-",
];
const BUZZ_ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX =
  "buzz-onboarding-complete.v1:";
const LOCAL_DEV_RELAY_URLS = new Set([
  "ws://localhost:3000",
  "ws://127.0.0.1:3000",
]);

type LegacyCommunityStorageSnapshot = {
  workspaces: string | null;
  activeWorkspaceId: string | null;
  onboardingCompletions: Array<{
    pubkey: string;
    value: string;
  }>;
};

type StoredCommunity = {
  relayUrl?: unknown;
};

function parseCommunityList(raw: string | null): StoredCommunity[] | null {
  if (!raw) {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as StoredCommunity[]) : null;
  } catch {
    return null;
  }
}

function normalizeRelayUrl(relayUrl: string) {
  return relayUrl.trim().replace(/\/$/, "");
}

function hasOnlyLocalDevCommunity(raw: string | null): boolean {
  const communities = parseCommunityList(raw);
  return (
    communities?.length === 1 &&
    typeof communities[0]?.relayUrl === "string" &&
    LOCAL_DEV_RELAY_URLS.has(normalizeRelayUrl(communities[0].relayUrl))
  );
}

function hasNonLocalCurrentCommunities(raw: string | null): boolean {
  const communities = parseCommunityList(raw);
  return (
    communities !== null &&
    communities.length > 0 &&
    !hasOnlyLocalDevCommunity(raw)
  );
}

function shouldWriteLegacyCommunities({
  currentCommunitiesRaw,
  legacyCommunitiesRaw,
}: {
  currentCommunitiesRaw: string | null;
  legacyCommunitiesRaw: string | null;
}) {
  const legacyCommunities = parseCommunityList(legacyCommunitiesRaw);
  if (!legacyCommunities || legacyCommunities.length === 0) {
    return false;
  }

  return !hasNonLocalCurrentCommunities(currentCommunitiesRaw);
}

export function applyLegacyCommunityStorage(
  legacyStorage: LegacyCommunityStorageSnapshot,
  storage: Storage = window.localStorage,
): void {
  const currentCommunitiesRaw = storage.getItem(BUZZ_COMMUNITIES_KEY);
  const shouldWriteCommunities = shouldWriteLegacyCommunities({
    currentCommunitiesRaw,
    legacyCommunitiesRaw: legacyStorage.workspaces,
  });

  if (shouldWriteCommunities && legacyStorage.workspaces) {
    storage.setItem(BUZZ_COMMUNITIES_KEY, legacyStorage.workspaces);
  }

  const currentActiveCommunityId = storage.getItem(BUZZ_ACTIVE_COMMUNITY_KEY);
  if (
    legacyStorage.activeWorkspaceId &&
    (!currentActiveCommunityId || shouldWriteCommunities)
  ) {
    storage.setItem(BUZZ_ACTIVE_COMMUNITY_KEY, legacyStorage.activeWorkspaceId);
  }

  for (const completion of legacyStorage.onboardingCompletions) {
    const key = `${BUZZ_ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX}${completion.pubkey}`;
    if (storage.getItem(key) === null) {
      storage.setItem(key, completion.value);
    }
  }
}

/**
 * Seed Buzz localStorage from legacy Sprout WebKit localStorage before the app
 * renders providers that read community state. The native command reads the old
 * app identifier's WebKit SQLite database; this frontend step writes only when
 * Buzz does not already have community state, except for the known broken
 * Sprout→Buzz first-run handoff that created a single localhost community.
 */
export async function migrateLegacyCommunityStorageBeforeRender(): Promise<void> {
  if (typeof window === "undefined") {
    return;
  }

  migrateLegacyCommunityStorage(window.localStorage);
  repairStaleCommunityCaches(window.localStorage);
  // block/buzz#5078 — read through the throw-safe accessor so a denied-storage
  // origin degrades to "no community state" instead of crashing pre-render.
  const currentCommunitiesRaw = getStorageItem(BUZZ_COMMUNITIES_KEY);
  const hasCurrentActiveCommunity = getStorageItem(BUZZ_ACTIVE_COMMUNITY_KEY);
  if (
    currentCommunitiesRaw &&
    hasCurrentActiveCommunity &&
    !hasOnlyLocalDevCommunity(currentCommunitiesRaw)
  ) {
    return;
  }

  try {
    applyLegacyCommunityStorage(
      await invokeTauri<LegacyCommunityStorageSnapshot>(
        "get_legacy_workspace_storage",
      ),
    );
  } catch (error) {
    console.warn("Failed to read legacy Sprout community storage.", error);
  }
}

/**
 * One-time cleanup for state written by pre-repair builds. These snapshots are
 * disposable and can contain a relay URL from a previous LAN/public setup;
 * keeping them can make the first channel query appear permanently loading.
 * Community definitions, identity data, Agent data, and message history use
 * different keys and are intentionally retained.
 */
function repairStaleCommunityCaches(storage: Storage): void {
  try {
    if (storage.getItem(BUZZ_STORAGE_REPAIR_KEY) === "1") return;
    const keysToRemove: string[] = [];
    for (let index = 0; index < storage.length; index += 1) {
      const key = storage.key(index);
      if (
        key &&
        COMMUNITY_CACHE_PREFIXES.some((prefix) => key.startsWith(prefix))
      ) {
        keysToRemove.push(key);
      }
    }
    for (const key of keysToRemove) storage.removeItem(key);
    storage.setItem(BUZZ_STORAGE_REPAIR_KEY, "1");
  } catch (error) {
    console.warn("[communityStorage] stale cache repair skipped:", error);
  }
}
