import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "./MaskedAvatarBadgeFrame";
import { ProfileAvatar } from "./ProfileAvatar";

export type ProfileAvatarStatusGeometry = {
  dotSize: number;
  cutoutSize: number;
  centerX: number;
  centerY: number;
};

type ProfileAvatarWithStatusProps = {
  avatarClassName?: string;
  avatarUrl: string | null;
  className?: string;
  geometry?: ProfileAvatarStatusGeometry;
  iconClassName?: string;
  label: string;
  size: number;
  status?: PresenceStatus;
  statusTestId?: string;
  testId?: string;
};

export const DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY = {
  dotSize: 10,
  cutoutSize: 16,
  centerX: 34,
  centerY: 34,
} satisfies ProfileAvatarStatusGeometry;

export function scaleProfileAvatarStatusGeometry(
  geometry: ProfileAvatarStatusGeometry,
  size: number,
  baseSize = 40,
): ProfileAvatarStatusGeometry {
  const scale = size / baseSize;
  return {
    dotSize: geometry.dotSize * scale,
    cutoutSize: geometry.cutoutSize * scale,
    centerX: geometry.centerX * scale,
    centerY: geometry.centerY * scale,
  };
}

export function ProfileAvatarWithStatus({
  avatarClassName,
  avatarUrl,
  className,
  geometry = DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  iconClassName,
  label,
  size,
  status,
  statusTestId,
  testId,
}: ProfileAvatarWithStatusProps) {
  const statusLabel = status ? getPresenceLabel(status) : null;
  const cutout = status
    ? {
        cx: geometry.centerX,
        cy: geometry.centerY,
        r: geometry.cutoutSize / 2,
      }
    : undefined;
  const badgeBox = status
    ? {
        bottom: size - geometry.centerY - geometry.dotSize / 2,
        height: geometry.dotSize,
        right: size - geometry.centerX - geometry.dotSize / 2,
        width: geometry.dotSize,
      }
    : undefined;

  return (
    <MaskedAvatarBadgeFrame
      badge={
        status ? (
          <span
            aria-label={statusLabel ?? undefined}
            className="flex h-full w-full items-center justify-center rounded-full"
            data-testid={statusTestId}
            role="img"
          >
            <PresenceDot className="h-full w-full" status={status} />
            {statusLabel ? (
              <span className="sr-only">{statusLabel}</span>
            ) : null}
          </span>
        ) : undefined
      }
      badgeBox={badgeBox}
      className={cn("inline-flex", className)}
      curve={STATUS_DOT_MASK_CURVE}
      cutout={cutout}
      size={size}
    >
      <ProfileAvatar
        avatarUrl={avatarUrl}
        className={cn("h-full w-full rounded-full", avatarClassName)}
        iconClassName={iconClassName}
        label={label}
        testId={testId}
      />
    </MaskedAvatarBadgeFrame>
  );
}
