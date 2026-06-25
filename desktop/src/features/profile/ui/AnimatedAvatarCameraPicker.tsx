import { Smartphone, Webcam } from "lucide-react";

import type { CameraSource } from "@/features/profile/ui/AnimatedAvatarCapture.helpers";
import { cn } from "@/shared/lib/cn";

type AnimatedAvatarCameraPickerProps = {
  activeCameraSource: CameraSource | null;
  computerDisabled: boolean;
  disabled?: boolean;
  iphoneDisabled: boolean;
  onSelectSource: (source: CameraSource) => void;
  testIdPrefix: string;
};

export function AnimatedAvatarCameraPicker({
  activeCameraSource,
  computerDisabled,
  disabled = false,
  iphoneDisabled,
  onSelectSource,
  testIdPrefix,
}: AnimatedAvatarCameraPickerProps) {
  return (
    <div className="grid grid-cols-2 gap-3">
      {[
        {
          disabled: iphoneDisabled,
          icon: Smartphone,
          label: "Use iPhone",
          source: "iphone" as const,
        },
        {
          disabled: computerDisabled,
          icon: Webcam,
          label: "Use this computer",
          source: "computer" as const,
        },
      ].map((option) => {
        const Icon = option.icon;
        const isSelected = activeCameraSource === option.source;
        const isDisabled = disabled || option.disabled;
        return (
          <button
            aria-pressed={isSelected}
            className={cn(
              "relative flex h-[120px] flex-col items-center justify-center gap-3 overflow-hidden rounded-xl border border-transparent bg-muted text-foreground transition-[background-color,border-color,color,opacity] duration-[250ms] ease-out hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              isSelected &&
                "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10",
              isDisabled && "cursor-not-allowed opacity-45 hover:bg-muted",
            )}
            data-testid={`${testIdPrefix}-animated-camera-${option.source}`}
            disabled={isDisabled}
            key={option.source}
            onClick={() => onSelectSource(option.source)}
            type="button"
          >
            <Icon
              aria-hidden="true"
              className={cn(
                "h-5 w-5 text-foreground transition-colors duration-[250ms] ease-out",
                isSelected && "text-primary",
              )}
            />
            <span
              className={cn(
                "text-sm font-medium text-foreground transition-colors duration-[250ms] ease-out",
                isSelected && "text-primary",
              )}
            >
              {option.label}
            </span>
          </button>
        );
      })}
    </div>
  );
}
