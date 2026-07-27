import { cn } from "@/shared/lib/cn";

import {
  AVATAR_COLOR_SWATCHES,
  CUSTOM_AVATAR_COLOR_SWATCH,
  contrastColorForBackground,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";

type AnimatedAvatarBackdropPanelProps = {
  backdropColor: string | null;
  compact?: boolean;
  disabled?: boolean;
  isCustomBackdropSelected: boolean;
  isSaving: boolean;
  onOpenCustomPicker: () => void;
  onSelectColor: (color: string) => void;
  testIdPrefix: string;
};

export function AnimatedAvatarBackdropPanel({
  backdropColor,
  compact = false,
  disabled = false,
  isCustomBackdropSelected,
  isSaving,
  onOpenCustomPicker,
  onSelectColor,
  testIdPrefix,
}: AnimatedAvatarBackdropPanelProps) {
  return (
    <div
      className={cn(
        "grid grid-cols-8 justify-items-center rounded-xl bg-muted transition-colors duration-[250ms] ease-out",
        compact ? "gap-2 p-3" : "gap-3 p-4",
      )}
      data-testid={`${testIdPrefix}-animated-backdrop-grid`}
    >
      {AVATAR_COLOR_SWATCHES.map((swatch) => {
        const isCustomSwatch = swatch === CUSTOM_AVATAR_COLOR_SWATCH;
        const isSelected = isCustomSwatch
          ? isCustomBackdropSelected
          : backdropColor !== null &&
            swatch.toUpperCase() === backdropColor.toUpperCase();

        return (
          <button
            aria-label={
              isCustomSwatch
                ? "Choose custom backdrop color"
                : `Use ${swatch} backdrop`
            }
            aria-pressed={isSelected}
            className={cn(
              "relative rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              compact ? "h-7 w-7" : "h-10 w-10",
            )}
            data-testid={
              isCustomSwatch
                ? `${testIdPrefix}-animated-backdrop-custom`
                : undefined
            }
            disabled={disabled || isSaving}
            key={swatch}
            onClick={() => {
              if (isCustomSwatch) {
                onOpenCustomPicker();
                return;
              }
              onSelectColor(swatch);
            }}
            style={{
              background: isCustomSwatch
                ? isSelected && backdropColor
                  ? backdropColor
                  : "conic-gradient(from 0deg, #ff4d4d, #ffe75c, #73ef75, #63c6f2, #b141ff, #ff4d4d)"
                : swatch,
            }}
            type="button"
          >
            {isSelected ? (
              <span
                className={cn(
                  "absolute rounded-full border-[3px]",
                  compact ? "inset-0.5" : "inset-1",
                )}
                style={{
                  borderColor: contrastColorForBackground(
                    isCustomSwatch && backdropColor ? backdropColor : swatch,
                  ),
                }}
              />
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
