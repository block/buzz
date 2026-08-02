import { CircleAlert, Play } from "lucide-react";
import { useReducedMotion } from "motion/react";

import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import {
  type AvatarBadgeCurve,
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentRuntimeAvatarControlProps = {
  activeTestId: string;
  avatarUrl?: string | null;
  errorLabel?: string | null;
  errorTestId?: string;
  isActive: boolean;
  isStarting: boolean;
  label: string;
  /**
   * Live relay presence of the agent, read only while `isActive`. The dot must
   * not be hardcoded green: `isActive` is a control-plane fact ("this desktop
   * started it" / "this desktop deployed it"), and for a remote deployment that
   * fact never expires, so a stopped remote agent would otherwise read as
   * online forever. `null` is for a card with no agent behind it (a persona
   * that has never been spawned), which is never active.
   */
  presenceStatus: PresenceStatus | null;
  startTestId: string;
  onOpenError?: () => void;
  onStart: () => void;
};

const TAILWIND_SPACING = {
  "1": 4,
  "2": 8,
  "2.5": 10,
  "6": 24,
  "11": 44,
  "24": 96,
} as const;

const AGENT_AVATAR_SIZE = TAILWIND_SPACING["24"];
const ACTION_BADGE_SIZE = TAILWIND_SPACING["11"];
const ACTIVE_BADGE_SIZE = TAILWIND_SPACING["6"];
const ACTION_BADGE_OFFSET = TAILWIND_SPACING["2.5"];
const ACTIVE_BADGE_INSET = TAILWIND_SPACING["1"];
const ACTIVE_DOT_CLASS_NAME = "h-4.5 w-4.5";
const PROFILE_STATUS_CUTOUT_RATIO = 1.25;

function getBadgeCenter(badgeSize: number, outwardOffset: number) {
  return AGENT_AVATAR_SIZE + outwardOffset - badgeSize / 2;
}

function getActionBadge(offset: number) {
  return {
    cutout: {
      cx: getBadgeCenter(ACTION_BADGE_SIZE, offset),
      cy: getBadgeCenter(ACTION_BADGE_SIZE, offset),
      r: ACTION_BADGE_SIZE / 2,
    },
    shell: {
      bottom: -offset,
      height: ACTION_BADGE_SIZE,
      right: -offset,
      width: ACTION_BADGE_SIZE,
    },
  } as const;
}

function getActiveBadge(inset: number) {
  return {
    cutout: {
      cx: getBadgeCenter(ACTIVE_BADGE_SIZE, -inset),
      cy: getBadgeCenter(ACTIVE_BADGE_SIZE, -inset),
      r: (ACTIVE_BADGE_SIZE / 2) * PROFILE_STATUS_CUTOUT_RATIO,
    },
    shell: {
      bottom: inset,
      height: ACTIVE_BADGE_SIZE,
      right: inset,
      width: ACTIVE_BADGE_SIZE,
    },
  } as const;
}

const ACTION_MASK_CURVE = {
  avatarRoundingAngle: 0.16,
  cutoutRoundingLength: ACTION_BADGE_SIZE * 0.18,
  cutoutRoundingMinAngle: 0.34,
  cutoutRoundingMaxAngle: 0.52,
  handleDistanceRatio: 0.58,
  handleLengthRatio: 0.26,
} satisfies AvatarBadgeCurve;

const ACTION_BADGE = getActionBadge(ACTION_BADGE_OFFSET);
const ACTIVE_BADGE = getActiveBadge(ACTIVE_BADGE_INSET);

const MASK_TRANSITION = {
  duration: 0.22,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function AgentRuntimeAvatarControl({
  activeTestId,
  avatarUrl,
  errorLabel,
  errorTestId,
  isActive,
  isStarting,
  label,
  presenceStatus,
  startTestId,
  onOpenError,
  onStart,
}: AgentRuntimeAvatarControlProps) {
  const shouldReduceMotion = useReducedMotion();
  const trimmedAvatarUrl = avatarUrl?.trim() || null;
  const actionLabel = isStarting ? `Starting ${label}` : `Start ${label}`;
  const hasError = !isActive && !isStarting && Boolean(errorLabel);
  const errorActionLabel = `${label} has a runtime error. Open runtime details.`;
  const transition = shouldReduceMotion ? { duration: 0 } : MASK_TRANSITION;
  const badge = isActive ? ACTIVE_BADGE : ACTION_BADGE;
  const activePresence = presenceStatus ?? "offline";
  const activeLabel = `${label} is ${getPresenceLabel(
    activePresence,
  ).toLowerCase()}`;

  return (
    <MaskedAvatarBadgeFrame
      badge={
        <span className="grid h-full w-full place-items-center">
          {isActive ? (
            <span
              aria-label={activeLabel}
              className="flex h-6 w-6 items-center justify-center rounded-full"
              data-testid={activeTestId}
              role="img"
              title={activeLabel}
            >
              <PresenceDot
                className={ACTIVE_DOT_CLASS_NAME}
                status={activePresence}
              />
            </span>
          ) : (
            <button
              aria-label={hasError ? errorActionLabel : actionLabel}
              className={cn(
                "pointer-events-auto flex h-9 w-9 items-center justify-center rounded-full transition-colors duration-150 ease-out focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90",
                hasError
                  ? "bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  : "bg-primary text-primary-foreground hover:bg-primary/90",
              )}
              data-testid={hasError ? errorTestId : startTestId}
              disabled={isStarting}
              onClick={(event) => {
                event.stopPropagation();
                if (hasError) {
                  onOpenError?.();
                  return;
                }
                onStart();
              }}
              title={hasError ? errorLabel || errorActionLabel : actionLabel}
              type="button"
            >
              <span className="grid h-4 w-4 place-items-center">
                {isStarting ? (
                  <Spinner
                    aria-label={actionLabel}
                    className="h-4 w-4 border-2"
                  />
                ) : hasError ? (
                  <CircleAlert className="h-4 w-4" />
                ) : (
                  <Play className="h-4 w-4 fill-current" />
                )}
              </span>
            </button>
          )}
        </span>
      }
      badgeBox={badge.shell}
      className="h-24 w-24"
      curve={isActive ? STATUS_DOT_MASK_CURVE : ACTION_MASK_CURVE}
      cutout={badge.cutout}
      maskTransition={transition}
      size={AGENT_AVATAR_SIZE}
    >
      {trimmedAvatarUrl ? (
        <ProfileAvatar
          avatarUrl={trimmedAvatarUrl}
          className="h-full w-full bg-muted shadow-none"
          iconClassName="h-8 w-8"
          label={label}
        />
      ) : (
        <IdentityInitialsAvatar
          className="border-0 shadow-none"
          label={label}
          size={AGENT_AVATAR_SIZE}
        />
      )}
    </MaskedAvatarBadgeFrame>
  );
}
