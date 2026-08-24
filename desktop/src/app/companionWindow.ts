import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type CompanionWindowKind = "agent-activity" | "huddle";

/** Classify native companion surfaces that reuse the main application shell. */
export function companionWindowKindForLabel(
  label: string,
): CompanionWindowKind | null {
  if (label.startsWith("agent-activity-")) return "agent-activity";
  if (label.startsWith("huddle-")) return "huddle";
  return null;
}

/** Return this webview's companion kind, or null for the primary app window. */
export function currentCompanionWindowKind(): CompanionWindowKind | null {
  if (!isTauri()) return null;

  try {
    return companionWindowKindForLabel(getCurrentWindow().label);
  } catch {
    // Browser previews can expose the Tauri IPC mock without window metadata.
    return null;
  }
}

export type AgentActivityCompanionCoordinates = {
  community: string;
  agentSession: string;
  agentSessionChannel: string;
};

export function agentActivityCompanionCoordinates(
  companionKind: CompanionWindowKind | null,
  search: Record<string, unknown>,
): AgentActivityCompanionCoordinates | undefined {
  if (
    companionKind !== "agent-activity" ||
    typeof search.community !== "string" ||
    typeof search.agentSession !== "string" ||
    typeof search.agentSessionChannel !== "string"
  ) {
    return undefined;
  }

  return {
    community: search.community,
    agentSession: search.agentSession,
    agentSessionChannel: search.agentSessionChannel,
  };
}

/** Return the immutable route coordinates owned by this activity companion. */
export function currentAgentActivityCompanionCoordinates(
  search: Record<string, unknown>,
): AgentActivityCompanionCoordinates | undefined {
  return agentActivityCompanionCoordinates(
    currentCompanionWindowKind(),
    search,
  );
}

/** Search keys that replace the dedicated feed with another channel panel. */
const AGENT_ACTIVITY_COMPANION_PANEL_KEYS = [
  "autoSend",
  "channelManagement",
  "messageId",
  "profile",
  "profileTab",
  "profileView",
  "thread",
  "threadRootId",
] as const;

/** Keep a dedicated feed on its immutable coordinates and activity panel. */
export function pinAgentActivityCompanionSearch(
  companionKind: CompanionWindowKind | null,
  currentSearch: Record<string, unknown>,
  nextSearch: Record<string, unknown>,
): Record<string, unknown> {
  const coordinates = agentActivityCompanionCoordinates(
    companionKind,
    currentSearch,
  );
  if (!coordinates) return nextSearch;

  const pinnedSearch = { ...nextSearch };
  for (const key of AGENT_ACTIVITY_COMPANION_PANEL_KEYS) {
    delete pinnedSearch[key];
  }
  return { ...pinnedSearch, ...coordinates };
}

/** Apply the dedicated-feed search invariant for this native window. */
export function pinCurrentAgentActivityCompanionSearch(
  currentSearch: Record<string, unknown>,
  nextSearch: Record<string, unknown>,
): Record<string, unknown> {
  return pinAgentActivityCompanionSearch(
    currentCompanionWindowKind(),
    currentSearch,
    nextSearch,
  );
}

/** Community encoded into a companion bootstrap hash. */
export function companionCommunityIdForHash(hash: string): string | null {
  const query = hash.indexOf("?");
  if (query === -1) return null;
  return new URLSearchParams(hash.slice(query + 1)).get("community");
}

export type CompanionCommunityBootstrap = {
  initialActiveCommunityId: string | undefined;
  missingRequiredCommunity: boolean;
};

/** Agent activity windows pin their origin community; huddles use normal selection. */
export function companionCommunityBootstrap(
  companionKind: CompanionWindowKind | null,
  hash: string,
): CompanionCommunityBootstrap {
  if (companionKind !== "agent-activity") {
    return {
      initialActiveCommunityId: undefined,
      missingRequiredCommunity: false,
    };
  }
  const communityId = companionCommunityIdForHash(hash);
  return {
    initialActiveCommunityId: communityId ?? undefined,
    missingRequiredCommunity: communityId === null,
  };
}

/** Whether this realm owns the native pending deep-link queue. */
export function acceptsNativeDeepLinks(
  companionKind: CompanionWindowKind | null,
): boolean {
  return companionKind === null;
}

/** Whether this webview is a focused companion rather than the primary app. */
export function isCompanionWindow(): boolean {
  return currentCompanionWindowKind() !== null;
}
