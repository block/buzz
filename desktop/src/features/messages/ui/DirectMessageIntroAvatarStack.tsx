import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
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
            className={index > 0 ? "-ml-5" : ""}
            data-testid="message-dm-intro-avatar-stack-participant"
            style={{
              zIndex: index + 1,
              ...(index < stackItemCount - 1 && {
                mask: "radial-gradient(circle 34px at calc(100% + 10px) 50%, transparent 99%, #fff 100%)",
                WebkitMask:
                  "radial-gradient(circle 34px at calc(100% + 10px) 50%, transparent 99%, #fff 100%)",
              }),
            }}
          >
            <UserAvatar
              avatarUrl={participant.avatarUrl}
              className="h-[60px] w-[60px] text-base"
              displayName={participant.displayName}
              size="md"
            />
          </span>
        </UserProfilePopover>
      ))}
      {hiddenCount > 0 ? (
        <div
          className={visibleParticipants.length > 0 ? "-ml-5" : ""}
          data-testid="message-dm-intro-avatar-stack-more"
          style={{ zIndex: stackItemCount }}
        >
          <span className="flex h-[60px] w-[60px] items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs">
            <span className="text-lg leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
