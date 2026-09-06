import { Video } from "lucide-react";
import { motion } from "motion/react";

import { AnimatedAvatarCameraPicker } from "@/features/profile/ui/AnimatedAvatarCameraPicker";
import {
  type CameraSource,
  ENTRANCE_TRANSITION,
  RECORD_SECONDS,
} from "@/features/profile/ui/AnimatedAvatarCapture.helpers";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type AnimatedAvatarCameraControlsProps = {
  actionButtonClassName?: string;
  activeCameraSource: CameraSource | null;
  compact: boolean;
  computerDisabled: boolean;
  dense?: boolean;
  disabled: boolean;
  helpText: string | null;
  iphoneDisabled: boolean;
  isLive: boolean;
  isStarting: boolean;
  onRecord: () => void;
  onRetry?: () => void;
  onSelectSource: (source: CameraSource) => void;
  showCameraPicker: boolean;
  testIdPrefix: string;
};

export function AnimatedAvatarCameraControls({
  actionButtonClassName,
  activeCameraSource,
  compact,
  computerDisabled,
  dense = false,
  disabled,
  helpText,
  iphoneDisabled,
  isLive,
  isStarting,
  onRecord,
  onRetry,
  onSelectSource,
  showCameraPicker,
  testIdPrefix,
}: AnimatedAvatarCameraControlsProps) {
  return (
    <div className={cn("grid gap-4", dense && "gap-2")}>
      {showCameraPicker ? (
        <AnimatedAvatarCameraPicker
          activeCameraSource={activeCameraSource}
          computerDisabled={computerDisabled}
          dense={dense}
          disabled={disabled || isStarting}
          iphoneDisabled={iphoneDisabled}
          onSelectSource={onSelectSource}
          testIdPrefix={testIdPrefix}
        />
      ) : null}
      {helpText ? (
        <p className="px-1 text-center text-sm text-muted-foreground">
          {helpText}
        </p>
      ) : null}
      {onRetry || isLive ? (
        <div
          className={cn(
            "h-14 pt-2",
            compact && "flex justify-center",
            dense && "h-10 pt-1",
            actionButtonClassName && "h-10 pt-0",
          )}
        >
          {onRetry ? (
            <Button
              className={
                actionButtonClassName ??
                cn(
                  "h-12 w-full rounded-xl",
                  dense && "h-9",
                  compact &&
                    "bg-[rgb(var(--buzz-onboarding-avatar-accent-bg))] text-[rgb(var(--buzz-onboarding-avatar-accent-fg))] hover:bg-[rgb(var(--buzz-onboarding-avatar-accent-bg))]",
                )
              }
              data-testid={`${testIdPrefix}-animated-retry`}
              disabled={disabled}
              onClick={onRetry}
              type="button"
            >
              Try camera again
            </Button>
          ) : isLive ? (
            <Button
              className={
                actionButtonClassName ??
                (compact
                  ? "h-[2.375rem] rounded-full bg-[rgb(var(--buzz-onboarding-avatar-action-bg))] px-6 text-sm font-medium text-[rgb(var(--buzz-onboarding-avatar-action-fg))] hover:bg-[color:rgb(var(--buzz-onboarding-avatar-action-bg)_/_0.9)]"
                  : "h-12 w-full rounded-xl")
              }
              data-testid={`${testIdPrefix}-animated-record`}
              disabled={disabled}
              onClick={onRecord}
              type="button"
            >
              <motion.span
                animate={{ opacity: 1 }}
                className="inline-flex items-center"
                initial={{ opacity: 0 }}
                transition={ENTRANCE_TRANSITION}
              >
                {actionButtonClassName ? null : (
                  <Video aria-hidden="true" className="mr-2 h-4 w-4" />
                )}
                Capture {RECORD_SECONDS} sec video
              </motion.span>
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
