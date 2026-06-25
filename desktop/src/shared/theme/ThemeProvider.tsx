import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { createThemeVars, hexToHsl } from "./adaptive-theme";
import {
  SYNTAX_THEMES,
  type SyntaxThemeName,
  extractThemeInfo,
  loadThemeData,
} from "./theme-loader";

export const THEME_STORAGE_KEY = "buzz-theme";
const CACHE_KEY = "buzz-theme-cache";
export const ACCENT_STORAGE_KEY = "buzz-accent-color";
export const NEUTRAL_ACCENT = "neutral";
const VIDEO_REVIEW_NEUTRAL_ACCENT = "0 0% 98%";
const VIDEO_REVIEW_CHIP_SURFACE = "#161616";
const VIDEO_REVIEW_TEXT_CONTRAST = 4.5;
const VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS = [0.15, 0.3] as const;

export const ACCENT_COLORS = [
  { name: "Neutral", value: NEUTRAL_ACCENT },
  { name: "Blue", value: "#3b82f6" },
  { name: "Cyan", value: "#06b6d4" },
  { name: "Green", value: "#22c55e" },
  { name: "Orange", value: "#f97316" },
  { name: "Red", value: "#ef4444" },
  { name: "Pink", value: "#ec4899" },
  { name: "Lilac", value: "#c0a2f1" },
  { name: "Purple", value: "#a855f7" },
  { name: "Indigo", value: "#6366f1" },
] as const;

const DEFAULT_ACCENT = "#3b82f6";

type ThemeContextValue = {
  themeName: string;
  isDark: boolean;
  isLoading: boolean;
  accentColor: string;
  setTheme: (name: string) => void;
  setAccentColor: (color: string) => void;
};

type ThemeProviderProps = {
  children: ReactNode;
  defaultTheme?: SyntaxThemeName;
};

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function isValidThemeName(name: string): name is SyntaxThemeName {
  return (SYNTAX_THEMES as readonly string[]).includes(name);
}

/** Read stored theme, migrating legacy "light"/"dark"/"system" values. */
function readStoredTheme(fallback: SyntaxThemeName): SyntaxThemeName {
  const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
  if (!stored) return fallback;

  // Migrate legacy values
  if (stored === "light") return "catppuccin-latte";
  if (stored === "dark" || stored === "system") return "houston";

  return isValidThemeName(stored) ? stored : fallback;
}

function getContrastColor(hex: string): string {
  const m = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (!m) return "#ffffff";
  const r = parseInt(m[1], 16);
  const g = parseInt(m[2], 16);
  const b = parseInt(m[3], 16);
  const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return lum > 0.5 ? "#000000" : "#ffffff";
}

type Rgb = {
  r: number;
  g: number;
  b: number;
};

function hexToRgb(hex: string): Rgb {
  const m = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (!m) return { r: 255, g: 255, b: 255 };
  return {
    r: parseInt(m[1], 16),
    g: parseInt(m[2], 16),
    b: parseInt(m[3], 16),
  };
}

function mixRgb(from: Rgb, to: Rgb, factor: number): Rgb {
  return {
    r: from.r + (to.r - from.r) * factor,
    g: from.g + (to.g - from.g) * factor,
    b: from.b + (to.b - from.b) * factor,
  };
}

function compositeRgb(foreground: Rgb, background: Rgb, alpha: number): Rgb {
  return mixRgb(background, foreground, alpha);
}

function relativeLuminance({ r, g, b }: Rgb): number {
  const [rs, gs, bs] = [r, g, b].map((channel) => {
    const value = channel / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * rs + 0.7152 * gs + 0.0722 * bs;
}

function contrastRatio(a: Rgb, b: Rgb): number {
  const aLum = relativeLuminance(a);
  const bLum = relativeLuminance(b);
  return (Math.max(aLum, bLum) + 0.05) / (Math.min(aLum, bLum) + 0.05);
}

function getReviewAccentForeground(hex: string): string {
  const accent = hexToRgb(hex);
  const surface = hexToRgb(VIDEO_REVIEW_CHIP_SURFACE);
  const white = { r: 255, g: 255, b: 255 };
  const backgrounds = VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS.map((alpha) =>
    compositeRgb(accent, surface, alpha),
  );
  let low = 0;
  let high = 1;

  for (let i = 0; i < 20; i++) {
    const mid = (low + high) / 2;
    const candidate = mixRgb(accent, white, mid);
    const minContrast = Math.min(
      ...backgrounds.map((background) => contrastRatio(candidate, background)),
    );

    if (minContrast >= VIDEO_REVIEW_TEXT_CONTRAST) {
      high = mid;
    } else {
      low = mid;
    }
  }

  return hexToHsl(rgbToHex(mixRgb(accent, white, high)));
}

function rgbToHex({ r, g, b }: Rgb): string {
  const clamp = (value: number) =>
    Math.max(0, Math.min(255, Math.round(value)));
  return `#${[r, g, b]
    .map((channel) => clamp(channel).toString(16).padStart(2, "0"))
    .join("")}`;
}

function applyAccentColor(value: string) {
  const root = document.documentElement;
  if (value === NEUTRAL_ACCENT) {
    const styles = window.getComputedStyle(root);
    const foreground = styles.getPropertyValue("--foreground").trim();
    const background = styles.getPropertyValue("--background").trim();
    root.style.setProperty("--buzz-selected-accent", foreground);
    root.style.setProperty(
      "--buzz-video-review-accent",
      VIDEO_REVIEW_NEUTRAL_ACCENT,
    );
    root.style.setProperty(
      "--buzz-video-review-accent-foreground",
      VIDEO_REVIEW_NEUTRAL_ACCENT,
    );
    root.style.setProperty("--primary", foreground);
    root.style.setProperty("--primary-foreground", background);
    root.style.setProperty("--sidebar-primary", foreground);
    root.style.setProperty("--sidebar-primary-foreground", background);
    root.style.setProperty("--sidebar-active", foreground);
    root.style.setProperty("--sidebar-active-foreground", background);
    return;
  }

  const hex = value;
  const accentHsl = hexToHsl(hex);
  const fgHsl = hexToHsl(getContrastColor(hex));
  root.style.setProperty("--buzz-selected-accent", accentHsl);
  root.style.setProperty("--buzz-video-review-accent", accentHsl);
  root.style.setProperty(
    "--buzz-video-review-accent-foreground",
    getReviewAccentForeground(hex),
  );
  root.style.setProperty("--primary", accentHsl);
  root.style.setProperty("--primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-primary", accentHsl);
  root.style.setProperty("--sidebar-primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-active", accentHsl);
  root.style.setProperty("--sidebar-active-foreground", fgHsl);
}

/** Apply cached CSS vars synchronously to prevent FOUC. */
function applyCachedVars(): string | null {
  try {
    const cached = window.localStorage.getItem(CACHE_KEY);
    if (!cached) return null;
    const { themeName, vars, isDark } = JSON.parse(cached);
    const root = document.documentElement;
    for (const [key, value] of Object.entries(vars)) {
      root.style.setProperty(key, value as string);
    }
    root.classList.remove("light", "dark");
    root.classList.add(isDark ? "dark" : "light");

    const accent =
      window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
    applyAccentColor(accent);

    return themeName;
  } catch {
    return null;
  }
}

/** Apply a theme: load data, derive CSS vars, set them on :root. */
async function applyTheme(name: SyntaxThemeName): Promise<{ isDark: boolean }> {
  const themeData = await loadThemeData(name);
  const info = extractThemeInfo(name, themeData);
  const { isDark, vars } = createThemeVars(info.bg, info.fg, info.comment, {
    added: info.added,
    deleted: info.deleted,
    modified: info.modified,
  });

  const root = document.documentElement;
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value);
  }

  root.classList.remove("light", "dark");
  root.classList.add(isDark ? "dark" : "light");

  // Cache for FOUC prevention
  try {
    window.localStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ themeName: name, vars, isDark }),
    );
  } catch {
    // Storage full — non-critical
  }

  return { isDark };
}

export function ThemeProvider({
  children,
  defaultTheme = "houston",
}: ThemeProviderProps) {
  // Apply cached vars synchronously before first render
  const [themeName, setThemeName] = useState<string>(() => {
    const cached = applyCachedVars();
    return cached ?? readStoredTheme(defaultTheme);
  });
  const [isDark, setIsDark] = useState<boolean>(() => {
    return document.documentElement.classList.contains("dark");
  });
  const [isLoading, setIsLoading] = useState(true);
  const loadingRef = useRef<string | null>(null);
  const [accentColor, setAccentColorState] = useState<string>(() => {
    return window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
  });

  useEffect(() => {
    if (!isValidThemeName(themeName)) return;

    // Track which theme we're loading to avoid race conditions
    const thisTheme = themeName;
    loadingRef.current = thisTheme;
    setIsLoading(true);

    applyTheme(themeName).then(({ isDark: dark }) => {
      // Only update if this is still the theme we want
      if (loadingRef.current === thisTheme) {
        setIsDark(dark);
        setIsLoading(false);
        // Re-apply accent after theme load (theme vars don't include primary)
        applyAccentColor(
          window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT,
        );
      }
    });
  }, [themeName]);

  useEffect(() => {
    applyAccentColor(accentColor);
  }, [accentColor]);

  const setTheme = useCallback((name: string) => {
    if (!isValidThemeName(name)) return;
    setThemeName(name);
    window.localStorage.setItem(THEME_STORAGE_KEY, name);
  }, []);

  const setAccentColor = useCallback((color: string) => {
    window.localStorage.setItem(ACCENT_STORAGE_KEY, color);
    setAccentColorState(color);
  }, []);

  const value: ThemeContextValue = {
    themeName,
    isDark,
    isLoading,
    accentColor,
    setTheme,
    setAccentColor,
  };

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme() {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error("useTheme must be used within a ThemeProvider");
  }
  return context;
}
