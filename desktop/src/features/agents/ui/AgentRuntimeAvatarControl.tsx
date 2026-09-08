import { CircleAlert } from "lucide-react";
import { useReducedMotion } from "motion/react";

import {
  type AvatarBadgeCurve,
  MaskedAvatarBadgeFrame,
  STATUS_DOT_MASK_CURVE,
} from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  getPresenceDotClassName,
  getPresenceLabel,
} from "@/features/presence/lib/presence";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";
import { agentPresenceStartBlockReason } from "../lib/useAgentAvailability";
import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type AgentRuntimeAvatarControlProps = {
  activeTestId: string;
  avatarUrl?: string | null;
  errorLabel?: string | null;
  errorTestId?: string;
  /** Lifecycle bookkeeping controls actions, not the availability dot. */
  isActive: boolean;
  availability?: PresenceStatus;
  isRestarting?: boolean;
  isStarting: boolean;
  label: string;
  requiresRestart?: boolean;
  startTestId: string;
  onOpenError?: () => void;
  onStart: () => void;
};

const TAILWIND_SPACING = {
  "1": 4,
  "2": 8,
  "2.5": 10,
  "3.5": 14,
  "6": 24,
  "11": 44,
  "24": 96,
} as const;

const AGENT_AVATAR_SIZE = TAILWIND_SPACING["24"];
const ACTION_BADGE_SIZE = TAILWIND_SPACING["11"];
const ACTION_BUTTON_HEIGHT = 36;
const START_ACTION_BADGE_WIDTH = 56;
const RESTART_ACTION_BADGE_WIDTH = 72;
const ACTIVE_BADGE_CUTOUT_SIZE = TAILWIND_SPACING["6"];
const ACTIVE_DOT_SIZE = 18;
const ACTION_BADGE_OFFSET = TAILWIND_SPACING["3.5"] + TAILWIND_SPACING["1"];
const ACTIVE_BADGE_INSET = TAILWIND_SPACING["1"];
const PROFILE_STATUS_CUTOUT_RATIO = 1.25;

function getBadgeCenter(badgeSize: number, outwardOffset: number) {
  return AGENT_AVATAR_SIZE + outwardOffset - badgeSize / 2;
}

function getActionBadge(width: number, height: number, offset: number) {
  const centerY = getBadgeCenter(ACTION_BADGE_SIZE, offset);
  const clearance = (ACTION_BADGE_SIZE - height) / 2;

  return {
    cutout: {
      // Keep the cutout on the avatar edge so the mask has the same soft,
      // two-point join as the status dot. Unlike the status dot, center the
      // primary action horizontally to make its purpose easier to spot.
      cx: AGENT_AVATAR_SIZE / 2,
      cy: centerY,
      r: ACTION_BADGE_SIZE / 2,
    },
    shell: {
      bottom: AGENT_AVATAR_SIZE - centerY - height / 2,
      height,
      right: (AGENT_AVATAR_SIZE - width) / 2,
      width,
    },
    // Carry the vertical clearance around the end caps horizontally too, so
    // the avatar gap stays even around the pill.
    cutoutWidth: width + clearance * 2,
  } as const;
}

function getActiveBadge(inset: number) {
  const center = getBadgeCenter(ACTIVE_BADGE_CUTOUT_SIZE, -inset);

  return {
    cutout: {
      cx: center,
      cy: center,
      r: (ACTIVE_BADGE_CUTOUT_SIZE / 2) * PROFILE_STATUS_CUTOUT_RATIO,
    },
    shell: {
      bottom: AGENT_AVATAR_SIZE - center - ACTIVE_DOT_SIZE / 2,
      height: ACTIVE_DOT_SIZE,
      right: AGENT_AVATAR_SIZE - center - ACTIVE_DOT_SIZE / 2,
      width: ACTIVE_DOT_SIZE,
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

const START_ACTION_BADGE = getActionBadge(
  START_ACTION_BADGE_WIDTH,
  ACTION_BUTTON_HEIGHT,
  ACTION_BADGE_OFFSET,
);
const RESTART_ACTION_BADGE = getActionBadge(
  RESTART_ACTION_BADGE_WIDTH,
  ACTION_BUTTON_HEIGHT,
  ACTION_BADGE_OFFSET,
);
const ERROR_BADGE = getActionBadge(
  ACTION_BADGE_SIZE,
  ACTION_BUTTON_HEIGHT,
  ACTION_BADGE_OFFSET,
);
const ACTIVE_BADGE = getActiveBadge(ACTIVE_BADGE_INSET);

const MASK_TRANSITION = {
  duration: 0.3,
  ease: [0.4, 0, 0.2, 1],
} as const;

export function AgentRuntimeAvatarControl({
  activeTestId,
  avatarUrl,
  availability,
  errorLabel,
  errorTestId,
  isActive,
  isRestarting = false,
  isStarting,
  label,
  requiresRestart = false,
  startTestId,
  onOpenError,
  onStart,
}: AgentRuntimeAvatarControlProps) {
  const shouldReduceMotion = useReducedMotion();
  const trimmedAvatarUrl = avatarUrl?.trim() || null;
  const isRestartAction = requiresRestart || isRestarting;
  const actionLabel = isRestarting
    ? "Restarting Agent"
    : isStarting
      ? "Starting Agent"
      : isRestartAction
        ? "Restart Agent"
        : "Start Agent";
  const actionText = isRestartAction ? "Restart" : "Start";
  const isPending = isStarting || isRestarting;
  const availabilityLabel = availability
    ? getPresenceLabel(availability)
    : "Availability unknown";
  const startBlockReason = agentPresenceStartBlockReason(
    isActive,
    availability,
  );
  // A present identity need not be a process this supervisor owns. Replace
  // Start (even a stale Restart/error badge) without inventing Stop authority.
  const showStatusDot =
    Boolean(startBlockReason) || (isActive && !isRestartAction);
  const hasError = !isActive && !isPending && Boolean(errorLabel);
  const errorActionLabel = `${label} has a runtime error. Open runtime details.`;
  const transition = shouldReduceMotion ? { duration: 0 } : MASK_TRANSITION;
  const actionBadge = isRestartAction
    ? RESTART_ACTION_BADGE
    : START_ACTION_BADGE;
  const badge = showStatusDot
    ? ACTIVE_BADGE
    : hasError
      ? ERROR_BADGE
      : actionBadge;
  const actionCutoutWidth =
    showStatusDot || hasError ? undefined : actionBadge.cutoutWidth;

  return (
    <MaskedAvatarBadgeFrame
      badge={
        <span className="grid h-full w-full place-items-center">
          {showStatusDot ? (
            <span
              aria-label={`${label}: ${availabilityLabel}`}
              className="h-full w-full rounded-full"
              data-testid={activeTestId}
              role="img"
              title={startBlockReason ?? `${label}: ${availabilityLabel}`}
            />
          ) : (
            <button
              aria-label={hasError ? errorActionLabel : actionLabel}
              className={cn(
                "pointer-events-auto flex h-full w-full items-center justify-center rounded-full px-2.5 text-xs font-semibold leading-none transition-colors duration-150 ease-out focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90",
                hasError
                  ? "bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  : isRestartAction
                    ? "bg-transparent text-amber-800 hover:bg-amber-500/10 dark:text-amber-400"
                    : "bg-primary text-primary-foreground hover:bg-primary/90",
              )}
              data-testid={hasError ? errorTestId : startTestId}
              disabled={isPending}
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
              {isPending ? (
                <Spinner
                  aria-label={actionLabel}
                  className="h-4 w-4 border-2"
                />
              ) : hasError ? (
                <CircleAlert className="h-4 w-4" />
              ) : (
                actionText
              )}
            </button>
          )}
        </span>
      }
      badgeBox={badge.shell}
      badgeClassName={cn(
        "transition-colors ease-in-out",
        shouldReduceMotion ? "duration-0" : "duration-300",
        showStatusDot
          ? availability
            ? getPresenceDotClassName(availability)
            : "bg-muted-foreground/35"
          : hasError
            ? "bg-destructive"
            : isRestartAction
              ? "bg-amber-500/15"
              : "bg-primary",
      )}
      className="h-24 w-24"
      cornerRadius={AGENT_AVATAR_SIZE * 0.3}
      curve={showStatusDot ? STATUS_DOT_MASK_CURVE : ACTION_MASK_CURVE}
      cutout={badge.cutout}
      cutoutWidth={actionCutoutWidth}
      maskTransition={transition}
      size={AGENT_AVATAR_SIZE}
    >
      {trimmedAvatarUrl ? (
        <ProfileAvatar
          avatarUrl={trimmedAvatarUrl}
          className="h-full w-full bg-muted shadow-none"
          iconClassName="h-8 w-8"
          label={label}
          shape="squircle"
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
