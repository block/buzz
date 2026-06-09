/**
 * Theme Loader
 *
 * Loads Shiki theme JSON files and extracts key colors (bg, fg, comment, git).
 * Only imports the theme JSON — the Shiki highlighter engine is not used here.
 */

import type { ThemeRegistrationRaw } from "shiki";

export const LOCAL_THEME_NAMES = ["neutral"] as const;
export type LocalThemeName = (typeof LOCAL_THEME_NAMES)[number];

type LocalThemeVariant = {
  vars: Record<string, string>;
  syntaxTheme: SyntaxThemeName;
};

type LocalThemeDefinition = {
  light: LocalThemeVariant;
  dark: LocalThemeVariant;
};

// Available syntax themes (all Shiki bundled themes, alphabetically sorted)
export const SYNTAX_THEMES = [
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
export type AppThemeName = SyntaxThemeName | LocalThemeName;

export const APP_THEMES = [...LOCAL_THEME_NAMES, ...SYNTAX_THEMES] as const;

// Known light themes — used by the theme picker to show sun/moon icons
// for themes that haven't been loaded yet.
export const LIGHT_THEMES: ReadonlySet<SyntaxThemeName> = new Set([
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

const localThemes: Record<LocalThemeName, LocalThemeDefinition> = {
  neutral: {
    light: {
      syntaxTheme: "github-light-default",
      vars: {
        "--radius": "0.625rem",
        "--background": "0 0% 100%",
        "--foreground": "0 0% 3.9%",
        "--card": "0 0% 100%",
        "--card-foreground": "0 0% 3.9%",
        "--popover": "0 0% 100%",
        "--popover-foreground": "0 0% 3.9%",
        "--primary": "0 0% 9%",
        "--primary-foreground": "0 0% 98%",
        "--secondary": "0 0% 96.1%",
        "--secondary-foreground": "0 0% 9%",
        "--muted": "0 0% 96.1%",
        "--muted-foreground": "0 0% 45.1%",
        "--accent": "0 0% 96.1%",
        "--accent-foreground": "0 0% 9%",
        "--destructive": "0 84.2% 60.2%",
        "--destructive-foreground": "0 0% 98%",
        "--border": "0 0% 89.8%",
        "--input": "0 0% 89.8%",
        "--ring": "0 0% 63.9%",
        "--chart-1": "12 76% 61%",
        "--chart-2": "173 58% 39%",
        "--chart-3": "197 37% 24%",
        "--chart-4": "43 74% 66%",
        "--chart-5": "27 87% 67%",
        "--sidebar": "0 0% 98%",
        "--sidebar-background": "0 0% 98%",
        "--sidebar-foreground": "0 0% 3.9%",
        "--sidebar-primary": "0 0% 9%",
        "--sidebar-primary-foreground": "0 0% 98%",
        "--sidebar-accent": "0 0% 96.1%",
        "--sidebar-accent-foreground": "0 0% 9%",
        "--sidebar-border": "0 0% 89.8%",
        "--sidebar-ring": "0 0% 63.9%",
        "--status-added": "#16a34a",
        "--status-deleted": "#dc2626",
        "--status-modified": "#ca8a04",
        "--ui-warning": "#ca8a04",
        "--ui-warning-bg": "rgba(202, 138, 4, 0.08)",
      },
    },
    dark: {
      syntaxTheme: "github-dark-default",
      vars: {
        "--radius": "0.625rem",
        "--background": "0 0% 3.9%",
        "--foreground": "0 0% 98%",
        "--card": "0 0% 9%",
        "--card-foreground": "0 0% 98%",
        "--popover": "0 0% 9%",
        "--popover-foreground": "0 0% 98%",
        "--primary": "0 0% 89.8%",
        "--primary-foreground": "0 0% 9%",
        "--secondary": "0 0% 14.9%",
        "--secondary-foreground": "0 0% 98%",
        "--muted": "0 0% 14.9%",
        "--muted-foreground": "0 0% 63.9%",
        "--accent": "0 0% 14.9%",
        "--accent-foreground": "0 0% 98%",
        "--destructive": "0 62.8% 30.6%",
        "--destructive-foreground": "0 0% 98%",
        "--border": "0 0% 14.9%",
        "--input": "0 0% 14.9%",
        "--ring": "0 0% 45.1%",
        "--chart-1": "220 70% 50%",
        "--chart-2": "160 60% 45%",
        "--chart-3": "30 80% 55%",
        "--chart-4": "280 65% 60%",
        "--chart-5": "340 75% 55%",
        "--sidebar": "0 0% 9%",
        "--sidebar-background": "0 0% 9%",
        "--sidebar-foreground": "0 0% 98%",
        "--sidebar-primary": "0 0% 89.8%",
        "--sidebar-primary-foreground": "0 0% 9%",
        "--sidebar-accent": "0 0% 14.9%",
        "--sidebar-accent-foreground": "0 0% 98%",
        "--sidebar-border": "0 0% 14.9%",
        "--sidebar-ring": "0 0% 45.1%",
        "--status-added": "#3fb950",
        "--status-deleted": "#f85149",
        "--status-modified": "#d29922",
        "--ui-warning": "#d29922",
        "--ui-warning-bg": "rgba(210, 153, 34, 0.1)",
      },
    },
  },
};

// Static theme imports (Vite needs static strings for tree-shaking)
const themeImports: Record<
  SyntaxThemeName,
  () => Promise<{ default: ThemeRegistrationRaw }>
> = {
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
  const localTheme = getLocalTheme(name);
  return localTheme
    ? !prefersDarkMode()
    : LIGHT_THEMES.has(name as SyntaxThemeName);
}

export function isAppThemeName(name: string): name is AppThemeName {
  return (APP_THEMES as readonly string[]).includes(name);
}

export function getLocalTheme(name: string): LocalThemeDefinition | null {
  return localThemes[name as LocalThemeName] ?? null;
}

export function resolveLocalThemeVariant(
  name: string,
  isDark: boolean,
): LocalThemeVariant | null {
  const localTheme = getLocalTheme(name);
  if (!localTheme) return null;
  return isDark ? localTheme.dark : localTheme.light;
}

export function resolveSyntaxThemeName(
  name: string,
  isDark = prefersDarkMode(),
): SyntaxThemeName {
  return (
    resolveLocalThemeVariant(name, isDark)?.syntaxTheme ??
    (name as SyntaxThemeName)
  );
}

export function prefersDarkMode(): boolean {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return false;
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
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
