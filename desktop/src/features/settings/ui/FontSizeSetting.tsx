import { useMemo } from "react";
import { Minus, Plus } from "lucide-react";

import {
  FONT_SCALE_PRESETS,
  setFontScale,
  useFontScale,
} from "@/shared/lib/fontScalePreference";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

/**
 * Font-size preference control. Lets the user pick a base font scale (85%–130%)
 * to improve readability on high-DPI or unusually-scaled displays.
 *
 * The value is persisted in localStorage and applied to the document root
 * as a CSS `font-size` multiplier via the {@link FontScaleApplier} component.
 */
export function FontSizeSetting() {
  const fontScale = useFontScale();

  const { sliderPercent, label } = useMemo(() => {
    const min = FONT_SCALE_PRESETS[0];
    const max = FONT_SCALE_PRESETS[FONT_SCALE_PRESETS.length - 1];
    const percent = ((fontScale - min) / (max - min)) * 100;
    return {
      sliderPercent: percent,
      label: `${Math.round(fontScale * 100)}%`,
    };
  }, [fontScale]);

  const currentIndex = FONT_SCALE_PRESETS.indexOf(
    fontScale as (typeof FONT_SCALE_PRESETS)[number],
  );
  const canDecrease = currentIndex > 0;
  const canIncrease =
    currentIndex >= 0 && currentIndex < FONT_SCALE_PRESETS.length - 1;

  const handleDecrease = () => {
    if (canDecrease) {
      setFontScale(FONT_SCALE_PRESETS[currentIndex - 1]);
    }
  };

  const handleIncrease = () => {
    if (canIncrease) {
      setFontScale(FONT_SCALE_PRESETS[currentIndex + 1]);
    }
  };

  return (
    <SettingsOptionGroup className="mt-4">
      <SettingsOptionRow>
        <div className="min-w-0">
          <p className="text-sm font-medium">Font size</p>
          <p className="text-sm font-normal text-muted-foreground">
            Scale the base text size for readability on high-DPI displays.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            aria-label="Decrease font size"
            className="h-7 w-7 shrink-0 rounded-full border border-border/50 bg-muted/45 p-0 text-foreground shadow-none hover:bg-muted/70"
            data-testid="font-size-decrease"
            disabled={!canDecrease}
            onClick={handleDecrease}
            size="sm"
            type="button"
            variant="ghost"
          >
            <Minus className="h-4 w-4" />
          </Button>
          <span
            className="min-w-12 text-center text-xs font-medium tabular-nums"
            data-testid="font-size-label"
          >
            {label}
          </span>
          <Button
            aria-label="Increase font size"
            className="h-7 w-7 shrink-0 rounded-full border border-border/50 bg-muted/45 p-0 text-foreground shadow-none hover:bg-muted/70"
            data-testid="font-size-increase"
            disabled={!canIncrease}
            onClick={handleIncrease}
            size="sm"
            type="button"
            variant="ghost"
          >
            <Plus className="h-4 w-4" />
          </Button>
        </div>
      </SettingsOptionRow>
      {/* Native range slider for direct/drag interaction */}
      <input
        aria-label="Font size slider"
        className={cn(
          "mt-2 w-full cursor-pointer accent-primary",
          "h-1.5 appearance-none rounded-full bg-muted",
        )}
        data-testid="font-size-slider"
        max={FONT_SCALE_PRESETS[FONT_SCALE_PRESETS.length - 1]}
        min={FONT_SCALE_PRESETS[0]}
        onChange={(e) => setFontScale(Number.parseFloat(e.target.value))}
        step={0.01}
        style={{
          background: `linear-gradient(to right, hsl(var(--primary)) ${sliderPercent}%, hsl(var(--muted)) ${sliderPercent}%)`,
        }}
        type="range"
        value={fontScale}
      />
    </SettingsOptionGroup>
  );
}
