import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { messageAvatarSizeStyle } from "@/shared/lib/avatarScale";
import { useAvatarScale } from "@/shared/lib/useAvatarScale";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type DirectMessageIntroParticipant = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

export function DirectMessageIntroAvatarStack({
  participants,
}: {
  participants: DirectMessageIntroParticipant[];
}) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const stackItemCount = visibleParticipants.length + (hiddenCount > 0 ? 1 : 0);
  const avatarScale = useAvatarScale();
  const introBaseRem = 3.75;
  const introAvatarStyle = messageAvatarSizeStyle(introBaseRem, avatarScale);
  const introSizeRem = introBaseRem * avatarScale;
  // Overlap ~20% of the disc; mask radius tracks the scaled size.
  const stackOverlapRem = introSizeRem * 0.33;
  const maskRadiusRem = introSizeRem * 0.567;
  const maskOffsetRem = introSizeRem * 0.167;

  return (
    <div
      className="flex shrink-0 items-center"
      data-testid="message-dm-intro-avatar-stack"
    >
      {visibleParticipants.map((participant, index) => (
        <UserProfilePopover
          key={participant.pubkey}
          pubkey={participant.pubkey}
          triggerAriaLabel={`Open profile for ${participant.displayName}`}
          triggerElement="span"
        >
          <span
            className={index > 0 ? "relative" : "relative"}
            data-testid="message-dm-intro-avatar-stack-participant"
            style={{
              zIndex: index + 1,
              marginLeft: index > 0 ? `-${stackOverlapRem}rem` : undefined,
              ...(index < stackItemCount - 1 && {
                mask: `radial-gradient(circle ${maskRadiusRem}rem at calc(100% + ${maskOffsetRem}rem) 50%, transparent 99%, #fff 100%)`,
                WebkitMask: `radial-gradient(circle ${maskRadiusRem}rem at calc(100% + ${maskOffsetRem}rem) 50%, transparent 99%, #fff 100%)`,
              }),
            }}
          >
            <UserAvatar
              avatarUrl={participant.avatarUrl}
              className="text-base"
              displayName={participant.displayName}
              size="md"
              style={introAvatarStyle}
            />
          </span>
        </UserProfilePopover>
      ))}
      {hiddenCount > 0 ? (
        <div
          className="relative"
          data-testid="message-dm-intro-avatar-stack-more"
          style={{
            zIndex: stackItemCount,
            marginLeft:
              visibleParticipants.length > 0
                ? `-${stackOverlapRem}rem`
                : undefined,
          }}
        >
          <span
            className="flex items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs"
            style={introAvatarStyle}
          >
            <span className="text-lg leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
