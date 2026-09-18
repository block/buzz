import { deriveCommunityName } from "./communityStorage";
import type { Community } from "./types";
import type { CommunityProfile } from "@/shared/api/communityProfile";

/** Apply relay truth without discarding a previous label or explicit nickname. */
export function reconcileCommunityName(
  community: Community,
  profile: CommunityProfile,
): Community {
  const fallbackName = community.fallbackName ?? community.name;
  let localName = community.localName;
  if (localName === undefined && community.fallbackName === undefined) {
    let legacyIpLabel = false;
    try {
      const host = new URL(community.relayUrl).hostname;
      legacyIpLabel =
        (/^\d+(?:\.\d+){3}$/.test(host) || host.includes(":")) &&
        community.name === host.split(".")[0];
    } catch {
      /* Invalid old URLs retain their device label. */
    }
    localName =
      community.name === deriveCommunityName(community.relayUrl) ||
      legacyIpLabel
        ? ""
        : community.name;
  }
  const name = localName?.trim() || profile.name || fallbackName;
  if (
    community.canonicalName === profile.name &&
    community.name === name &&
    community.localName === localName &&
    community.fallbackName === fallbackName
  )
    return community;
  return {
    ...community,
    canonicalName: profile.name,
    fallbackName,
    localName,
    name,
  };
}

export function withLocalCommunityName(
  community: Community,
  localName: string,
): Community {
  const fallbackName = community.fallbackName ?? community.name;
  return {
    ...community,
    fallbackName,
    localName: localName.trim(),
    name: localName.trim() || community.canonicalName || fallbackName,
  };
}
