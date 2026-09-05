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
  shape?: "circle" | "squircle";
  size: number;
  status?: PresenceStatus;
  statusClipTestId?: string;
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

const SQUIRCLE_STATUS_INSET_RATIO = 0.05;

export function insetProfileAvatarStatusGeometry(
  geometry: ProfileAvatarStatusGeometry,
  size: number,
): ProfileAvatarStatusGeometry {
  const inset = size * SQUIRCLE_STATUS_INSET_RATIO;

  return {
    ...geometry,
    centerX: geometry.centerX - inset,
    centerY: geometry.centerY - inset,
  };
}

export function ProfileAvatarWithStatus({
  avatarClassName,
  avatarUrl,
  className,
  geometry = DEFAULT_HOVER_PROFILE_STATUS_GEOMETRY,
  iconClassName,
  label,
  shape = "circle",
  size,
  status,
  statusClipTestId,
  statusTestId,
  testId,
}: ProfileAvatarWithStatusProps) {
  const badgeGeometry =
    shape === "squircle"
      ? insetProfileAvatarStatusGeometry(geometry, size)
      : geometry;
  const statusLabel = status ? getPresenceLabel(status) : null;
  const cutout = status
    ? {
        // Keep the opening anchored at the avatar edge for both shapes. Only
        // the squircle's visible dot moves inward; insetting the cutout too
        // closes the notch and makes the badge look painted over the avatar.
        cx: geometry.centerX,
        cy: geometry.centerY,
        r: geometry.cutoutSize / 2,
      }
    : undefined;
  const badgeBox = status
    ? {
        bottom: size - badgeGeometry.centerY - badgeGeometry.dotSize / 2,
        height: badgeGeometry.dotSize,
        right: size - badgeGeometry.centerX - badgeGeometry.dotSize / 2,
        width: badgeGeometry.dotSize,
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
      badgeCenter={{ cx: badgeGeometry.centerX, cy: badgeGeometry.centerY }}
      className={cn("inline-flex", className)}
      clipTestId={statusClipTestId}
      cornerRadius={shape === "squircle" ? size * 0.3 : undefined}
      curve={STATUS_DOT_MASK_CURVE}
      cutout={cutout}
      size={size}
    >
      <ProfileAvatar
        avatarUrl={avatarUrl}
        className={cn("h-full w-full", avatarClassName)}
        iconClassName={iconClassName}
        label={label}
        shape={shape}
        testId={testId}
      />
    </MaskedAvatarBadgeFrame>
  );
}
