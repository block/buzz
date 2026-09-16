import * as React from "react";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import type { MentionCandidate } from "./mentionCandidates";
import { pickDefaultAgentSuggestion } from "./mentionSuggestionMapping";

export function useDefaultAgentSuggestion({
  activePersonaIds,
  agentProvenanceReady,
  candidates,
  channelType,
  currentPubkey,
  ownerProfiles,
  profiles,
  recentMentionPubkeys,
}: {
  activePersonaIds: ReadonlySet<string>;
  agentProvenanceReady: boolean;
  candidates: readonly MentionCandidate[];
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
  recentMentionPubkeys?: readonly string[];
}): () => MentionSuggestion | null {
  return React.useCallback(
    () =>
      pickDefaultAgentSuggestion({
        activePersonaIds,
        agentProvenanceReady,
        candidates,
        channelType,
        currentPubkey,
        ownerProfiles,
        profiles,
        recentMentionPubkeys,
      }),
    [
      activePersonaIds,
      agentProvenanceReady,
      candidates,
      channelType,
      currentPubkey,
      ownerProfiles,
      profiles,
      recentMentionPubkeys,
    ],
  );
}
