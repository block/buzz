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
  type AppThemeName,
  type SyntaxThemeName,
  extractThemeInfo,
  isAppThemeName,
  loadThemeData,
  prefersDarkMode,
  resolveLocalThemeVariant,
} from "./theme-loader";

const STORAGE_KEY = "sprout-theme";
const CACHE_KEY = "sprout-theme-cache";
const ACCENT_KEY = "sprout-accent-color";
export const NEUTRAL_ACCENT = "neutral";

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
  defaultTheme?: AppThemeName;
};

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined);

function normalizeStoredTheme(stored: string | null): AppThemeName | null {
  if (!stored) return null;
  if (stored === "neutral-light" || stored === "neutral-dark") return "neutral";

  // Migrate legacy values.
  if (stored === "light") return "catppuccin-latte";
  if (stored === "dark") return "houston";
  if (stored === "system") return "houston";

  return isAppThemeName(stored) ? stored : null;
}

/** Read stored theme, migrating legacy "light"/"dark"/"system" values. */
function readStoredTheme(fallback: AppThemeName): AppThemeName {
  return (
    normalizeStoredTheme(window.localStorage.getItem(STORAGE_KEY)) ?? fallback
  );
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

function applyAccentColor(value: string) {
  const root = document.documentElement;
  if (value === NEUTRAL_ACCENT) {
    const styles = window.getComputedStyle(root);
    const foreground = styles.getPropertyValue("--foreground").trim();
    const background = styles.getPropertyValue("--background").trim();
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
  root.style.setProperty("--primary", accentHsl);
  root.style.setProperty("--primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-primary", accentHsl);
  root.style.setProperty("--sidebar-primary-foreground", fgHsl);
  root.style.setProperty("--sidebar-active", accentHsl);
  root.style.setProperty("--sidebar-active-foreground", fgHsl);
}

/** Apply initial CSS vars synchronously to prevent FOUC. */
function applyCachedVars(themeName: AppThemeName, systemIsDark: boolean) {
  try {
    const localVariant = resolveLocalThemeVariant(themeName, systemIsDark);
    if (localVariant) {
      const root = document.documentElement;
      for (const [key, value] of Object.entries(localVariant.vars)) {
        root.style.setProperty(key, value);
      }
      root.classList.remove("light", "dark");
      root.classList.add(systemIsDark ? "dark" : "light");
      const accent = window.localStorage.getItem(ACCENT_KEY) ?? DEFAULT_ACCENT;
      applyAccentColor(accent);
      return;
    }

    const cached = window.localStorage.getItem(CACHE_KEY);
    if (!cached) return;
    const { themeName: cachedThemeName, vars, isDark } = JSON.parse(cached);
    if (normalizeStoredTheme(cachedThemeName) !== themeName) return;

    const root = document.documentElement;
    for (const [key, value] of Object.entries(vars)) {
      root.style.setProperty(key, value as string);
    }
    root.classList.remove("light", "dark");
    root.classList.add(isDark ? "dark" : "light");

    const accent = window.localStorage.getItem(ACCENT_KEY) ?? DEFAULT_ACCENT;
    applyAccentColor(accent);
  } catch {
    // Ignore corrupt cache and let the async theme application correct it.
  }
}

/** Apply a theme: load data, derive CSS vars, set them on :root. */
async function applyTheme(
  name: AppThemeName,
  systemIsDark: boolean,
): Promise<{ isDark: boolean }> {
  let isDark: boolean;
  let vars: Record<string, string>;

  const localVariant = resolveLocalThemeVariant(name, systemIsDark);
  if (localVariant) {
    isDark = systemIsDark;
    vars = localVariant.vars;
  } else {
    const themeData = await loadThemeData(name as SyntaxThemeName);
    const info = extractThemeInfo(name, themeData);
    ({ isDark, vars } = createThemeVars(info.bg, info.fg, info.comment, {
      added: info.added,
      deleted: info.deleted,
      modified: info.modified,
    }));
  }

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
  const [systemIsDark, setSystemIsDark] = useState(prefersDarkMode);
  // Apply cached vars synchronously before first render
  const [themeName, setThemeName] = useState<string>(() => {
    const initialTheme = readStoredTheme(defaultTheme);
    applyCachedVars(initialTheme, systemIsDark);
    return initialTheme;
  });
  const [isDark, setIsDark] = useState<boolean>(() => {
    if (resolveLocalThemeVariant(themeName, systemIsDark)) return systemIsDark;
    return document.documentElement.classList.contains("dark");
  });
  const [isLoading, setIsLoading] = useState(true);
  const loadingRef = useRef<string | null>(null);
  const [accentColor, setAccentColorState] = useState<string>(() => {
    return window.localStorage.getItem(ACCENT_KEY) ?? DEFAULT_ACCENT;
  });

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const handleChange = (event: MediaQueryListEvent) => {
      setSystemIsDark(event.matches);
    };
    media.addEventListener("change", handleChange);
    return () => {
      media.removeEventListener("change", handleChange);
    };
  }, []);

  // Load and apply theme
  useEffect(() => {
    if (!isAppThemeName(themeName)) return;

    // Track which theme we're loading to avoid race conditions
    const thisLoadKey = `${themeName}:${systemIsDark}`;
    loadingRef.current = thisLoadKey;
    setIsLoading(true);

    applyTheme(themeName, systemIsDark).then(({ isDark: dark }) => {
      // Only update if this is still the theme we want
      if (loadingRef.current === thisLoadKey) {
        setIsDark(dark);
        setIsLoading(false);
        // Re-apply accent after theme load since theme vars reset primary colors.
        applyAccentColor(
          window.localStorage.getItem(ACCENT_KEY) ?? DEFAULT_ACCENT,
        );
      }
    });
  }, [themeName, systemIsDark]);

  // Apply accent color changes
  useEffect(() => {
    applyAccentColor(accentColor);
  }, [accentColor]);

  const setTheme = useCallback((name: string) => {
    if (!isAppThemeName(name)) return;
    setThemeName(name);
    window.localStorage.setItem(STORAGE_KEY, name);
  }, []);

  const setAccentColor = useCallback((color: string) => {
    window.localStorage.setItem(ACCENT_KEY, color);
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
