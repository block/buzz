import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invokeTauri } from "@/shared/api/tauri";
import { isMacPlatform } from "@/shared/lib/platform";
import { getStorageItem } from "@/shared/lib/safeStorage";
import { createThemeVars, hexToHsl } from "./adaptive-theme";
import {
  SYNTAX_THEMES,
  type SyntaxThemeName,
  type ThemeInfo,
  extractThemeInfo,
  getThemePair,
  loadThemeData,
  resolveSystemTheme,
} from "./theme-loader";

export const THEME_STORAGE_KEY = "buzz-theme";
const CACHE_KEY = "buzz-theme-cache";
export const ACCENT_STORAGE_KEY = "buzz-accent-color";
export const GLASS_BACKGROUND_STORAGE_KEY = "buzz-glass-background";
export const GLASS_OPACITY_STORAGE_KEY = "buzz-glass-opacity";
export const PROMINENT_ACTIVE_TAB_STORAGE_KEY = "buzz-prominent-active-tab";
export const GLASS_OPACITY_MIN = 30;
export const GLASS_OPACITY_MAX = 90;
export const DEFAULT_GLASS_BACKGROUND = false;
export const DEFAULT_GLASS_OPACITY = 65;
export const DEFAULT_PROMINENT_ACTIVE_TAB = false;
export const NEUTRAL_ACCENT = "neutral";
const FOLLOW_SYSTEM_KEY = "buzz-follow-system";
const VIDEO_REVIEW_NEUTRAL_ACCENT = "0 0% 98%";
const VIDEO_REVIEW_CHIP_SURFACE = "#161616";
const VIDEO_REVIEW_TEXT_CONTRAST = 4.5;
const VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS = [0.15, 0.3] as const;
const GLASS_VIBRANCY_MATERIAL = "sidebar";

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
  selectedThemeName: string;
  isDark: boolean;
  isLoading: boolean;
  accentColor: string;
  followSystem: boolean;
  glassBackground: boolean;
  glassOpacity: number;
  glassBackgroundSupported: boolean;
  prominentActiveTab: boolean;
  hasPair: boolean;
  terminalPalette: ThemeInfo["terminalPalette"] | null;
  setTheme: (name: string) => void;
  setAccentColor: (color: string) => void;
  setFollowSystem: (enabled: boolean) => void;
  applyAppearance: (appearance: {
    theme: SyntaxThemeName;
    accent: string;
    followSystem: boolean;
    glassBackground: boolean;
    glassOpacity: number;
    prominentActiveTab: boolean;
  }) => void;
  setGlassBackground: (enabled: boolean) => void;
  setGlassOpacity: (opacity: number) => void;
  setProminentActiveTab: (enabled: boolean) => void;
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
  // block/buzz#5078 — WebKit throws SecurityError from getItem under a
  // denied-storage origin; the throw-safe helper lets the provider degrade to
  // the fallback instead of unmounting the root during first render.
  const stored = getStorageItem(THEME_STORAGE_KEY);
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

/**
 * The Buzz themes ship with a fixed neutral accent (the GitHub black/white
 * foreground) rather than a user-selectable accent color. When a Buzz theme is
 * active we force `NEUTRAL_ACCENT` regardless of the stored preference, and the
 * appearance panel hides the accent picker. The user's chosen accent is left
 * untouched in storage so it returns when they switch back to another theme.
 */
export function isBuzzTheme(themeName: string): boolean {
  return themeName === "buzz" || themeName === "buzz-dark";
}

/**
 * Resolve the accent to actually apply for a theme: Buzz themes are pinned to
 * the neutral accent; every other theme uses the stored/selected accent.
 */
function resolveEffectiveAccent(
  themeName: string,
  accentColor: string,
): string {
  return isBuzzTheme(themeName) ? NEUTRAL_ACCENT : accentColor;
}

/** Toggle the Buzz-specific gradient marker independently from glass. */
function applyBuzzSidebar(themeName: string) {
  const root = document.documentElement;
  if (isBuzzTheme(themeName)) {
    root.setAttribute("data-buzz-sidebar", "");
    // Keep the concrete Buzz variant on the root as well as the generic
    // marker. The gradient stylesheet matches this attribute directly, which
    // makes WKWebView invalidate the painted background when light/dark mode
    // changes instead of relying only on a custom-property dependency update.
    root.setAttribute("data-buzz-theme", themeName);
  } else {
    root.removeAttribute("data-buzz-sidebar");
    root.removeAttribute("data-buzz-theme");
  }
}

/**
 * Toggle the transparent CSS surfaces that reveal native macOS vibrancy behind
 * the navigation and outer chrome. The center content panel remains opaque.
 *
 * IMPORTANT: enabling glass exposes whatever the compositor paints
 * behind the webview. Only enable it once the native `NSVisualEffectView`
 * vibrancy layer and the active theme colors are both ready.
 */
function setGlassBackgroundActive(enabled: boolean) {
  const root = document.documentElement;
  if (enabled) {
    // WKWebView keeps its page canvas opaque unless the root background is an
    // inline transparent value, even when the equivalent author rule wins in
    // the stylesheet. Clear it before exposing the native vibrancy layer.
    root.style.setProperty("background", "transparent");
    root.setAttribute("data-glass-background", "");
  } else {
    root.removeAttribute("data-glass-background");
    root.style.removeProperty("background");
  }
}

/** Apply the optional higher-contrast selected navigation surface. */
function setProminentActiveTabActive(enabled: boolean) {
  document.documentElement.toggleAttribute(
    "data-prominent-active-tab",
    enabled,
  );
}

function clampGlassOpacity(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_GLASS_OPACITY;
  return Math.min(
    GLASS_OPACITY_MAX,
    Math.max(GLASS_OPACITY_MIN, Math.round(value)),
  );
}

function readStoredGlassOpacity(): number {
  const stored = getStorageItem(GLASS_OPACITY_STORAGE_KEY);
  return stored === null
    ? DEFAULT_GLASS_OPACITY
    : clampGlassOpacity(Number(stored));
}

/** Set the tint opacity layered above native blur; lower values reveal more. */
function applyGlassOpacity(value: number) {
  document.documentElement.style.setProperty(
    "--glass-background-opacity",
    `${clampGlassOpacity(value)}%`,
  );
}

/** Only the newest overlapping native glass request may update CSS state. */
let glassVibrancyRequest = 0;

/** Whether the native vibrancy layer is confirmed installed. */
let glassVibrancyReady = false;

/** The native layer does not need rebuilding when only the theme changes. */
let glassVibrancyEnabled = false;

/** Mirrors the current preference for the async theme/native handshake. */
let glassBackgroundPreferenceEnabled = false;

/** Theme colors must be installed before the transparent surface is exposed. */
let glassThemeReady = false;

/**
 * Theme loading and native vibrancy can finish in either order. Whichever lands
 * last reveals the glass once both prerequisites are ready.
 */
function maybeEnableGlassBackground(requestToken: number) {
  if (requestToken !== glassVibrancyRequest) return;
  if (!glassBackgroundPreferenceEnabled || !isMacPlatform()) return;
  if (!glassVibrancyReady || !glassThemeReady) return;
  setGlassBackgroundActive(true);
}

/**
 * Install native vibrancy before making the webview transparent. Non-macOS and
 * web builds retain the normal opaque theme surface.
 */
async function applyWindowGlass(enabled: boolean) {
  glassBackgroundPreferenceEnabled = enabled;
  const requestToken = ++glassVibrancyRequest;

  if (!isTauri() || !isMacPlatform()) {
    glassBackgroundPreferenceEnabled = false;
    glassVibrancyEnabled = false;
    glassVibrancyReady = false;
    setGlassBackgroundActive(false);
    return;
  }

  if (enabled && glassVibrancyEnabled && glassVibrancyReady) {
    maybeEnableGlassBackground(requestToken);
    return;
  }

  glassVibrancyReady = false;

  try {
    await invokeTauri<void>("set_window_vibrancy", {
      enabled,
      material: GLASS_VIBRANCY_MATERIAL,
    });
    if (requestToken !== glassVibrancyRequest) return;
    glassVibrancyEnabled = enabled;
    if (enabled && isMacPlatform()) {
      glassVibrancyReady = true;
      maybeEnableGlassBackground(requestToken);
    }
  } catch (error) {
    console.warn("set_window_vibrancy failed", error);
    if (requestToken !== glassVibrancyRequest) return;
    glassVibrancyEnabled = false;
    setGlassBackgroundActive(false);
  }
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
    applyBuzzSidebar(themeName);
    glassThemeReady = true;

    const accent = getStorageItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
    // Pin Buzz themes to the neutral accent here too, matching applyTheme.
    // Otherwise a cached Buzz theme + non-neutral stored accent flashes the
    // old accent on reload until the async applyTheme effect runs.
    applyAccentColor(resolveEffectiveAccent(themeName, accent));

    return themeName;
  } catch {
    return null;
  }
}

/** The latest theme load is the only one allowed to write document styles. */
let themeApplyRequest = 0;

/** Apply a theme: load data, derive CSS vars, set them on :root. */
async function applyTheme(name: SyntaxThemeName): Promise<{
  isDark: boolean;
  terminalPalette: ThemeInfo["terminalPalette"];
} | null> {
  const requestToken = ++themeApplyRequest;
  const themeData = await loadThemeData(name);
  if (requestToken !== themeApplyRequest) return null;

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
  applyBuzzSidebar(name);
  glassThemeReady = true;
  maybeEnableGlassBackground(glassVibrancyRequest);

  // Apply the accent synchronously in the same batch as the theme vars so the
  // browser paints the new theme + accent together. Doing this in a later
  // microtask (e.g. the caller's `.then`) let the previous accent flash on the
  // new theme for a frame — the flicker seen when switching to Buzz. Buzz
  // themes resolve to the neutral accent regardless of the stored value.
  applyAccentColor(
    resolveEffectiveAccent(
      name,
      getStorageItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT,
    ),
  );

  // Cache for FOUC prevention
  try {
    window.localStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ themeName: name, vars, isDark }),
    );
  } catch {
    // Storage full — non-critical
  }

  return { isDark, terminalPalette: info.terminalPalette };
}

export function ThemeProvider({
  children,
  defaultTheme = "buzz",
}: ThemeProviderProps) {
  const glassBackgroundSupported = isTauri() && isMacPlatform();

  // Apply cached vars synchronously before first render
  const [selectedTheme, setSelectedTheme] = useState<string>(() => {
    applyCachedVars();
    return readStoredTheme(defaultTheme);
  });
  const [isDark, setIsDark] = useState<boolean>(() => {
    return document.documentElement.classList.contains("dark");
  });
  const [isLoading, setIsLoading] = useState(true);
  const [terminalPalette, setTerminalPalette] = useState<
    ThemeInfo["terminalPalette"] | null
  >(null);
  const loadingRef = useRef<string | null>(null);
  const [accentColor, setAccentColorState] = useState<string>(() => {
    // block/buzz#5078 — use the throw-safe accessor for init-time reads; a
    // denied-storage origin would otherwise kill the root on first mount.
    return getStorageItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
  });
  const [glassBackground, setGlassBackgroundState] = useState<boolean>(() => {
    const stored = getStorageItem(GLASS_BACKGROUND_STORAGE_KEY);
    // Glass is opt-in. Explicitly saved preferences remain intact, while a
    // fresh profile starts with the normal opaque window treatment. The state
    // represents the preference, not platform capability, so a machine
    // that cannot render glass must retain and sync a Mac's enabled setting.
    const enabled =
      stored === null ? DEFAULT_GLASS_BACKGROUND : stored === "true";
    glassBackgroundPreferenceEnabled = glassBackgroundSupported && enabled;
    return enabled;
  });
  const [glassOpacity, setGlassOpacityState] = useState<number>(() => {
    const opacity = readStoredGlassOpacity();
    applyGlassOpacity(opacity);
    return opacity;
  });
  const [prominentActiveTab, setProminentActiveTabState] = useState<boolean>(
    () => {
      const stored = getStorageItem(PROMINENT_ACTIVE_TAB_STORAGE_KEY);
      return stored === null ? DEFAULT_PROMINENT_ACTIVE_TAB : stored === "true";
    },
  );
  const [followSystem, setFollowSystemState] = useState<boolean>(() => {
    const stored = getStorageItem(FOLLOW_SYSTEM_KEY);
    if (stored !== null) return stored === "true";
    // Fresh profiles (no saved theme) default to System mode so the Buzz
    // default tracks the OS light/dark scheme. Profiles that picked a theme
    // before this toggle existed keep their fixed theme until they opt in.
    return getStorageItem(THEME_STORAGE_KEY) === null;
  });
  const [systemIsDark, setSystemIsDark] = useState<boolean>(() => {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });

  // Resolve the effective theme based on follow-system preference
  const effectiveTheme = (() => {
    if (!followSystem || !isValidThemeName(selectedTheme)) return selectedTheme;
    return resolveSystemTheme(selectedTheme as SyntaxThemeName, systemIsDark);
  })();

  // Check if the selected theme has a pair (for UI hint)
  const hasPair = isValidThemeName(selectedTheme)
    ? getThemePair(selectedTheme as SyntaxThemeName) !== null
    : false;

  useEffect(() => {
    if (!isValidThemeName(effectiveTheme)) return;

    // Track which theme we're loading to avoid race conditions
    const thisTheme = effectiveTheme;
    loadingRef.current = thisTheme;
    setIsLoading(true);

    applyTheme(effectiveTheme as SyntaxThemeName).then((result) => {
      if (!result) return;
      // Only update if this is still the theme we want. The accent is applied
      // inside applyTheme (synchronously with the theme vars), so there's no
      // separate re-application here — that avoided the switch-time flicker.
      if (loadingRef.current === thisTheme) {
        setIsDark(result.isDark);
        setTerminalPalette(result.terminalPalette);
        setIsLoading(false);
      }
    });
  }, [effectiveTheme]);

  useEffect(() => {
    // `initial-render-ready` fires from a layout effect, so it is already
    // enqueued before this passive effect's IPC call is dispatched. The native
    // reveal can in theory precede the transparency call; the Rust-side
    // stable-geometry wait provides the gap in practice, and a brief opaque
    // first frame is the accepted worst case for glass users.
    void applyWindowGlass(glassBackground);
  }, [glassBackground]);

  // The stronger selected-row treatment belongs exclusively to Buzz. Keep
  // the saved preference so it is restored when the user returns to Buzz,
  // but remove the live marker for every other theme.
  useEffect(() => {
    setProminentActiveTabActive(
      prominentActiveTab && isBuzzTheme(effectiveTheme),
    );
  }, [effectiveTheme, prominentActiveTab]);

  // Listen for system color scheme changes when followSystem is enabled
  useEffect(() => {
    if (!followSystem) return;

    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handleMediaChange = (event: MediaQueryListEvent) => {
      setSystemIsDark(event.matches);
    };
    let disposed = false;
    let unlistenNativeTheme: (() => void) | undefined;

    setSystemIsDark(mq.matches);
    mq.addEventListener("change", handleMediaChange);

    // WKWebView can update the media query value without dispatching its
    // change event until the page reloads. Tauri's native window event arrives
    // immediately when macOS appearance changes, so use it as the reliable app
    // signal while retaining matchMedia for the browser build.
    if (isTauri()) {
      void getCurrentWindow()
        .onThemeChanged(({ payload }) => {
          if (!disposed) setSystemIsDark(payload === "dark");
        })
        .then((unlisten) => {
          if (disposed) {
            unlisten();
          } else {
            unlistenNativeTheme = unlisten;
          }
        })
        .catch((error) => {
          console.warn("system theme listener unavailable", error);
        });
    }

    return () => {
      disposed = true;
      mq.removeEventListener("change", handleMediaChange);
      unlistenNativeTheme?.();
    };
  }, [followSystem]);

  // Re-apply the accent when the user picks a new swatch or the effective theme
  // changes. applyTheme already applies the (Buzz-neutral-aware) accent in the
  // same synchronous batch as the theme vars — the flicker fix — so this effect
  // is idempotent on theme changes and simply covers accent-only changes.
  useEffect(() => {
    applyAccentColor(resolveEffectiveAccent(effectiveTheme, accentColor));
  }, [accentColor, effectiveTheme]);

  const setTheme = useCallback((name: string) => {
    if (!isValidThemeName(name)) return;
    setSelectedTheme(name);
    window.localStorage.setItem(THEME_STORAGE_KEY, name);
  }, []);

  const setAccentColor = useCallback((color: string) => {
    window.localStorage.setItem(ACCENT_STORAGE_KEY, color);
    setAccentColorState(color);
  }, []);

  const setFollowSystem = useCallback((enabled: boolean) => {
    window.localStorage.setItem(FOLLOW_SYSTEM_KEY, enabled ? "true" : "false");
    setFollowSystemState(enabled);
  }, []);

  const applyAppearance = useCallback(
    (appearance: {
      theme: SyntaxThemeName;
      accent: string;
      followSystem: boolean;
      glassBackground: boolean;
      glassOpacity: number;
      prominentActiveTab: boolean;
    }) => {
      // Write the complete preference before updating state so applyTheme reads
      // the target community's accent in the same batch, never the previous one.
      try {
        window.localStorage.setItem(THEME_STORAGE_KEY, appearance.theme);
        window.localStorage.setItem(ACCENT_STORAGE_KEY, appearance.accent);
        window.localStorage.setItem(
          FOLLOW_SYSTEM_KEY,
          appearance.followSystem ? "true" : "false",
        );
        window.localStorage.setItem(
          GLASS_BACKGROUND_STORAGE_KEY,
          appearance.glassBackground ? "true" : "false",
        );
        window.localStorage.setItem(
          GLASS_OPACITY_STORAGE_KEY,
          String(clampGlassOpacity(appearance.glassOpacity)),
        );
        window.localStorage.setItem(
          PROMINENT_ACTIVE_TAB_STORAGE_KEY,
          appearance.prominentActiveTab ? "true" : "false",
        );
      } catch {
        // Keep the active appearance responsive even if the local cache is full.
      }
      setSelectedTheme(appearance.theme);
      setAccentColorState(appearance.accent);
      setFollowSystemState(appearance.followSystem);
      const nextGlassOpacity = clampGlassOpacity(appearance.glassOpacity);
      applyGlassOpacity(nextGlassOpacity);
      glassBackgroundPreferenceEnabled =
        glassBackgroundSupported && appearance.glassBackground;
      if (!glassBackgroundPreferenceEnabled) {
        setGlassBackgroundActive(false);
      }
      setGlassBackgroundState(appearance.glassBackground);
      setGlassOpacityState(nextGlassOpacity);
      setProminentActiveTabState(appearance.prominentActiveTab);
    },
    [glassBackgroundSupported],
  );

  const setGlassBackground = useCallback(
    (enabled: boolean) => {
      if (!glassBackgroundSupported) {
        glassBackgroundPreferenceEnabled = false;
        setGlassBackgroundActive(false);
        setGlassBackgroundState(false);
        return;
      }

      window.localStorage.setItem(
        GLASS_BACKGROUND_STORAGE_KEY,
        enabled ? "true" : "false",
      );
      glassBackgroundPreferenceEnabled = enabled;
      if (!enabled) {
        setGlassBackgroundActive(false);
      }
      setGlassBackgroundState(enabled);
    },
    [glassBackgroundSupported],
  );

  const setGlassOpacity = useCallback((opacity: number) => {
    const nextOpacity = clampGlassOpacity(opacity);
    window.localStorage.setItem(GLASS_OPACITY_STORAGE_KEY, String(nextOpacity));
    applyGlassOpacity(nextOpacity);
    setGlassOpacityState(nextOpacity);
  }, []);

  const setProminentActiveTab = useCallback((enabled: boolean) => {
    window.localStorage.setItem(
      PROMINENT_ACTIVE_TAB_STORAGE_KEY,
      enabled ? "true" : "false",
    );
    setProminentActiveTabState(enabled);
  }, []);

  const value: ThemeContextValue = {
    themeName: effectiveTheme,
    selectedThemeName: selectedTheme,
    isDark,
    isLoading,
    accentColor,
    followSystem,
    glassBackground,
    glassOpacity,
    glassBackgroundSupported,
    prominentActiveTab,
    hasPair,
    terminalPalette,
    setTheme,
    setAccentColor,
    setFollowSystem,
    applyAppearance,
    setGlassBackground,
    setGlassOpacity,
    setProminentActiveTab,
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
