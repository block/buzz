import * as React from "react";
import { cn } from "@/shared/lib/cn";

export type ThemePreviewVars = Record<string, string>;

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
      className="h-full w-full shrink-0"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
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

/**
 * Split preview SVG: dark theme on top, light theme on bottom.
 * Matches the "System Preference" visual — one image showing both modes.
 */
function SystemPreferencePreviewSvg({
  darkVars,
  lightVars,
}: {
  darkVars: ThemePreviewVars | null;
  lightVars: ThemePreviewVars | null;
}) {
  const clipBase = React.useId().replace(/:/g, "");
  const clipDark = `${clipBase}-dark`;
  const clipLight = `${clipBase}-light`;
  const clipOuter = `${clipBase}-outer`;

  // Dark half colors
  const darkBg = hsl(darkVars, "--background");
  const darkSidebar = hsl(darkVars, "--sidebar-background");
  const darkSidebarFg = hslAlpha(darkVars, "--sidebar-foreground", 0.58);
  const darkMutedFg = hsl(darkVars, "--muted-foreground");

  // Light half colors
  const lightBg = hsl(lightVars, "--background");
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
        <rect fill={lightSidebar} height="180" width="57.375" />
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
          x="4.5"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="4.55625"
          y="4.7811"
        />
        <rect
          fill="#FEBC2E"
          height="2.7"
          rx="1.35"
          width="2.7"
          x="9"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="9.05625"
          y="4.7811"
        />
        <rect
          fill="#28C840"
          height="2.7"
          rx="1.35"
          width="2.7"
          x="13.5"
          y="4.72485"
        />
        <rect
          height="2.5875"
          rx="1.29375"
          stroke="black"
          strokeOpacity="0.2"
          strokeWidth="0.1125"
          width="2.5875"
          x="13.5563"
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
      </g>

      {/* Dark half (bottom) — clipped to bottom portion */}
      <g clipPath={`url(#${clipDark})`}>
        <g clipPath={`url(#${clipOuter})`}>
          <rect fill={darkBg} height="180" rx="3.6" width="288" y="22" />
          <rect fill={darkSidebar} height="180" width="57.375" y="22" />
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
      </defs>
    </svg>
  );
}

export function ThemePreviewFrame({
  className,
  vars,
}: {
  className?: string;
  vars: ThemePreviewVars | null;
}) {
  return (
    <div
      className={cn("relative overflow-hidden rounded-2xl border", className)}
      style={{
        backgroundColor: hsl(vars, "--muted"),
        borderColor: hsl(vars, "--border"),
      }}
    >
      <div className="absolute inset-0">
        <ThemePreviewSvg vars={vars} />
      </div>
    </div>
  );
}

/**
 * System preference preview frame: shows dark on top, light on bottom
 * in a single image to represent auto-switching themes.
 */
export function SystemPreferencePreviewFrame({
  className,
  darkVars,
  lightVars,
}: {
  className?: string;
  darkVars: ThemePreviewVars | null;
  lightVars: ThemePreviewVars | null;
}) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-2xl border border-border/70",
        className,
      )}
    >
      <SystemPreferencePreviewSvg darkVars={darkVars} lightVars={lightVars} />
    </div>
  );
}
