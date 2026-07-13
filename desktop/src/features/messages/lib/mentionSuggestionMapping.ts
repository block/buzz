import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { formatOwnerLabel } from "@/features/profile/lib/identity";
import type { ChannelRole, ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type MentionSuggestionCandidate = {
  kind: "identity" | "persona";
  pubkey?: string;
  personaId?: string | null;
  avatarUrl?: string | null;
  isAgent: boolean;
  isMember: boolean;
  role?: ChannelRole | null;
  ownerPubkey?: string | null;
};

export function mapMentionCandidateToSuggestion(opts: {
  candidate: MentionSuggestionCandidate;
  label: string;
  channelType?: ChannelType | null;
  currentPubkey?: string | null;
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
}): MentionSuggestion {
  const {
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
    kind: candidate.kind,
    displayName: label,
    avatarUrl:
      candidate.avatarUrl ??
      (candidate.pubkey
        ? profiles?.[normalizePubkey(candidate.pubkey)]?.avatarUrl
        : null) ??
      null,
    isAgent: candidate.isAgent,
    notInChannel: channelType !== "dm" && candidate.isMember === false,
    ownerLabel,
    role: !candidate.isAgent && candidate.role === "admin" ? "admin" : null,
  };
}
