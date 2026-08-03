import type { RelayConnectionPolicy } from "@/shared/api/tauriRelayPolicy";
import type { Community } from "./types";

export const UNRESTRICTED_RELAY_CONNECTION_POLICY: RelayConnectionPolicy = {
  lockedToDefaultRelay: false,
  defaultRelayUrl: "ws://localhost:3000",
};

function relayOrigin(raw: string): string | null {
  try {
    const url = new URL(raw.trim());
    if (
      (url.protocol !== "ws:" && url.protocol !== "wss:") ||
      url.username !== "" ||
      url.password !== "" ||
      (url.pathname !== "" && url.pathname !== "/") ||
      url.search !== "" ||
      url.hash !== ""
    ) {
      return null;
    }
    return url.origin;
  } catch {
    return null;
  }
}

export function isRelayUrlAllowed(
  policy: RelayConnectionPolicy,
  relayUrl: string,
): boolean {
  if (!policy.lockedToDefaultRelay) return true;
  const candidate = relayOrigin(relayUrl);
  const allowed = relayOrigin(policy.defaultRelayUrl);
  return candidate !== null && allowed !== null && candidate === allowed;
}

export function applyRelayConnectionPolicy(
  communities: Community[],
  activeId: string | null,
  policy: RelayConnectionPolicy,
): { communities: Community[]; activeId: string | null; changed: boolean } {
  if (!policy.lockedToDefaultRelay) {
    return { communities, activeId, changed: false };
  }
  const allowedCommunities = communities.filter((community) =>
    isRelayUrlAllowed(policy, community.relayUrl),
  );
  const nextActiveId = allowedCommunities.some(
    (community) => community.id === activeId,
  )
    ? activeId
    : (allowedCommunities[0]?.id ?? null);
  return {
    communities: allowedCommunities,
    activeId: nextActiveId,
    changed:
      allowedCommunities.length !== communities.length ||
      nextActiveId !== activeId,
  };
}
