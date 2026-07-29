/**
 * App chrome "shell style" presets for Buzz Desktop.
 *
 * These override shell CSS variables (sidebar / primary / border / frame) while
 * keeping Shiki syntax themes independent.
 *
 * How to switch:
 * 1. Open Settings → Appearance
 * 2. Use a Buzz theme (github-light/dark pair labeled Buzz)
 * 3. Pick a shell style (Raft, Persona 5, Persona 5 Max, …)
 * 4. Choice is stored in localStorage under `buzz-shell-style`
 *
 * Theme colors target chrome (outer frame, primary, sidebar), not full-page
 * flood fills. Persona 5 Max adds extra chrome CSS via `data-shell-style`.
 */

import { getRaftShellVars } from "./raft-shell";

export type ShellStyleId =
  | "raft"
  | "persona5"
  | "persona5max"
  | "brutstack"
  | "limepunch"
  | "coralink"
  | "rosebrick"
  | "tealblock"
  | "violetpaper"
  | "skypost"
  | "inkmono";

export type ShellStyleVars = Record<string, string>;

export type ShellStyleDef = {
  id: ShellStyleId;
  label: string;
  blurb: string;
  /** Swatch hex for picker UI */
  swatch: string;
  /** Preferred accent hex when this shell is active */
  accentHex: string;
  /** Whether chrome should force dark class for this shell */
  forceDark?: boolean;
  getVars: (isDark: boolean) => ShellStyleVars;
};

function shell(
  primary: string,
  primaryFg: string,
  bg: string,
  fg: string,
  card: string,
  sidebar: string,
  sidebarFg: string,
  border: string,
  muted: string,
  mutedFg: string,
  accent: string,
  accentFg: string,
): ShellStyleVars {
  return {
    "--radius": "0rem",
    "--background": bg,
    "--foreground": fg,
    "--card": card,
    "--card-foreground": fg,
    "--popover": card,
    "--popover-foreground": fg,
    "--primary": primary,
    "--primary-foreground": primaryFg,
    "--secondary": muted,
    "--secondary-foreground": fg,
    "--muted": muted,
    "--muted-foreground": mutedFg,
    "--accent": accent,
    "--accent-foreground": accentFg,
    "--destructive": "347 77% 50%",
    "--destructive-foreground": "0 0% 100%",
    "--border": border,
    "--input": border,
    "--ring": primary,
    "--sidebar": sidebar,
    "--sidebar-background": sidebar,
    "--sidebar-foreground": sidebarFg,
    "--sidebar-primary": primary,
    "--sidebar-primary-foreground": primaryFg,
    "--sidebar-active": primary,
    "--sidebar-active-foreground": primaryFg,
    "--sidebar-accent": muted,
    "--sidebar-accent-foreground": sidebarFg,
    "--sidebar-border": border,
    "--sidebar-ring": primary,
  };
}

/** Persona 5 — red / black / white */
const P5 = shell(
  "355 100% 45%", // #e60012-ish
  "0 0% 100%",
  "0 0% 4%",
  "0 0% 96%",
  "0 0% 7%",
  "0 0% 0%",
  "0 0% 96%",
  "0 0% 96%",
  "0 0% 10%",
  "0 0% 64%",
  "355 100% 45%",
  "0 0% 100%",
);

/** Persona 5 Max — hotter red */
const P5_MAX = shell(
  "348 100% 50%", // #ff0033
  "0 0% 100%",
  "0 0% 0%",
  "0 0% 100%",
  "0 0% 4%",
  "0 0% 0%",
  "0 0% 100%",
  "0 0% 100%",
  "0 0% 7%",
  "0 0% 80%",
  "348 100% 50%",
  "0 0% 100%",
);

const BRUTSTACK = shell(
  "217 91% 60%",
  "0 0% 100%",
  "226 100% 97%",
  "222 47% 11%",
  "0 0% 100%",
  "217 33% 17%",
  "210 40% 98%",
  "222 47% 11%",
  "214 32% 91%",
  "215 16% 47%",
  "214 95% 93%",
  "222 47% 11%",
);

const LIME = shell(
  "84 81% 44%",
  "142 76% 20%",
  "80 89% 95%",
  "142 76% 20%",
  "0 0% 100%",
  "142 76% 20%",
  "80 89% 95%",
  "142 76% 20%",
  "80 89% 90%",
  "85 50% 25%",
  "80 89% 80%",
  "142 76% 20%",
);

const CORAL = shell(
  "25 95% 53%",
  "0 0% 100%",
  "33 100% 96%",
  "15 75% 28%",
  "0 0% 100%",
  "15 75% 32%",
  "33 100% 96%",
  "15 75% 28%",
  "33 100% 90%",
  "15 60% 35%",
  "30 100% 85%",
  "15 75% 28%",
);

const ROSE = shell(
  "347 89% 60%",
  "0 0% 100%",
  "355 100% 97%",
  "340 70% 30%",
  "0 0% 100%",
  "340 70% 35%",
  "355 100% 97%",
  "340 70% 30%",
  "355 100% 92%",
  "340 50% 40%",
  "350 100% 90%",
  "340 70% 30%",
);

const TEAL = shell(
  "173 80% 40%",
  "175 84% 10%",
  "166 76% 97%",
  "175 70% 20%",
  "0 0% 100%",
  "175 70% 22%",
  "166 76% 97%",
  "175 70% 20%",
  "166 70% 90%",
  "175 50% 30%",
  "166 70% 80%",
  "175 70% 20%",
);

const VIOLET = shell(
  "258 90% 66%",
  "0 0% 100%",
  "250 100% 98%",
  "263 70% 35%",
  "0 0% 100%",
  "263 70% 40%",
  "250 100% 98%",
  "263 70% 35%",
  "250 100% 94%",
  "263 50% 40%",
  "250 100% 90%",
  "263 70% 35%",
);

const SKY = shell(
  "199 89% 48%",
  "0 0% 100%",
  "204 100% 97%",
  "201 90% 24%",
  "0 0% 100%",
  "201 90% 28%",
  "204 100% 97%",
  "201 90% 24%",
  "204 94% 94%",
  "201 70% 30%",
  "204 94% 86%",
  "201 90% 24%",
);

const INK = shell(
  "0 0% 4%",
  "0 0% 98%",
  "0 0% 98%",
  "0 0% 4%",
  "0 0% 100%",
  "0 0% 4%",
  "0 0% 98%",
  "0 0% 4%",
  "0 0% 96%",
  "0 0% 32%",
  "0 0% 90%",
  "0 0% 4%",
);

export const SHELL_STYLES: ShellStyleDef[] = [
  {
    id: "raft",
    label: "Raft",
    blurb: "奶油 + 琥珀黄（home_portal）",
    swatch: "#fbbf24",
    accentHex: "#fbbf24",
    // Delegate to raft-shell.ts so light/dark stay the single source of truth.
    getVars: (dark) => getRaftShellVars(dark),
  },
  {
    id: "persona5",
    label: "Persona 5",
    blurb: "红黑白 · 锐角",
    swatch: "#e60012",
    accentHex: "#e60012",
    forceDark: true,
    getVars: () => ({ ...P5 }),
  },
  {
    id: "persona5max",
    label: "Persona 5 Max",
    blurb: "全红 · 漫画分镜感",
    swatch: "#ff0033",
    accentHex: "#ff0033",
    forceDark: true,
    getVars: () => ({ ...P5_MAX }),
  },
  {
    id: "brutstack",
    label: "Brutstack",
    blurb: "冷灰 + 电蓝",
    swatch: "#3b82f6",
    accentHex: "#3b82f6",
    getVars: (dark) =>
      dark
        ? shell(
            "217 91% 60%",
            "0 0% 100%",
            "222 47% 11%",
            "210 40% 98%",
            "217 33% 17%",
            "222 47% 8%",
            "210 40% 98%",
            "217 33% 25%",
            "217 33% 20%",
            "215 20% 65%",
            "217 33% 22%",
            "210 40% 98%",
          )
        : { ...BRUTSTACK },
  },
  {
    id: "limepunch",
    label: "LimePunch",
    blurb: "纸白 + 酸绿",
    swatch: "#84cc16",
    accentHex: "#84cc16",
    getVars: () => ({ ...LIME }),
  },
  {
    id: "coralink",
    label: "CoralInk",
    blurb: "浅桃 + 珊瑚橙",
    swatch: "#f97316",
    accentHex: "#f97316",
    getVars: () => ({ ...CORAL }),
  },
  {
    id: "rosebrick",
    label: "RoseBrick",
    blurb: "米白 + 玫红",
    swatch: "#f43f5e",
    accentHex: "#f43f5e",
    getVars: () => ({ ...ROSE }),
  },
  {
    id: "tealblock",
    label: "TealBlock",
    blurb: "薄荷 + 青绿",
    swatch: "#14b8a6",
    accentHex: "#14b8a6",
    getVars: () => ({ ...TEAL }),
  },
  {
    id: "violetpaper",
    label: "VioletPaper",
    blurb: "浅紫 + 紫罗兰",
    swatch: "#8b5cf6",
    accentHex: "#8b5cf6",
    getVars: () => ({ ...VIOLET }),
  },
  {
    id: "skypost",
    label: "SkyPost",
    blurb: "天空纸 + 天蓝",
    swatch: "#0ea5e9",
    accentHex: "#0ea5e9",
    getVars: () => ({ ...SKY }),
  },
  {
    id: "inkmono",
    label: "InkMono",
    blurb: "高对比黑白",
    swatch: "#0a0a0a",
    accentHex: "#0a0a0a",
    getVars: (dark) =>
      dark
        ? shell(
            "0 0% 98%",
            "0 0% 4%",
            "0 0% 4%",
            "0 0% 98%",
            "0 0% 8%",
            "0 0% 0%",
            "0 0% 98%",
            "0 0% 30%",
            "0 0% 12%",
            "0 0% 64%",
            "0 0% 16%",
            "0 0% 98%",
          )
        : { ...INK },
  },
];

export const DEFAULT_SHELL_STYLE: ShellStyleId = "raft";
export const SHELL_STYLE_STORAGE_KEY = "buzz-shell-style";

export function isShellStyleId(value: string): value is ShellStyleId {
  return SHELL_STYLES.some((s) => s.id === value);
}

export function getShellStyle(id: ShellStyleId): ShellStyleDef {
  return SHELL_STYLES.find((s) => s.id === id) ?? SHELL_STYLES[0];
}

export function getShellStyleVars(
  id: ShellStyleId,
  isDark: boolean,
): ShellStyleVars {
  return getShellStyle(id).getVars(isDark);
}
