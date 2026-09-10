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
  activeCameraSource: CameraSource | null;
  compact: boolean;
  computerDisabled: boolean;
  disabled: boolean;
  helpText: string | null;
  iphoneDisabled: boolean;
  isLive: boolean;
  isStarting: boolean;
  onRecord: () => void;
  onRetry?: () => void;
  onSelectSource: (source: CameraSource) => void;
  showCameraPicker: boolean;
  stackCameraOptions: boolean;
  testIdPrefix: string;
};

export function AnimatedAvatarCameraControls({
  activeCameraSource,
  compact,
  computerDisabled,
  disabled,
  helpText,
  iphoneDisabled,
  isLive,
  isStarting,
  onRecord,
  onRetry,
  onSelectSource,
  showCameraPicker,
  stackCameraOptions,
  testIdPrefix,
}: AnimatedAvatarCameraControlsProps) {
  const showCameraAction = Boolean(onRetry || isLive);
  const usesAnimatedStack = stackCameraOptions && showCameraPicker && !helpText;
  const reserveCameraAction = !stackCameraOptions || showCameraAction;
  const cameraAction = reserveCameraAction ? (
    <div className="h-14 pt-2">
      {onRetry ? (
        <Button
          className={cn(
            "h-12 w-full rounded-xl",
            compact &&
              "bg-[rgb(var(--buzz-onboarding-avatar-accent-bg))] text-[rgb(var(--buzz-onboarding-avatar-accent-fg))] hover:bg-[rgb(var(--buzz-onboarding-avatar-accent-bg))]",
          )}
          data-testid={`${testIdPrefix}-animated-retry`}
          disabled={disabled}
          onClick={onRetry}
          type="button"
        >
          Try camera again
        </Button>
      ) : isLive ? (
        <Button
          asChild
          className={cn(
            compact
              ? "h-[2.375rem] rounded-full bg-[rgb(var(--buzz-onboarding-avatar-action-bg))] px-6 text-sm font-medium text-[rgb(var(--buzz-onboarding-avatar-action-fg))] hover:bg-[color:rgb(var(--buzz-onboarding-avatar-action-bg)_/_0.9)]"
              : "h-12 w-full rounded-xl",
          )}
          data-testid={`${testIdPrefix}-animated-record`}
          disabled={disabled}
          onClick={onRecord}
          type="button"
        >
          <motion.button
            animate={{ opacity: 1 }}
            initial={{ opacity: 0 }}
            transition={ENTRANCE_TRANSITION}
          >
            <Video aria-hidden="true" className="mr-2 h-4 w-4" />
            Capture {RECORD_SECONDS} sec video
          </motion.button>
        </Button>
      ) : null}
    </div>
  ) : null;

  return (
    <div
      className={cn(
        "grid",
        usesAnimatedStack
          ? cn(
              "h-full min-h-0 overflow-hidden transition-[grid-template-rows,gap] duration-[250ms] ease-in-out motion-reduce:transition-none",
              showCameraAction
                ? "grid-rows-[minmax(0,1fr)_3.5rem] gap-4"
                : "grid-rows-[minmax(0,1fr)_0rem] gap-0",
            )
          : "gap-4",
      )}
    >
      {showCameraPicker ? (
        <AnimatedAvatarCameraPicker
          activeCameraSource={activeCameraSource}
          computerDisabled={computerDisabled}
          disabled={disabled || isStarting}
          iphoneDisabled={iphoneDisabled}
          onSelectSource={onSelectSource}
          stacked={stackCameraOptions}
          testIdPrefix={testIdPrefix}
        />
      ) : null}
      {helpText ? (
        <p className="px-1 text-center text-sm text-muted-foreground">
          {helpText}
        </p>
      ) : null}
      {usesAnimatedStack ? (
        <div className="min-h-0 overflow-hidden">{cameraAction}</div>
      ) : null}
      {!usesAnimatedStack && reserveCameraAction ? cameraAction : null}
    </div>
  );
}
