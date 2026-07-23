import * as React from "react";
import { cn } from "@/shared/lib/cn";

export type ThemePreviewVars = Record<string, string>;

/**
 * Buzz sidebar-gradient stop tokens, keyed by theme name. The actual custom
 * colors live once in `shared/styles/globals/theme.css`; the picker consumes
 * those same semantic variables instead of duplicating raw values here.
 */
export const BUZZ_GRADIENT_STOPS: Record<
  string,
  { top: string; bottom: string }
> = {
  buzz: {
    top: "var(--buzz-gradient-light-top)",
    bottom: "var(--buzz-gradient-light-bottom)",
  },
  "buzz-dark": {
    top: "var(--buzz-gradient-dark-top)",
    bottom: "var(--buzz-gradient-dark-bottom)",
  },
};

export const LIGHT_PREVIEW_VARS: ThemePreviewVars = {
  "--background": "0 0% 100%",
  "--border": "0 0% 89.8%",
  "--foreground": "0 0% 9%",
  "--muted": "0 0% 96.1%",
  "--muted-foreground": "0 0% 45.1%",
  "--primary": "0 0% 9%",
  "--sidebar-background": "0 0% 98%",
  "--sidebar-foreground": "0 0% 9%",
};

export const DARK_PREVIEW_VARS: ThemePreviewVars = {
  "--background": "0 0% 3.9%",
  "--border": "0 0% 14.9%",
  "--foreground": "0 0% 98%",
  "--muted": "0 0% 14.9%",
  "--muted-foreground": "0 0% 63.9%",
  "--primary": "0 0% 98%",
  "--sidebar-background": "0 0% 0%",
  "--sidebar-foreground": "0 0% 98%",
};

function hsl(vars: ThemePreviewVars | null, key: string) {
  return `hsl(${vars?.[key] ?? LIGHT_PREVIEW_VARS[key]})`;
}

function hslAlpha(vars: ThemePreviewVars | null, key: string, alpha: number) {
  return `hsl(${vars?.[key] ?? LIGHT_PREVIEW_VARS[key]} / ${alpha})`;
}

function ThemePreviewSvg({
  vars,
  sidebarGradient,
}: {
  vars: ThemePreviewVars | null;
  sidebarGradient?: { top: string; bottom: string };
}) {
  const clipId = React.useId().replace(/:/g, "");
  const gradientId = `${clipId}-buzz`;
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
      className="h-full w-full shrink-0"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
      viewBox="0 0 118 80"
      xmlns="http://www.w3.org/2000/svg"
    >
      <g clipPath={`url(#${clipId})`}>
        <rect fill={background} height="180" rx="3.6" width="288" />
        <line stroke={border} x1="57" x2="117" y1="10.5" y2="10.5" />
        <rect
          fill={sidebarGradient ? `url(#${gradientId})` : sidebar}
          height="180"
          width="57.375"
        />
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
        {sidebarGradient ? (
          <linearGradient
            id={gradientId}
            x1="0"
            x2="0"
            y1="0"
            y2="180"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0" stopColor={sidebarGradient.top} />
            <stop offset="1" stopColor={sidebarGradient.bottom} />
          </linearGradient>
        ) : null}
      </defs>
    </svg>
  );
}

/**
 * Split preview SVG: light theme on top, dark theme on bottom.
 * Matches the "System Preference" visual — one image showing both modes.
 */
function SystemPreferencePreviewSvg({
  darkVars,
  lightVars,
  lightGradient,
  darkGradient,
}: {
  darkVars: ThemePreviewVars | null;
  lightVars: ThemePreviewVars | null;
  lightGradient?: { top: string; bottom: string };
  darkGradient?: { top: string; bottom: string };
}) {
  const clipBase = React.useId().replace(/:/g, "");
  const clipDark = `${clipBase}-dark`;
  const clipLight = `${clipBase}-light`;
  const clipOuter = `${clipBase}-outer`;
  const lightGradientId = `${clipBase}-buzz-light`;
  const darkGradientId = `${clipBase}-buzz-dark`;

  // Dark half colors
  const darkBg = hsl(darkVars, "--background");
  const darkSidebar = hsl(darkVars, "--sidebar-background");
  const darkSidebarFg = hslAlpha(darkVars, "--sidebar-foreground", 0.58);
  const darkMutedFg = hsl(darkVars, "--muted-foreground");

  // Light half colors
  const lightBg = hsl(lightVars, "--background");
  const lightBorder = hsl(lightVars, "--border");
  const lightForeground = hsl(lightVars, "--foreground");
  const lightPrimary = hsl(lightVars, "--primary");
  const lightPrimarySoft = hslAlpha(lightVars, "--primary", 0.68);
  const lightSidebar = hsl(lightVars, "--sidebar-background");
  const lightSidebarFg = hslAlpha(lightVars, "--sidebar-foreground", 0.58);
  const lightMutedFg = hsl(lightVars, "--muted-foreground");

  return (
    <svg
      aria-hidden="true"
      className="h-full w-full"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
      viewBox="0 0 118 80"
      xmlns="http://www.w3.org/2000/svg"
    >
      {/* Light half (top) */}
      <g clipPath={`url(#${clipLight})`}>
        <rect fill={lightBg} height="180" rx="3.6" width="288" />
        <rect
          fill={lightGradient ? `url(#${lightGradientId})` : lightSidebar}
          height="180"
          width="57.375"
        />
        <rect
          fill={lightSidebarFg}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="15.9751"
        />
        <rect
          fill={lightSidebarFg}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="21.375"
        />
        <rect
          fill={lightSidebarFg}
          height="3.6"
          rx="0.9"
          width="3.6"
          x="3.60156"
          y="26.7749"
        />
        <rect
          fill={lightSidebarFg}
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
          fill={lightSidebarFg}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="16.875"
        />
        <rect
          fill={lightSidebarFg}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="22.2749"
        />
        <rect
          fill={lightSidebarFg}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="27.675"
        />
        <rect
          fill={lightSidebarFg}
          height="1.8"
          rx="0.225"
          width="45.225"
          x="9"
          y="33.075"
        />
        <rect
          fill={lightMutedFg}
          height="1.8"
          rx="0.225"
          width="26.775"
          x="3.60156"
          y="43.875"
        />
        <line stroke={lightBorder} x1="57" x2="118" y1="10.5" y2="10.5" />
        <rect
          fill={lightForeground}
          height="2"
          rx="0.5"
          width="21"
          x="60"
          y="4"
        />
        <rect fill={lightPrimary} height="4" rx="1" width="4" x="105" y="3" />
        <rect
          fill={lightPrimarySoft}
          height="4"
          rx="1"
          width="4"
          x="111"
          y="3"
        />
      </g>

      {/* Dark half (bottom) — clipped to bottom portion */}
      <g clipPath={`url(#${clipDark})`}>
        <g clipPath={`url(#${clipOuter})`}>
          <rect fill={darkBg} height="180" rx="3.6" width="288" y="22" />
          <rect
            fill={darkGradient ? `url(#${darkGradientId})` : darkSidebar}
            height="180"
            width="57.375"
            y="22"
          />
          <rect
            fill={darkSidebarFg}
            height="3.6"
            rx="0.9"
            width="3.6"
            x="3.60156"
            y="37.9751"
          />
          <rect
            fill={darkSidebarFg}
            height="3.6"
            rx="0.9"
            width="3.6"
            x="3.60156"
            y="43.375"
          />
          <rect
            fill={darkSidebarFg}
            height="3.6"
            rx="0.9"
            width="3.6"
            x="3.60156"
            y="48.7749"
          />
          <rect
            fill={darkSidebarFg}
            height="3.6"
            rx="0.9"
            width="3.6"
            x="3.60156"
            y="54.175"
          />
          <rect
            fill={darkSidebarFg}
            height="1.8"
            rx="0.225"
            width="45.225"
            x="9"
            y="38.875"
          />
          <rect
            fill={darkSidebarFg}
            height="1.8"
            rx="0.225"
            width="45.225"
            x="9"
            y="44.2749"
          />
          <rect
            fill={darkSidebarFg}
            height="1.8"
            rx="0.225"
            width="45.225"
            x="9"
            y="49.675"
          />
          <rect
            fill={darkSidebarFg}
            height="1.8"
            rx="0.225"
            width="45.225"
            x="9"
            y="55.075"
          />
          <rect
            fill={darkMutedFg}
            height="1.8"
            rx="0.225"
            width="26.775"
            x="3.60156"
            y="65.875"
          />
        </g>
      </g>

      <defs>
        <clipPath id={clipLight}>
          <rect fill="white" height="180" rx="3.6" width="288" />
        </clipPath>
        <clipPath id={clipDark}>
          <path d="M0 37H118V80H0V37Z" fill="white" />
        </clipPath>
        <clipPath id={clipOuter}>
          <rect fill="white" height="180" rx="3.6" width="288" y="22" />
        </clipPath>
        {lightGradient ? (
          <linearGradient
            gradientUnits="userSpaceOnUse"
            id={lightGradientId}
            x1="0"
            x2="0"
            y1="0"
            y2="80"
          >
            <stop offset="0" stopColor={lightGradient.top} />
            <stop offset="1" stopColor={lightGradient.bottom} />
          </linearGradient>
        ) : null}
        {darkGradient ? (
          <linearGradient
            gradientUnits="userSpaceOnUse"
            id={darkGradientId}
            x1="0"
            x2="0"
            y1="22"
            y2="80"
          >
            <stop offset="0" stopColor={darkGradient.top} />
            <stop offset="1" stopColor={darkGradient.bottom} />
          </linearGradient>
        ) : null}
      </defs>
    </svg>
  );
}

export function ThemePreviewFrame({
  className,
  vars,
  sidebarGradient,
}: {
  className?: string;
  vars: ThemePreviewVars | null;
  sidebarGradient?: { top: string; bottom: string };
}) {
  return (
    <div
      className={cn(
        "relative aspect-[3/2] overflow-hidden rounded-2xl border",
        className,
      )}
      style={{
        backgroundColor: hsl(vars, "--muted"),
        borderColor: hsl(vars, "--border"),
      }}
    >
      <div className="absolute -bottom-1 -right-1 h-[90%] w-[90%]">
        <ThemePreviewSvg vars={vars} sidebarGradient={sidebarGradient} />
      </div>
    </div>
  );
}

/**
 * System preference preview frame: shows light on top, dark on bottom
 * in a single image to represent auto-switching themes.
 */
export function SystemPreferencePreviewFrame({
  className,
  darkVars,
  lightVars,
  lightGradient,
  darkGradient,
}: {
  className?: string;
  darkVars: ThemePreviewVars | null;
  lightVars: ThemePreviewVars | null;
  lightGradient?: { top: string; bottom: string };
  darkGradient?: { top: string; bottom: string };
}) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-2xl border border-border/70",
        className,
      )}
      style={{
        backgroundColor: "hsl(var(--muted))",
      }}
    >
      <div className="absolute -bottom-1 -right-1 h-[90%] w-[90%]">
        <SystemPreferencePreviewSvg
          darkGradient={darkGradient}
          darkVars={darkVars}
          lightGradient={lightGradient}
          lightVars={lightVars}
        />
      </div>
    </div>
  );
}
