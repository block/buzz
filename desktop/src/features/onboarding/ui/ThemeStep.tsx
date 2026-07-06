import { Check } from "lucide-react";
import * as React from "react";

import {
  ACCENT_COLORS,
  ACCENT_STORAGE_KEY,
  NEUTRAL_ACCENT,
  THEME_STORAGE_KEY,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import {
  ONBOARDING_DEFAULT_THEME_NAME,
  SYNTAX_THEMES,
  type SyntaxThemeName,
} from "@/shared/theme/theme-loader";
import {
  ThemePreviewFrame,
  type ThemePreviewVars,
} from "@/shared/theme/ThemePreviewFrame";
import {
  getThemeFallbackPreviewVars,
  useThemePreviewVars,
  withAccentPreviewVars,
} from "@/shared/theme/useThemePreviewVars";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { StepProgress } from "@/shared/ui/step-progress";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { ThemeStepActions } from "./types";

type ThemeStepProps = {
  actions: ThemeStepActions;
  direction: OnboardingTransitionDirection;
};

const GRADUAL_BLUR_LEVELS = [0.5, 1.25, 2.5, 4.5, 7, 10] as const;
const THEME_TILE_WIDTH = 174;
const THEME_TILE_HEIGHT = 160;
const THEME_TILE_GAP = 12;
const THEME_VISIBLE_ROW_COUNT = 2;
const THEME_ROW_PEEK_HEIGHT = 48;
const THEME_SCROLL_MAX_HEIGHT =
  THEME_TILE_HEIGHT * THEME_VISIBLE_ROW_COUNT +
  THEME_TILE_GAP * THEME_VISIBLE_ROW_COUNT +
  THEME_ROW_PEEK_HEIGHT;

function contrastColorForHex(hex: string) {
  const match = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  if (!match) {
    return "#ffffff";
  }

  const r = Number.parseInt(match[1], 16);
  const g = Number.parseInt(match[2], 16);
  const b = Number.parseInt(match[3], 16);
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.5 ? "#000000" : "#ffffff";
}

function formatThemeLabel(name: string): string {
  return name
    .split("-")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function getOrderedThemes() {
  return [
    ONBOARDING_DEFAULT_THEME_NAME,
    ...SYNTAX_THEMES.filter((name) => name !== ONBOARDING_DEFAULT_THEME_NAME),
  ];
}

export { preloadThemePreviewVars } from "@/shared/theme/useThemePreviewVars";

function GradualBottomBlur() {
  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute inset-x-0 -bottom-4 z-10 h-28 overflow-hidden"
    >
      {GRADUAL_BLUR_LEVELS.map((blur, index) => {
        const transparentStop = 96 - index * 11;
        const solidStop = Math.max(0, transparentStop - 28);
        const maskImage = `linear-gradient(to top, black 0%, black ${solidStop}%, transparent ${transparentStop}%)`;

        return (
          <div
            className="absolute inset-0"
            key={blur}
            style={{
              WebkitBackdropFilter: `blur(${blur}px)`,
              WebkitMaskImage: maskImage,
              backdropFilter: `blur(${blur}px)`,
              maskImage,
            }}
          />
        );
      })}
      <div
        className="absolute inset-0"
        style={{
          background:
            "linear-gradient(to top, hsl(var(--background)) 0%, hsl(var(--background) / 0.64) 30%, hsl(var(--background) / 0.18) 64%, transparent 100%)",
        }}
      />
    </div>
  );
}

function ThemeTile({
  isActive,
  name,
  onSelect,
  vars,
}: {
  isActive: boolean;
  name: SyntaxThemeName;
  onSelect: () => void;
  vars: ThemePreviewVars | null;
}) {
  return (
    <button
      aria-pressed={isActive}
      className={cn(
        "group flex w-[174px] min-w-0 flex-col rounded-lg border bg-background/70 p-2 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        isActive
          ? "border-primary text-foreground shadow-sm"
          : "border-border/70 text-muted-foreground hover:border-border hover:bg-accent/70 hover:text-accent-foreground",
      )}
      data-testid={`onboarding-theme-option-${name}`}
      onClick={onSelect}
      type="button"
    >
      <ThemePreviewFrame vars={vars} />

      <div className="mt-2 flex min-h-6 items-center gap-2 px-1">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {formatThemeLabel(name)}
        </span>
        {isActive ? <Check className="h-4 w-4 shrink-0 text-primary" /> : null}
      </div>
    </button>
  );
}

function AccentColorPicker({
  accentColor,
  onSelect,
}: {
  accentColor: string;
  onSelect: (value: string) => void;
}) {
  return (
    <div className="mx-auto mt-6 w-fit max-w-full rounded-xl bg-muted p-3">
      <div className="flex flex-wrap items-center justify-center gap-2">
        {ACCENT_COLORS.map((color) => {
          const isNeutral = color.value === NEUTRAL_ACCENT;
          const isSelected = accentColor === color.value;
          const swatchBackground = isNeutral
            ? "hsl(var(--foreground))"
            : color.value;
          const selectedRingColor = isNeutral
            ? "hsl(var(--background))"
            : contrastColorForHex(color.value);

          return (
            <button
              aria-label={`Use ${color.name} accent color`}
              aria-pressed={isSelected}
              className="relative h-9 w-9 rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.12] focus-visible:scale-[1.12] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              data-testid={`onboarding-accent-color-${color.name.toLowerCase()}`}
              key={color.value}
              onClick={() => onSelect(color.value)}
              style={{ background: swatchBackground }}
              title={color.name}
              type="button"
            >
              {isSelected ? (
                <span
                  className="absolute inset-1 rounded-full border-[3px]"
                  style={{
                    borderColor: selectedRingColor,
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

export function ThemeStep({ actions, direction }: ThemeStepProps) {
  const { skip, submit } = actions;
  const { accentColor, setAccentColor, setTheme, themeName } = useTheme();
  const previewVarsByTheme = useThemePreviewVars();
  const orderedThemes = React.useMemo(() => getOrderedThemes(), []);

  React.useEffect(() => {
    const hasStoredTheme =
      window.localStorage.getItem(THEME_STORAGE_KEY) !== null;
    const hasStoredAccent =
      window.localStorage.getItem(ACCENT_STORAGE_KEY) !== null;

    if (!hasStoredTheme && themeName !== ONBOARDING_DEFAULT_THEME_NAME) {
      setTheme(ONBOARDING_DEFAULT_THEME_NAME);
    }

    if (!hasStoredAccent && accentColor !== NEUTRAL_ACCENT) {
      setAccentColor(NEUTRAL_ACCENT);
    }
  }, [accentColor, setAccentColor, setTheme, themeName]);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center pb-40 text-center lg:pb-0"
      data-testid="onboarding-page-theme"
      direction={direction}
      transitionKey={`theme-${direction}`}
    >
      <div className="grid w-full max-w-[1180px] items-start gap-12 lg:grid-cols-[minmax(260px,320px)_minmax(0,760px)] lg:gap-14">
        <div className="flex w-full flex-col items-center text-center lg:items-start lg:text-left">
          <div className="w-full max-w-[360px]">
            <h1 className="text-3xl font-semibold text-foreground">
              Pick a theme
            </h1>
            <p className="mt-3 text-sm leading-6 text-muted-foreground">
              Choose a look that makes Buzz feel like yours.
            </p>
          </div>
        </div>

        <div className="w-full">
          <div className="relative w-full">
            <div
              className="overflow-y-auto pb-20 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
              style={{
                maxHeight: `min(60dvh, ${THEME_SCROLL_MAX_HEIGHT}px)`,
              }}
            >
              <div
                className="grid justify-center gap-3"
                style={{
                  gridTemplateColumns: `repeat(auto-fill, ${THEME_TILE_WIDTH}px)`,
                }}
              >
                {orderedThemes.map((name) => {
                  const vars = withAccentPreviewVars(
                    previewVarsByTheme[name] ??
                      getThemeFallbackPreviewVars(name),
                    accentColor,
                  );

                  return (
                    <ThemeTile
                      isActive={themeName === name}
                      key={name}
                      name={name}
                      onSelect={() => setTheme(name)}
                      vars={vars}
                    />
                  );
                })}
              </div>
            </div>
            <GradualBottomBlur />
          </div>

          <AccentColorPicker
            accentColor={accentColor}
            onSelect={setAccentColor}
          />

          <div className="mt-10 flex w-full flex-col gap-3 lg:mx-auto lg:max-w-[500px] max-lg:fixed max-lg:inset-x-0 max-lg:bottom-0 max-lg:z-40 max-lg:mt-0 max-lg:max-w-none max-lg:border-t max-lg:border-border max-lg:bg-background max-lg:p-4 max-lg:pb-[max(1rem,env(safe-area-inset-bottom))]">
            <Button
              className="h-10 w-full"
              data-testid="onboarding-next"
              onClick={submit}
              type="button"
            >
              Next
            </Button>

            <Button
              className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
              data-testid="onboarding-skip"
              onClick={skip}
              type="button"
              variant="ghost"
            >
              Skip
            </Button>

            <StepProgress
              activeSegmentClassName="bg-primary"
              className="mt-1 lg:hidden"
              completeSegmentClassName="bg-primary/35"
              currentStep={4}
              inactiveSegmentClassName="bg-muted-foreground/25"
            />
          </div>
        </div>
      </div>
    </OnboardingSlideTransition>
  );
}
