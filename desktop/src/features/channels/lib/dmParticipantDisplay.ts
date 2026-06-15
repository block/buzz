export const DM_PARTICIPANT_PREVIEW_LIMIT = 3;

export type DmParticipantDisplay = {
  displayName: string;
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
