import { Check } from "lucide-react";
import * as React from "react";

import { createThemeVars, hexToHsl } from "@/shared/theme/adaptive-theme";
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
  extractThemeInfo,
  isLightTheme,
  loadThemeData,
} from "@/shared/theme/theme-loader";
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

type ThemePreviewVars = Record<string, string>;

const LIGHT_PREVIEW_VARS: ThemePreviewVars = {
  "--background": "0 0% 100%",
  "--border": "0 0% 89.8%",
  "--foreground": "0 0% 9%",
  "--muted": "0 0% 96.1%",
  "--muted-foreground": "0 0% 45.1%",
  "--primary": "0 0% 9%",
  "--sidebar-background": "0 0% 98%",
  "--sidebar-foreground": "0 0% 9%",
};

const DARK_PREVIEW_VARS: ThemePreviewVars = {
  "--background": "0 0% 3.9%",
  "--border": "0 0% 14.9%",
  "--foreground": "0 0% 98%",
  "--muted": "0 0% 14.9%",
  "--muted-foreground": "0 0% 63.9%",
  "--primary": "0 0% 98%",
  "--sidebar-background": "0 0% 0%",
  "--sidebar-foreground": "0 0% 98%",
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

type ThemePreviewVarsByTheme = Partial<
  Record<SyntaxThemeName, ThemePreviewVars>
>;

let themePreviewVarsCache: ThemePreviewVarsByTheme | null = null;
let themePreviewVarsPromise: Promise<ThemePreviewVarsByTheme> | null = null;

function hsl(vars: ThemePreviewVars | null, key: string) {
  return `hsl(${vars?.[key] ?? LIGHT_PREVIEW_VARS[key]})`;
}

function hslAlpha(vars: ThemePreviewVars | null, key: string, alpha: number) {
  return `hsl(${vars?.[key] ?? LIGHT_PREVIEW_VARS[key]} / ${alpha})`;
}

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

function withAccentPreviewVars(
  vars: ThemePreviewVars | null,
  accentColor: string,
): ThemePreviewVars | null {
  if (!vars) {
    return null;
  }

  if (accentColor === NEUTRAL_ACCENT) {
    return {
      ...vars,
      "--primary": vars["--foreground"],
      "--primary-foreground": vars["--background"],
    };
  }

  return {
    ...vars,
    "--primary": hexToHsl(accentColor),
  };
}

function getOrderedThemes() {
  return [
    ONBOARDING_DEFAULT_THEME_NAME,
    ...SYNTAX_THEMES.filter((name) => name !== ONBOARDING_DEFAULT_THEME_NAME),
  ];
}

function getThemeFallbackPreviewVars(name: SyntaxThemeName) {
  return isLightTheme(name) ? LIGHT_PREVIEW_VARS : DARK_PREVIEW_VARS;
}

async function loadThemePreviewVars(name: SyntaxThemeName) {
  const themeData = await loadThemeData(name);
  const info = extractThemeInfo(name, themeData);
  const { vars } = createThemeVars(info.bg, info.fg, info.comment, {
    added: info.added,
    deleted: info.deleted,
    modified: info.modified,
  });
  return [name, vars] as const;
}

export function preloadThemePreviewVars() {
  if (themePreviewVarsCache) {
    return Promise.resolve(themePreviewVarsCache);
  }

  if (!themePreviewVarsPromise) {
    themePreviewVarsPromise = Promise.all(
      SYNTAX_THEMES.map((name) => loadThemePreviewVars(name)),
    )
      .then((entries) => {
        const previewVars = Object.fromEntries(
          entries,
        ) as ThemePreviewVarsByTheme;
        themePreviewVarsCache = previewVars;
        return previewVars;
      })
      .catch((error) => {
        themePreviewVarsPromise = null;
        throw error;
      });
  }

  return themePreviewVarsPromise;
}

function useThemePreviewVars() {
  const [previewVarsByTheme, setPreviewVarsByTheme] =
    React.useState<ThemePreviewVarsByTheme>(() => themePreviewVarsCache ?? {});

  React.useEffect(() => {
    let canceled = false;

    void preloadThemePreviewVars()
      .then((previewVars) => {
        if (!canceled) {
          setPreviewVarsByTheme(previewVars);
        }
      })
      .catch(() => {
        if (!canceled) {
          setPreviewVarsByTheme({});
        }
      });

    return () => {
      canceled = true;
    };
  }, []);

  return previewVarsByTheme;
}

function ThemePreviewSvg({ vars }: { vars: ThemePreviewVars | null }) {
  const clipId = React.useId().replace(/:/g, "");
  const background = hsl(vars, "--background");
  const border = hsl(vars, "--border");
  const foreground = hsl(vars, "--foreground");
  const mutedForeground = hsl(vars, "--muted-foreground");
  const primary = hsl(vars, "--primary");
  const primarySoft = hslAlpha(vars, "--primary", 0.68);
  const sidebar = hsl(vars, "--sidebar-background");
  const sidebarForeground = hslAlpha(vars, "--sidebar-foreground", 0.58);

  return (
    <svg
      aria-hidden="true"
      className="h-24 w-[142px] shrink-0 drop-shadow-sm"
      fill="none"
      viewBox="0 0 118 80"
      xmlns="http://www.w3.org/2000/svg"
    >
      <g clipPath={`url(#${clipId})`}>
        <rect fill={background} height="180" rx="3.6" width="288" />
        <line stroke={border} x1="57" x2="117" y1="10.5" y2="10.5" />
        <rect fill={sidebar} height="180" width="57.375" />
        <rect
          fill={sidebarForeground}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="15.9751"
        />
        <rect
          fill={sidebarForeground}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="21.375"
        />
        <rect
          fill={sidebarForeground}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="26.7749"
        />
        <rect
          fill={sidebarForeground}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="32.175"
        />
        <rect
          fill="#FF5F57"
          height="2.7"
          rx="1.35"
          width="2.7"
          x="3.5"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="3.55625"
          y="4.7811"
        />
        <rect
          fill="#FEBC2E"
          height="2.7"
          rx="1.35"
          width="2.7"
          x="8"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="8.05625"
          y="4.7811"
        />
        <rect
          fill="#28C840"
          height="2.7"
          rx="1.35"
          width="2.7"
          x="12.5"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="12.5563"
          y="4.7811"
        />
        <rect
          fill={sidebarForeground}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="16.875"
        />
        <rect
          fill={sidebarForeground}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="22.2749"
        />
        <rect
          fill={sidebarForeground}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="27.675"
        />
        <rect
          fill={sidebarForeground}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="33.075"
        />
        <rect
          fill={mutedForeground}
          height="1.8"
          rx="0.225"
          width="26.775"
          x="3.60156"
          y="43.875"
        />
        <rect fill={foreground} height="2" rx="0.5" width="21" x="60" y="4" />
        <rect fill={primary} height="4" rx="1" width="4" x="105" y="3" />
        <rect fill={primarySoft} height="4" rx="1" width="4" x="111" y="3" />
      </g>
      <defs>
        <clipPath id={clipId}>
          <rect fill={background} height="180" rx="3.6" width="288" />
        </clipPath>
      </defs>
    </svg>
  );
}

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

function ThemePreviewFrame({
  className,
  vars,
}: {
  className?: string;
  vars: ThemePreviewVars | null;
}) {
  return (
    <div
      className={cn(
        "relative h-28 w-[158px] overflow-hidden rounded-md border",
        className,
      )}
      style={{
        backgroundColor: hsl(vars, "--muted"),
        borderColor: hsl(vars, "--border"),
      }}
    >
      <div className="absolute bottom-0 right-0">
        <ThemePreviewSvg vars={vars} />
      </div>
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
