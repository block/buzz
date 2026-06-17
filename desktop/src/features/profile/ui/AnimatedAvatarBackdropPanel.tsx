import {
  AVATAR_COLOR_SWATCHES,
  CUSTOM_AVATAR_COLOR_SWATCH,
  contrastColorForBackground,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";

type AnimatedAvatarBackdropPanelProps = {
  backdropColor: string | null;
  disabled?: boolean;
  isCustomBackdropSelected: boolean;
  isSaving: boolean;
  onOpenCustomPicker: () => void;
  onSelectColor: (color: string) => void;
  testIdPrefix: string;
};

export function AnimatedAvatarBackdropPanel({
  backdropColor,
  disabled = false,
  isCustomBackdropSelected,
  isSaving,
  onOpenCustomPicker,
  onSelectColor,
  testIdPrefix,
}: AnimatedAvatarBackdropPanelProps) {
  return (
    <div
      className="grid gap-4 rounded-xl bg-muted p-4 transition-colors duration-[250ms] ease-out"
      data-testid={`${testIdPrefix}-animated-color-panel`}
    >
      <div
        className="grid grid-cols-8 justify-items-center gap-3"
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
              className="relative h-10 w-10 rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
                  className="absolute inset-1 rounded-full border-[3px]"
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
    </div>
  );
}
