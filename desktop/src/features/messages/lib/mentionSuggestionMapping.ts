import { isOwnedAgentNotManagedOnDevice } from "@/features/agents/lib/otherSetupAgent";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { formatOwnerLabel } from "@/features/profile/lib/identity";
import type { ChannelRole, ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { MentionCandidate, TeamMentionMember } from "./mentionCandidates";
import { mentionCandidateLabel } from "./mentionCandidates";
import { pickDefaultAgentCandidate } from "./mentionRanking";

export type MentionSuggestionCandidate = {
  kind: "identity" | "persona" | "team";
  pubkey?: string;
  personaId?: string | null;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  avatarUrl?: string | null;
  isAgent: boolean;
  isManagedAgent?: boolean;
  isMember: boolean;
  role?: ChannelRole | null;
  ownerPubkey?: string | null;
};

export function mapMentionCandidateToSuggestion(opts: {
  candidate: MentionSuggestionCandidate;
  label: string;
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  agentProvenanceReady: boolean;
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
}): MentionSuggestion {
  const {
    agentProvenanceReady,
    candidate,
    channelType,
    currentPubkey,
    label,
    ownerProfiles,
    profiles,
  } = opts;
  const ownerLabel = candidate.isAgent
    ? formatOwnerLabel(candidate.ownerPubkey, currentPubkey, ownerProfiles)
    : null;

  return {
    pubkey: candidate.pubkey,
    personaId: candidate.personaId ?? undefined,
    teamId: candidate.teamId,
    teamMembers: candidate.teamMembers,
    kind: candidate.kind,
    displayName: label,
    avatarUrl:
      candidate.avatarUrl ??
      (candidate.pubkey
        ? profiles?.[normalizePubkey(candidate.pubkey)]?.avatarUrl
        : null) ??
      null,
    isAgent: candidate.isAgent,
    agentProvenance:
      agentProvenanceReady && candidate.kind === "identity" && candidate.isAgent
        ? candidate.isManagedAgent
          ? "managed-here"
          : isOwnedAgentNotManagedOnDevice({
                currentPubkey: currentPubkey ?? undefined,
                ownerPubkey: candidate.ownerPubkey,
                localInventoryReady: agentProvenanceReady,
                isLocallyManaged: Boolean(candidate.isManagedAgent),
              })
            ? "managed-elsewhere"
            : undefined
        : undefined,
    notInChannel:
      candidate.kind !== "team" &&
      channelType !== "dm" &&
      candidate.isMember === false,
    ownerLabel,
    role: !candidate.isAgent && candidate.role === "admin" ? "admin" : null,
  };
}

export function pickDefaultAgentSuggestion(opts: {
  activePersonaIds: ReadonlySet<string>;
  agentProvenanceReady: boolean;
  candidates: readonly MentionCandidate[];
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
  recentMentionPubkeys?: readonly string[];
}): MentionSuggestion | null {
  const candidate = pickDefaultAgentCandidate(
    opts.candidates,
    opts.activePersonaIds,
    opts.recentMentionPubkeys,
  );
  if (!candidate) return null;
  return mapMentionCandidateToSuggestion({
    ...opts,
    candidate,
    label: mentionCandidateLabel(candidate),
  });
}
