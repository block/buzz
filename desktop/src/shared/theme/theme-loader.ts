/**
 * Theme Loader
 *
 * Loads Shiki theme JSON files and extracts key colors (bg, fg, comment, git).
 * Only imports the theme JSON — the Shiki highlighter engine is not used here.
 */

import type { ThemeRegistrationRaw } from "shiki";
import {
  type TerminalPalette,
  extractTerminalPalette,
} from "./terminal-palette";

/**
 * Legacy storage name for the first-party Zorro light theme. Its application
 * surfaces use the Zorro red-orange palette while syntax highlighting reuses GitHub
 * Light. A branded gradient is painted across the sidebar/nav canvas. It is
 * applied by {@link ThemeProvider} toggling a `data-buzz-sidebar` attribute
 * on the document root; the CSS lives in `shared/styles/globals/theme.css`.
 */
export const ZORRO_THEME_NAME = "zorro";

/**
 * Legacy storage name for the Zorro dark theme, the counterpart to
 * {@link ZORRO_THEME_NAME}. It uses the dark end of the Zorro palette with a
 * matching sidebar gradient; syntax highlighting reuses GitHub Dark.
 * {@link ThemeProvider} toggles the shared `data-buzz-sidebar` attribute for
 * this theme too; the `.dark` root class selects the dark gradient values.
 *
 * Zorro and Zorro Dark are paired in {@link THEME_PAIRS}, so the picker shows a
 * combined "Zorro" tile under System mode (follow-OS) plus a single "Zorro"
 * tile under Light and a "Zorro Dark" tile under Dark.
 */
export const ZORRO_DARK_THEME_NAME = "zorro-dark";

/** The Shiki bundle Zorro Light borrows for syntax highlighting only. */
export const ZORRO_BASE_THEME: SyntaxThemeName = "github-light";

/** The Shiki bundle Zorro Dark borrows for syntax highlighting only. */
export const ZORRO_DARK_BASE_THEME: SyntaxThemeName = "github-dark";

/**
 * Resolve a theme name to the real Shiki bundled theme it maps to.
 *
 * Most themes map to themselves, but the Zorro aliases (`zorro` / `zorro-dark`)
 * are not bundled Shiki themes — they reuse the GitHub Light / GitHub Dark
 * palettes. The Shiki highlighter engine (used for fenced code blocks in
 * `CodeBlock.tsx`) only understands bundled names, so callers that hand a
 * theme name to `loadTheme` / `codeToTokens` must resolve it through here
 * first; passing a raw Zorro alias makes Shiki throw and code blocks fall
 * back to unhighlighted plain text.
 */
export function resolveShikiThemeName(name: string): SyntaxThemeName {
  if (name === ZORRO_THEME_NAME) return ZORRO_BASE_THEME;
  if (name === ZORRO_DARK_THEME_NAME) return ZORRO_DARK_BASE_THEME;
  return name as SyntaxThemeName;
}

// Zorro's first-party palette resolves code highlighting through GitHub's
// Shiki themes while keeping application surfaces on the product palette.
export const SYNTAX_THEMES = [
  "zorro",
  "zorro-dark",
  "andromeeda",
  "aurora-x",
  "ayu-dark",
  "catppuccin-frappe",
  "catppuccin-latte",
  "catppuccin-macchiato",
  "catppuccin-mocha",
  "dark-plus",
  "dracula",
  "dracula-soft",
  "everforest-dark",
  "everforest-light",
  "github-dark",
  "github-dark-default",
  "github-dark-dimmed",
  "github-dark-high-contrast",
  "github-light",
  "github-light-default",
  "github-light-high-contrast",
  "gruvbox-dark-hard",
  "gruvbox-dark-medium",
  "gruvbox-dark-soft",
  "gruvbox-light-hard",
  "gruvbox-light-medium",
  "gruvbox-light-soft",
  "houston",
  "kanagawa-dragon",
  "kanagawa-lotus",
  "kanagawa-wave",
  "laserwave",
  "light-plus",
  "material-theme",
  "material-theme-darker",
  "material-theme-lighter",
  "material-theme-ocean",
  "material-theme-palenight",
  "min-dark",
  "min-light",
  "monokai",
  "night-owl",
  "nord",
  "one-dark-pro",
  "one-light",
  "plastic",
  "poimandres",
  "red",
  "rose-pine",
  "rose-pine-dawn",
  "rose-pine-moon",
  "slack-dark",
  "slack-ochin",
  "snazzy-light",
  "solarized-dark",
  "solarized-light",
  "synthwave-84",
  "tokyo-night",
  "vesper",
  "vitesse-black",
  "vitesse-dark",
  "vitesse-light",
] as const;

export type SyntaxThemeName = (typeof SYNTAX_THEMES)[number];

// Known light themes — used by the theme picker to show sun/moon icons
// for themes that haven't been loaded yet.
export const LIGHT_THEMES: ReadonlySet<SyntaxThemeName> = new Set([
  "zorro",
  "catppuccin-latte",
  "everforest-light",
  "github-light",
  "github-light-default",
  "github-light-high-contrast",
  "gruvbox-light-hard",
  "gruvbox-light-medium",
  "gruvbox-light-soft",
  "kanagawa-lotus",
  "light-plus",
  "material-theme-lighter",
  "min-light",
  "one-light",
  "rose-pine-dawn",
  "slack-ochin",
  "snazzy-light",
  "solarized-light",
  "vitesse-light",
]);

// Static theme imports (Vite needs static strings for tree-shaking)
const themeImports: Record<
  SyntaxThemeName,
  () => Promise<{ default: ThemeRegistrationRaw }>
> = {
  // Zorro reuses the github-light palette; its gradient is applied separately.
  zorro: () => import("shiki/themes/github-light.mjs"),
  // Zorro Dark reuses the github-dark palette; dark gradient applied separately.
  "zorro-dark": () => import("shiki/themes/github-dark.mjs"),
  andromeeda: () => import("shiki/themes/andromeeda.mjs"),
  "aurora-x": () => import("shiki/themes/aurora-x.mjs"),
  "ayu-dark": () => import("shiki/themes/ayu-dark.mjs"),
  "catppuccin-frappe": () => import("shiki/themes/catppuccin-frappe.mjs"),
  "catppuccin-latte": () => import("shiki/themes/catppuccin-latte.mjs"),
  "catppuccin-macchiato": () => import("shiki/themes/catppuccin-macchiato.mjs"),
  "catppuccin-mocha": () => import("shiki/themes/catppuccin-mocha.mjs"),
  "dark-plus": () => import("shiki/themes/dark-plus.mjs"),
  dracula: () => import("shiki/themes/dracula.mjs"),
  "dracula-soft": () => import("shiki/themes/dracula-soft.mjs"),
  "everforest-dark": () => import("shiki/themes/everforest-dark.mjs"),
  "everforest-light": () => import("shiki/themes/everforest-light.mjs"),
  "github-dark": () => import("shiki/themes/github-dark.mjs"),
  "github-dark-default": () => import("shiki/themes/github-dark-default.mjs"),
  "github-dark-dimmed": () => import("shiki/themes/github-dark-dimmed.mjs"),
  "github-dark-high-contrast": () =>
    import("shiki/themes/github-dark-high-contrast.mjs"),
  "github-light": () => import("shiki/themes/github-light.mjs"),
  "github-light-default": () => import("shiki/themes/github-light-default.mjs"),
  "github-light-high-contrast": () =>
    import("shiki/themes/github-light-high-contrast.mjs"),
  "gruvbox-dark-hard": () => import("shiki/themes/gruvbox-dark-hard.mjs"),
  "gruvbox-dark-medium": () => import("shiki/themes/gruvbox-dark-medium.mjs"),
  "gruvbox-dark-soft": () => import("shiki/themes/gruvbox-dark-soft.mjs"),
  "gruvbox-light-hard": () => import("shiki/themes/gruvbox-light-hard.mjs"),
  "gruvbox-light-medium": () => import("shiki/themes/gruvbox-light-medium.mjs"),
  "gruvbox-light-soft": () => import("shiki/themes/gruvbox-light-soft.mjs"),
  houston: () => import("shiki/themes/houston.mjs"),
  "kanagawa-dragon": () => import("shiki/themes/kanagawa-dragon.mjs"),
  "kanagawa-lotus": () => import("shiki/themes/kanagawa-lotus.mjs"),
  "kanagawa-wave": () => import("shiki/themes/kanagawa-wave.mjs"),
  laserwave: () => import("shiki/themes/laserwave.mjs"),
  "light-plus": () => import("shiki/themes/light-plus.mjs"),
  "material-theme": () => import("shiki/themes/material-theme.mjs"),
  "material-theme-darker": () =>
    import("shiki/themes/material-theme-darker.mjs"),
  "material-theme-lighter": () =>
    import("shiki/themes/material-theme-lighter.mjs"),
  "material-theme-ocean": () => import("shiki/themes/material-theme-ocean.mjs"),
  "material-theme-palenight": () =>
    import("shiki/themes/material-theme-palenight.mjs"),
  "min-dark": () => import("shiki/themes/min-dark.mjs"),
  "min-light": () => import("shiki/themes/min-light.mjs"),
  monokai: () => import("shiki/themes/monokai.mjs"),
  "night-owl": () => import("shiki/themes/night-owl.mjs"),
  nord: () => import("shiki/themes/nord.mjs"),
  "one-dark-pro": () => import("shiki/themes/one-dark-pro.mjs"),
  "one-light": () => import("shiki/themes/one-light.mjs"),
  plastic: () => import("shiki/themes/plastic.mjs"),
  poimandres: () => import("shiki/themes/poimandres.mjs"),
  red: () => import("shiki/themes/red.mjs"),
  "rose-pine": () => import("shiki/themes/rose-pine.mjs"),
  "rose-pine-dawn": () => import("shiki/themes/rose-pine-dawn.mjs"),
  "rose-pine-moon": () => import("shiki/themes/rose-pine-moon.mjs"),
  "slack-dark": () => import("shiki/themes/slack-dark.mjs"),
  "slack-ochin": () => import("shiki/themes/slack-ochin.mjs"),
  "snazzy-light": () => import("shiki/themes/snazzy-light.mjs"),
  "solarized-dark": () => import("shiki/themes/solarized-dark.mjs"),
  "solarized-light": () => import("shiki/themes/solarized-light.mjs"),
  "synthwave-84": () => import("shiki/themes/synthwave-84.mjs"),
  "tokyo-night": () => import("shiki/themes/tokyo-night.mjs"),
  vesper: () => import("shiki/themes/vesper.mjs"),
  "vitesse-black": () => import("shiki/themes/vitesse-black.mjs"),
  "vitesse-dark": () => import("shiki/themes/vitesse-dark.mjs"),
  "vitesse-light": () => import("shiki/themes/vitesse-light.mjs"),
};

export function isLightTheme(name: string): boolean {
  return LIGHT_THEMES.has(name as SyntaxThemeName);
}

/**
 * Theme pairs: maps a light theme to its dark counterpart and vice versa.
 * Used by the "Follow system" feature to auto-switch themes.
 */
export const THEME_PAIRS: ReadonlyMap<SyntaxThemeName, SyntaxThemeName> =
  new Map([
    // Light → Dark
    // Zorro is the first-party pair; keep it first so it leads every category.
    ["zorro", "zorro-dark"],
    ["catppuccin-latte", "catppuccin-mocha"],
    ["everforest-light", "everforest-dark"],
    ["github-light", "github-dark"],
    ["github-light-default", "github-dark-default"],
    ["github-light-high-contrast", "github-dark-high-contrast"],
    ["gruvbox-light-hard", "gruvbox-dark-hard"],
    ["gruvbox-light-medium", "gruvbox-dark-medium"],
    ["gruvbox-light-soft", "gruvbox-dark-soft"],
    ["kanagawa-lotus", "kanagawa-wave"],
    ["light-plus", "dark-plus"],
    ["material-theme-lighter", "material-theme"],
    ["min-light", "min-dark"],
    ["one-light", "one-dark-pro"],
    ["rose-pine-dawn", "rose-pine"],
    ["slack-ochin", "slack-dark"],
    ["solarized-light", "solarized-dark"],
    ["vitesse-light", "vitesse-dark"],
    // Dark → Light (reverse mappings)
    ["zorro-dark", "zorro"],
    ["catppuccin-mocha", "catppuccin-latte"],
    ["everforest-dark", "everforest-light"],
    ["github-dark", "github-light"],
    ["github-dark-default", "github-light-default"],
    ["github-dark-high-contrast", "github-light-high-contrast"],
    ["gruvbox-dark-hard", "gruvbox-light-hard"],
    ["gruvbox-dark-medium", "gruvbox-light-medium"],
    ["gruvbox-dark-soft", "gruvbox-light-soft"],
    ["kanagawa-wave", "kanagawa-lotus"],
    ["dark-plus", "light-plus"],
    ["material-theme", "material-theme-lighter"],
    ["min-dark", "min-light"],
    ["one-dark-pro", "one-light"],
    ["rose-pine", "rose-pine-dawn"],
    ["slack-dark", "slack-ochin"],
    ["solarized-dark", "solarized-light"],
    ["vitesse-dark", "vitesse-light"],
  ]);

/**
 * Get the counterpart theme for system theme switching.
 * Returns the paired theme if one exists, or null if the theme has no pair.
 */
export function getThemePair(name: SyntaxThemeName): SyntaxThemeName | null {
  return THEME_PAIRS.get(name) ?? null;
}

/**
 * Given a user-selected theme and the current system color scheme,
 * returns the theme that should actually be applied.
 */
export function resolveSystemTheme(
  selectedTheme: SyntaxThemeName,
  systemIsDark: boolean,
): SyntaxThemeName {
  const selectedIsLight = isLightTheme(selectedTheme);
  const needsSwitch =
    (systemIsDark && selectedIsLight) || (!systemIsDark && !selectedIsLight);

  if (!needsSwitch) return selectedTheme;

  const pair = getThemePair(selectedTheme);
  return pair ?? selectedTheme;
}

// Theme settings type from Shiki
interface ThemeSetting {
  scope?: string | string[];
  settings?: { foreground?: string };
}

function extractCommentColor(
  settings: ReadonlyArray<ThemeSetting> | undefined,
  fallback: string,
): string {
  if (!settings) return fallback;

  for (const setting of settings) {
    if (!setting.scope || !setting.settings?.foreground) continue;
    const scopes = Array.isArray(setting.scope)
      ? setting.scope
      : [setting.scope];
    if (scopes.includes("comment")) {
      return setting.settings.foreground;
    }
  }

  return fallback;
}

function stripAlpha(color: string): string {
  if (color.length === 9 && color.startsWith("#")) {
    return color.slice(0, 7);
  }
  return color;
}

function extractGitColors(colors: Record<string, string> | undefined): {
  added: string | null;
  deleted: string | null;
  modified: string | null;
} {
  if (!colors) {
    return { added: null, deleted: null, modified: null };
  }

  const addedKeys = [
    "gitDecoration.addedResourceForeground",
    "editorGutter.addedBackground",
    "diffEditor.insertedTextBackground",
  ];
  const deletedKeys = [
    "gitDecoration.deletedResourceForeground",
    "editorGutter.deletedBackground",
    "diffEditor.removedTextBackground",
  ];
  const modifiedKeys = [
    "gitDecoration.modifiedResourceForeground",
    "editorGutter.modifiedBackground",
  ];

  const findColor = (keys: string[]): string | null => {
    for (const key of keys) {
      const value = colors[key];
      if (value) return stripAlpha(value);
    }
    return null;
  };

  return {
    added: findColor(addedKeys),
    deleted: findColor(deletedKeys),
    modified: findColor(modifiedKeys),
  };
}

export interface ThemeInfo {
  name: string;
  bg: string;
  fg: string;
  comment: string;
  added: string | null;
  deleted: string | null;
  modified: string | null;
  terminalPalette: TerminalPalette;
}

export function extractThemeInfo(
  themeName: string,
  theme: ThemeRegistrationRaw,
): ThemeInfo {
  const zorroLight = themeName === ZORRO_THEME_NAME;
  const zorroDark = themeName === ZORRO_DARK_THEME_NAME;
  const bg = zorroLight
    ? "#fff8ed"
    : zorroDark
      ? "#320002"
      : (theme.colors?.["editor.background"] as string | undefined) ||
        "#1e1e1e";
  const fg = zorroLight
    ? "#7a0103"
    : zorroDark
      ? "#ffffff"
      : (theme.colors?.["editor.foreground"] as string | undefined) ||
        "#d4d4d4";
  const gitColors = extractGitColors(
    theme.colors as Record<string, string> | undefined,
  );
  return {
    name: themeName,
    bg,
    fg,
    comment: zorroLight
      ? "#a50104"
      : zorroDark
        ? "#d4d4d4"
        : extractCommentColor(
            theme.settings as ReadonlyArray<ThemeSetting> | undefined,
            fg,
          ),
    ...(zorroLight
      ? {
          added: "#ff931f",
          deleted: "#b81702",
          modified: "#ec3f13",
        }
      : zorroDark
        ? {
            added: "#ffad33",
            deleted: "#ff7e33",
            modified: "#fa5e1f",
          }
        : gitColors),
    terminalPalette: extractTerminalPalette(theme),
  };
}

export async function loadThemeData(
  name: SyntaxThemeName,
): Promise<ThemeRegistrationRaw> {
  const loader = themeImports[name];
  const { default: theme } = await loader();
  return theme;
}
