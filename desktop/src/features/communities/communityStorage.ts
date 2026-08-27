import type { Community } from "./types";
import { homeDir } from "@tauri-apps/api/path";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import { getStorageItem, removeStorageItem } from "@/shared/lib/safeStorage";

const COMMUNITIES_KEY = "buzz-communities";
const ACTIVE_COMMUNITY_KEY = "buzz-active-community-id";
const LEGACY_WORKSPACES_KEY = "buzz-workspaces";
const LEGACY_ACTIVE_WORKSPACE_KEY = "buzz-active-workspace-id";
const COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY =
  "buzz-community-discovery-after-leave";

/**
 * Expand a leading `~` to the user's home directory. The backend rejects
 * `~`-prefixed paths (`std::fs` does not expand the shell tilde), so the UI
 * resolves it before save. Returns non-`~` input unchanged. Empty/whitespace
 * input returns `undefined` so callers can clear the override.
 */
export async function expandTilde(input: string): Promise<string | undefined> {
  const trimmed = input.trim();
  if (!trimmed) {
    return undefined;
  }
  if (trimmed === "~") {
    return homeDir();
  }
  if (trimmed.startsWith("~/")) {
    const home = await homeDir();
    const base = home.endsWith("/") ? home.slice(0, -1) : home;
    return `${base}/${trimmed.slice(2)}`;
  }
  return trimmed;
}

export function migrateLegacyCommunityStorage(
  storage: Storage = localStorage,
): void {
  try {
    if (storage.getItem(COMMUNITIES_KEY) === null) {
      const legacyCommunities = storage.getItem(LEGACY_WORKSPACES_KEY);
      if (legacyCommunities !== null) {
        storage.setItem(COMMUNITIES_KEY, legacyCommunities);
      }
    }
    if (storage.getItem(ACTIVE_COMMUNITY_KEY) === null) {
      const legacyActiveCommunity = storage.getItem(
        LEGACY_ACTIVE_WORKSPACE_KEY,
      );
      if (legacyActiveCommunity !== null) {
        storage.setItem(ACTIVE_COMMUNITY_KEY, legacyActiveCommunity);
      }
    }
  } catch (error) {
    // WebKit throws SecurityError from getItem when storage access is denied
    // for the origin (block/buzz#5078). Fencing here so the app can still
    // boot with an empty/default community list instead of a blank window.
    console.warn(
      "[communityStorage] migrateLegacyCommunityStorage failed (storage denied?):",
      error,
    );
  }
}

export function loadCommunities(): Community[] {
  try {
    migrateLegacyCommunityStorage();
    const raw = getStorageItem(COMMUNITIES_KEY);
    if (!raw) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    if (parsed.length > 0) {
      removeStorageItem(COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY);
    }
    // Migration: older builds stored `nsec` and a non-functional community
    // `token` in localStorage. Identity keys now live in the OS keyring and
    // relay access is enforced by NIP-42/NIP-98 plus relay membership.
    let didStrip = false;
    const cleaned = (parsed as Array<Record<string, unknown>>).map((entry) => {
      if (
        entry &&
        typeof entry === "object" &&
        ("nsec" in entry || "token" in entry)
      ) {
        const { nsec: _nsec, token: _token, ...rest } = entry;
        didStrip = true;
        return rest;
      }
      return entry;
    }) as Community[];
    if (didStrip) {
      setLocalStorageItemWithRecovery(COMMUNITIES_KEY, JSON.stringify(cleaned));
    }
    return cleaned;
  } catch {
    return [];
  }
}

export function saveCommunities(communities: Community[]): boolean {
  const didSave = setLocalStorageItemWithRecovery(
    COMMUNITIES_KEY,
    JSON.stringify(communities),
  );
  if (didSave && communities.length > 0) {
    localStorage.removeItem(COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY);
  }
  return didSave;
}

export function loadCommunityDiscoveryAfterLeave(
  storage: Storage = localStorage,
): boolean {
  try {
    return storage.getItem(COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY) === "1";
  } catch (error) {
    // block/buzz#5078 — storage access can be denied for the origin; degrade
    // to the default ("didn't just leave") instead of crashing the boot path.
    console.warn(
      "[communityStorage] loadCommunityDiscoveryAfterLeave failed:",
      error,
    );
    return false;
  }
}

export function markCommunityDiscoveryAfterLeave(
  storage: Storage = localStorage,
): boolean {
  if (typeof window !== "undefined" && storage === window.localStorage) {
    return setLocalStorageItemWithRecovery(
      COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY,
      "1",
    );
  }
  try {
    storage.setItem(COMMUNITY_DISCOVERY_AFTER_LEAVE_KEY, "1");
    return true;
  } catch {
    return false;
  }
}

export function clearCommunityStorage(storage: Storage = localStorage): void {
  storage.removeItem(COMMUNITIES_KEY);
  storage.removeItem(ACTIVE_COMMUNITY_KEY);
  storage.removeItem(LEGACY_WORKSPACES_KEY);
  storage.removeItem(LEGACY_ACTIVE_WORKSPACE_KEY);
}

export function loadActiveCommunityId(): string | null {
  migrateLegacyCommunityStorage();
  // block/buzz#5078 — WebKit can throw SecurityError from a denied-storage
  // getItem. Fail closed so the boot path renders the default community UI
  // instead of unmounting the root.
  return getStorageItem(ACTIVE_COMMUNITY_KEY);
}

export function saveActiveCommunityId(id: string): boolean {
  return setLocalStorageItemWithRecovery(ACTIVE_COMMUNITY_KEY, id);
}

export function normalizeRelayUrl(url: string): string {
  if (!url.startsWith("ws://") && !url.startsWith("wss://")) {
    return `wss://${url}`;
  }
  return url;
}

function isPrivateIpv4(hostname: string): boolean {
  const octets = hostname.split(".").map(Number);
  if (
    octets.length !== 4 ||
    octets.some((octet) => !Number.isInteger(octet) || octet < 0 || octet > 255)
  ) {
    return false;
  }
  const [first, second] = octets;
  return (
    first === 10 ||
    first === 127 ||
    (first === 169 && second === 254) ||
    (first === 172 && second >= 16 && second <= 31) ||
    (first === 192 && second === 168) ||
    (first === 100 && second >= 64 && second <= 127)
  );
}

function isPrivateLanHost(hostname: string): boolean {
  const normalized = hostname.toLowerCase().replace(/^\[|\]$/g, "");
  return (
    normalized === "localhost" ||
    normalized === "::1" ||
    normalized.startsWith("fc") ||
    normalized.startsWith("fd") ||
    normalized.startsWith("fe8") ||
    normalized.startsWith("fe9") ||
    normalized.startsWith("fea") ||
    normalized.startsWith("feb") ||
    isPrivateIpv4(normalized)
  );
}

/** Normalize and validate the optional plaintext private-network fast path. */
export function normalizeLanRelayUrl(url: string): string | undefined {
  const trimmed = url.trim();
  if (!trimmed) return undefined;

  const withScheme = trimmed.startsWith("ws://") ? trimmed : `ws://${trimmed}`;
  let parsed: URL;
  try {
    parsed = new URL(withScheme);
  } catch {
    throw new Error("Enter a valid ws:// private-network relay URL.");
  }
  if (parsed.protocol !== "ws:") {
    throw new Error("The Campus / LAN relay must use ws://.");
  }
  if (!isPrivateLanHost(parsed.hostname)) {
    throw new Error(
      "The Campus / LAN relay must use localhost or a private IP address.",
    );
  }
  if (parsed.username || parsed.password || parsed.search || parsed.hash) {
    throw new Error(
      "The Campus / LAN relay URL cannot contain credentials or parameters.",
    );
  }
  if (parsed.pathname !== "/") {
    throw new Error("The Campus / LAN relay URL must not contain a path.");
  }
  return withScheme.replace(/\/+$/, "");
}

function isLocalRelayHost(hostname: string): boolean {
  return ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"].includes(hostname);
}

export function shouldAutoConnectDefaultRelay(relayUrl: string): boolean {
  try {
    const parsed = new URL(relayUrl);
    return (
      (parsed.protocol === "ws:" || parsed.protocol === "wss:") &&
      !isLocalRelayHost(parsed.hostname)
    );
  } catch {
    return false;
  }
}

export function deriveCommunityName(relayUrl: string): string {
  try {
    const url = new URL(
      relayUrl.replace("ws://", "http://").replace("wss://", "https://"),
    );
    const host = url.hostname;
    if (isLocalRelayHost(host)) {
      return "Local Dev";
    }
    const parts = host.split(".");
    // Detect staging environments (e.g. buzz-oss.stage.blox.sqprod.co)
    if (parts.some((p) => p === "stage" || p === "staging")) {
      return "Buzz (staging)";
    }
    // Use the first subdomain segment or the domain itself
    if (parts.length >= 2) {
      return parts[0] === "relay" ? parts[1] : parts[0];
    }
    return host;
  } catch {
    return "Community";
  }
}

export function initFirstCommunity(
  relayUrl: string,
  pubkey: string,
  name?: string,
): Community | null {
  const normalizedUrl = normalizeRelayUrl(relayUrl);
  const trimmedName = name?.trim();
  const community: Community = {
    id: crypto.randomUUID(),
    name: trimmedName || deriveCommunityName(normalizedUrl),
    relayUrl: normalizedUrl,
    // Compiled default relays must admit the first token-less connection; there
    // is no invite-token prompt on this auto-connect path.
    pubkey,
    addedAt: new Date().toISOString(),
  };
  // block/buzz#5078 — read the prior active id through the throw-safe helper;
  // a denied-storage origin would otherwise kill onboarding before a single
  // write is attempted.
  const previousActiveCommunityId = getStorageItem(ACTIVE_COMMUNITY_KEY);
  const didSaveActiveCommunity = saveActiveCommunityId(community.id);
  if (!didSaveActiveCommunity) {
    return null;
  }

  if (!saveCommunities([community])) {
    // A failed setItem leaves the existing communities value untouched. Roll
    // back only the active-ID write so inconsistent pre-existing data is never
    // destroyed while recovering from a quota failure.
    try {
      if (previousActiveCommunityId === null) {
        localStorage.removeItem(ACTIVE_COMMUNITY_KEY);
      } else {
        localStorage.setItem(ACTIVE_COMMUNITY_KEY, previousActiveCommunityId);
      }
    } catch {
      // Best effort: persistence is already unavailable, and callers will stay
      // on setup instead of reloading.
    }
    return null;
  }

  return community;
}
