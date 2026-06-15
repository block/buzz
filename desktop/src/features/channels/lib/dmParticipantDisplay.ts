import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const DM_PARTICIPANT_PREVIEW_LIMIT = 3;

export type DmParticipantDisplay = {
  displayName: string;
};

export type DirectMessageIntroParticipant = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

export type DirectMessageIntro = {
  displayName: string;
  participants: DirectMessageIntroParticipant[];
};

export function getDmParticipantPreview<T>(participants: readonly T[]) {
  const visibleParticipants = participants.slice(
    0,
    DM_PARTICIPANT_PREVIEW_LIMIT,
  );

  return {
    hiddenCount: Math.max(
      0,
      participants.length - DM_PARTICIPANT_PREVIEW_LIMIT,
    ),
    visibleParticipants,
  };
}

export function formatDmParticipantDisplayName(
  participants: readonly DmParticipantDisplay[],
) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const names = visibleParticipants.map(
    (participant) => participant.displayName,
  );

  return hiddenCount > 0
    ? [...names, `+${hiddenCount} more`].join(", ")
    : names.join(", ");
}

export function buildDirectMessageIntro({
  channel,
  currentPubkey,
  profiles,
}: {
  channel: Channel | null;
  currentPubkey?: string;
  profiles?: UserProfileLookup;
}): DirectMessageIntro | null {
  if (channel?.channelType !== "dm") {
    return null;
  }

  const participants = channel.participantPubkeys.map((pubkey, index) => ({
    fallbackName: channel.participants[index] ?? null,
    pubkey,
  }));
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const otherParticipants = normalizedCurrentPubkey
    ? participants.filter(
        (participant) =>
          normalizePubkey(participant.pubkey) !== normalizedCurrentPubkey,
      )
    : participants;
  const displayParticipants =
    otherParticipants.length > 0 ? otherParticipants : participants;

  if (displayParticipants.length === 0) {
    return null;
  }

  const introParticipants = displayParticipants.map((participant) => {
    const profile = profiles?.[normalizePubkey(participant.pubkey)] ?? null;

    return {
      avatarUrl: profile?.avatarUrl ?? null,
      displayName: resolveUserLabel({
        currentPubkey,
        fallbackName: participant.fallbackName,
        profiles,
        pubkey: participant.pubkey,
      }),
      pubkey: participant.pubkey,
    };
  });

  return {
    displayName: formatDmParticipantDisplayName(introParticipants),
    participants: introParticipants,
  };
}
