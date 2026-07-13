/**
 * Theme Loader
 *
 * Loads Shiki theme JSON files and extracts key colors (bg, fg, comment, git).
 * Only imports the theme JSON — the Shiki highlighter engine is not used here.
 */

import type { ThemeRegistrationRaw } from "shiki";

/**
 * Buzz theme name. Buzz is a first-party light theme that reuses GitHub
 * Light for every base color (backgrounds, text, borders, code) — the
 * message area and containers are indistinguishable from GitHub Light. Its
 * one distinguishing feature is a branded gradient painted across the
 * sidebar/nav canvas, replacing GitHub Light's flat grey. The gradient is
 * applied by {@link ThemeProvider} toggling a `data-buzz-sidebar` attribute
 * on the document root; the CSS lives in `shared/styles/globals/theme.css`.
 */
export const BUZZ_THEME_NAME = "buzz";

/**
 * Buzz Dark theme name. The dark-mode counterpart to {@link BUZZ_THEME_NAME}:
 * reuses the GitHub Dark palette for every base color, with the same branded
 * sidebar gradient (dark-tuned colors, see `shared/styles/globals/theme.css`).
 * {@link ThemeProvider} toggles the shared `data-buzz-sidebar` attribute for
 * this theme too; the `.dark` root class selects the dark gradient values.
 *
 * Buzz and Buzz Dark are paired in {@link THEME_PAIRS}, so the picker shows a
 * combined "Buzz" tile under System mode (follow-OS) plus a single "Buzz" tile
 * under Light and a "Buzz Dark" tile under Dark.
 */
export const BUZZ_DARK_THEME_NAME = "buzz-dark";

/** The Shiki bundle Buzz borrows its base palette from. */
export const BUZZ_BASE_THEME: SyntaxThemeName = "github-light";

/** The Shiki bundle Buzz Dark borrows its base palette from. */
export const BUZZ_DARK_BASE_THEME: SyntaxThemeName = "github-dark";

/**
 * Resolve a theme name to the real Shiki bundled theme it maps to.
 *
 * Most themes map to themselves, but the Buzz aliases (`buzz` / `buzz-dark`)
 * are not bundled Shiki themes — they reuse the GitHub Light / GitHub Dark
 * palettes. The Shiki highlighter engine (used for fenced code blocks in
 * `CodeBlock.tsx`) only understands bundled names, so callers that hand a
 * theme name to `loadTheme` / `codeToTokens` must resolve it through here
 * first; passing a raw Buzz alias makes Shiki throw and code blocks fall
 * back to unhighlighted plain text.
 */
export function resolveShikiThemeName(name: string): SyntaxThemeName {
  if (name === BUZZ_THEME_NAME) return BUZZ_BASE_THEME;
  if (name === BUZZ_DARK_THEME_NAME) return BUZZ_DARK_BASE_THEME;
  return name as SyntaxThemeName;
}

// Available themes. "buzz" is a Buzz-branded theme that reuses the
// github-light palette plus a sidebar gradient; the rest are the Shiki
// bundled syntax themes, alphabetically sorted.
export const SYNTAX_THEMES = [
  "buzz",
  "buzz-dark",
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

const ONBOARDING_THEME_FALLBACK: SyntaxThemeName = "github-light-default";
const ONBOARDING_THEME_PREFERENCES = [
  "neutral",
  ONBOARDING_THEME_FALLBACK,
] as const;

export const ONBOARDING_DEFAULT_THEME_NAME = (ONBOARDING_THEME_PREFERENCES.find(
  (name) => (SYNTAX_THEMES as readonly string[]).includes(name),
) ?? ONBOARDING_THEME_FALLBACK) as SyntaxThemeName;

// Known light themes — used by the theme picker to show sun/moon icons
// for themes that haven't been loaded yet.
export const LIGHT_THEMES: ReadonlySet<SyntaxThemeName> = new Set([
  "buzz",
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
  // Buzz reuses the github-light palette; its gradient is applied separately.
  buzz: () => import("shiki/themes/github-light.mjs"),
  // Buzz Dark reuses the github-dark palette; dark gradient applied separately.
  "buzz-dark": () => import("shiki/themes/github-dark.mjs"),
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
    // Buzz is the first-party pair; keep it first so it leads every category.
    ["buzz", "buzz-dark"],
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
    ["buzz-dark", "buzz"],
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
}

export function extractThemeInfo(
  themeName: string,
  theme: ThemeRegistrationRaw,
): ThemeInfo {
  const bg =
    (theme.colors?.["editor.background"] as string | undefined) || "#1e1e1e";
  const fg =
    (theme.colors?.["editor.foreground"] as string | undefined) || "#d4d4d4";
  const gitColors = extractGitColors(
    theme.colors as Record<string, string> | undefined,
  );
  return {
    name: themeName,
    bg,
    fg,
    comment: extractCommentColor(
      theme.settings as ReadonlyArray<ThemeSetting> | undefined,
      fg,
    ),
    ...gitColors,
  };
}

export async function loadThemeData(
  name: SyntaxThemeName,
): Promise<ThemeRegistrationRaw> {
  const loader = themeImports[name];
  const { default: theme } = await loader();
  return theme;
}
