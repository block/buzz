import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { formatOwnerLabel } from "@/features/profile/lib/identity";
import type { ChannelRole, ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { TeamMentionMember } from "./mentionCandidates";

export type MentionSuggestionCandidate = {
  kind: "identity" | "persona" | "team";
  pubkey?: string;
  personaId?: string | null;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  avatarUrl?: string | null;
  isAgent: boolean;
  isMember: boolean;
  role?: ChannelRole | null;
  description?: string | null;
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

  // Agent role line: prefer an `about` resolved at candidate-build time
  // (search results), else fall back to the profile lookup like avatarUrl.
  // Collapsed to a single line — the selector is a quick picker, not a
  // profile card.
  const rawDescription = candidate.isAgent
    ? (candidate.description ??
      (candidate.pubkey
        ? profiles?.[normalizePubkey(candidate.pubkey)]?.about
        : null))
    : null;
  const description = rawDescription?.replace(/\s+/g, " ").trim() || null;

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
    notInChannel:
      candidate.kind !== "team" &&
      channelType !== "dm" &&
      candidate.isMember === false,
    ownerLabel,
    role: !candidate.isAgent && candidate.role === "admin" ? "admin" : null,
    description,
  };
}
