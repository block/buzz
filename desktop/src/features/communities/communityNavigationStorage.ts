import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const COMMUNITY_DESTINATIONS_KEY = "buzz-community-destinations";

export type CommunityDestination =
  | { kind: "home" }
  | { kind: "channel"; channelId: string };

type CommunityDestinations = Record<string, CommunityDestination>;

function isCommunityDestination(value: unknown): value is CommunityDestination {
  if (!value || typeof value !== "object") {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return (
    candidate.kind === "home" ||
    (candidate.kind === "channel" &&
      typeof candidate.channelId === "string" &&
      candidate.channelId.length > 0)
  );
}

function loadCommunityDestinations(storage: Storage): CommunityDestinations {
  try {
    const raw = storage.getItem(COMMUNITY_DESTINATIONS_KEY);
    if (!raw) {
      return {};
    }

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, CommunityDestination] =>
          isCommunityDestination(entry[1]),
      ),
    );
  } catch {
    return {};
  }
}

function saveCommunityDestinations(
  destinations: CommunityDestinations,
  storage: Storage,
): void {
  const serialized = JSON.stringify(destinations);
  if (typeof window !== "undefined" && storage === window.localStorage) {
    setLocalStorageItemWithRecovery(COMMUNITY_DESTINATIONS_KEY, serialized);
    return;
  }
  storage.setItem(COMMUNITY_DESTINATIONS_KEY, serialized);
}

export function loadCommunityDestination(
  communityId: string,
  storage: Storage = localStorage,
): CommunityDestination | null {
  return loadCommunityDestinations(storage)[communityId] ?? null;
}

export function saveCommunityDestination(
  communityId: string,
  destination: CommunityDestination,
  storage: Storage = localStorage,
): void {
  saveCommunityDestinations(
    { ...loadCommunityDestinations(storage), [communityId]: destination },
    storage,
  );
}

export function removeCommunityDestination(
  communityId: string,
  storage: Storage = localStorage,
): void {
  const destinations = loadCommunityDestinations(storage);
  if (!(communityId in destinations)) {
    return;
  }
  delete destinations[communityId];
  saveCommunityDestinations(destinations, storage);
}

export function clearCommunityDestinations(
  storage: Storage = localStorage,
): void {
  storage.removeItem(COMMUNITY_DESTINATIONS_KEY);
}
