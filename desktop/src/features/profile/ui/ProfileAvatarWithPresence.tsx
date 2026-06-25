import type { PresenceStatus } from "@/shared/api/types";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";

type ProfileAvatarWithPresenceProps = {
  avatarDataUrl?: string | null;
  avatarUrl: string | null;
  className?: string;
  iconClassName?: string;
  label: string;
  plain?: boolean;
  presenceClassName?: string;
  presenceDotClassName?: string;
  presenceStatus?: PresenceStatus;
  presenceTestId?: string;
  testId?: string;
};

export function ProfileAvatarWithPresence({
  avatarDataUrl,
  avatarUrl,
  className,
  iconClassName,
  label,
  plain,
  presenceClassName,
  presenceDotClassName,
  presenceStatus,
  presenceTestId,
  testId,
}: ProfileAvatarWithPresenceProps) {
  return (
    <div className="relative shrink-0">
      <ProfileAvatar
        avatarDataUrl={avatarDataUrl}
        avatarUrl={avatarUrl}
        className={className}
        iconClassName={iconClassName}
        label={label}
        plain={plain}
        testId={testId}
      />
      {presenceStatus ? (
        <span
          aria-label={getPresenceLabel(presenceStatus)}
          className={cn(
            "absolute flex items-center justify-center rounded-full bg-background",
            presenceClassName,
          )}
          data-testid={presenceTestId}
          role="img"
        >
          <PresenceDot
            className={presenceDotClassName}
            status={presenceStatus}
          />
        </span>
      ) : null}
    </div>
  );
}
