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
import { createThemeVars, hexToHsl } from "./adaptive-theme";
import {
  SYNTAX_THEMES,
  type SyntaxThemeName,
  extractThemeInfo,
  getThemePair,
  loadThemeData,
  resolveSystemTheme,
} from "./theme-loader";

export const THEME_STORAGE_KEY = "buzz-theme";
const CACHE_KEY = "buzz-theme-cache";
export const ACCENT_STORAGE_KEY = "buzz-accent-color";
export const NEUTRAL_ACCENT = "neutral";
const FOLLOW_SYSTEM_KEY = "buzz-follow-system";
const VIDEO_REVIEW_NEUTRAL_ACCENT = "0 0% 98%";
const VIDEO_REVIEW_CHIP_SURFACE = "#161616";
const VIDEO_REVIEW_TEXT_CONTRAST = 4.5;
const VIDEO_REVIEW_CHIP_BACKGROUND_ALPHAS = [0.15, 0.3] as const;
const BUZZ_VIBRANCY_MATERIAL = "sidebar";

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
  hasPair: boolean;
  setTheme: (name: string) => void;
  setAccentColor: (color: string) => void;
  setFollowSystem: (enabled: boolean) => void;
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

/**
 * Toggle the opaque Buzz sidebar-gradient marker. This is always safe to apply
 * synchronously: `data-buzz-sidebar` paints solid gradient colors, so it never
 * makes the window see-through. The *translucent* treatment (transparent
 * root/body) is handled separately via {@link setBuzzTranslucent} because it
 * must be sequenced against the native vibrancy layer — see
 * {@link applyBuzzVibrancy}.
 */
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
    // Leaving Buzz: drop translucency synchronously here too. Going *opaque*
    // never shows desktop/prior content through, so there's no ordering risk
    // on the way out — only on the way in.
    setBuzzTranslucent(false);
  }
}

/**
 * Toggle the translucent (see-through) treatment: transparent root/body so the
 * native macOS vibrancy layer shows through behind the sidebar glass. The
 * transparent root/body themselves are driven by the `data-buzz-translucent`
 * CSS rule (theme.css), so we only flip the attribute here — no inline styles.
 *
 * IMPORTANT: enabling translucency exposes whatever the compositor paints
 * behind the webview. Only enable it once the native `NSVisualEffectView`
 * vibrancy layer is confirmed installed, otherwise there's a frame where the
 * transparent webview reveals the content behind it (the "main app nav
 * underneath" flicker). {@link applyBuzzVibrancy} owns that sequencing.
 */
function setBuzzTranslucent(enabled: boolean) {
  const root = document.documentElement;
  if (enabled) {
    root.setAttribute("data-buzz-translucent", "");
  } else {
    root.removeAttribute("data-buzz-translucent");
  }
}

/**
 * Monotonic token identifying the most recent vibrancy request. Because
 * {@link applyBuzzVibrancy} awaits the native `set_window_vibrancy` IPC, a rapid
 * Buzz → non-Buzz toggle can fire two overlapping calls whose awaits resolve out
 * of order. Each call captures the token before awaiting and re-checks it after;
 * a stale continuation (superseded by a newer request) bails without touching
 * translucency — otherwise the earlier Buzz call could re-add
 * `data-buzz-translucent` after the later non-Buzz call already cleared it,
 * leaving the window transparent under a non-Buzz theme.
 */
let buzzVibrancyRequest = 0;

/**
 * Whether the native vibrancy layer is confirmed installed for a Buzz theme.
 * Set true only after `set_window_vibrancy(true)` resolves; cleared as soon as a
 * new vibrancy request is issued (its outcome is not yet known).
 */
let buzzVibrancyReady = false;

/** The native layer does not need rebuilding when Buzz only changes mode. */
let buzzVibrancyEnabled = false;

/**
 * Enable the CSS translucency treatment, but only once BOTH prerequisites for
 * the current request are in place:
 *
 *  1. the native vibrancy layer is installed ({@link buzzVibrancyReady}), and
 *  2. the Buzz sidebar marker + gradient vars are applied (`data-buzz-sidebar`,
 *     set synchronously by {@link applyBuzzSidebar} inside {@link applyTheme}).
 *
 * Translucency clears the body/sidebar surfaces so the vibrancy layer shows
 * through; enabling it before the Buzz gradient vars are installed would flash a
 * transparent/unstyled sidebar. `applyTheme` (theme vars) and
 * `applyBuzzVibrancy` (native layer) are independent async effects that can win
 * their race in either order, so each calls this after its own step completes —
 * whichever lands last flips translucency on. The token check drops stale
 * continuations superseded by a newer theme switch.
 */
function maybeEnableBuzzTranslucent(themeName: string, requestToken: number) {
  if (requestToken !== buzzVibrancyRequest) return;
  if (!isBuzzTheme(themeName) || !isMacPlatform()) return;
  if (!buzzVibrancyReady) return;
  if (!document.documentElement.hasAttribute("data-buzz-sidebar")) return;
  setBuzzTranslucent(true);
}

/**
 * Sequence the native vibrancy layer and the CSS translucency so they land in
 * the right order and never leave a transparent webview with nothing painted
 * behind it:
 *
 * - Entering Buzz (macOS): install the vibrancy layer first (await the IPC),
 *   *then* flip on translucency. This closes the frame-gap where the root was
 *   transparent before the vibrancy view existed — the flicker.
 * - Leaving Buzz: translucency was already removed synchronously in
 *   `applyBuzzSidebar` (safe — opaque never shows through), so here we just
 *   clear the native layer.
 *
 * On non-macOS `set_window_vibrancy` is a no-op and translucency stays off, so
 * these platforms fall back to the opaque Buzz gradient.
 *
 * Overlapping calls are guarded by {@link buzzVibrancyRequest} so a stale async
 * continuation can't re-enable translucency after a newer theme superseded it.
 */
async function applyBuzzVibrancy(themeName: string) {
  const buzz = isBuzzTheme(themeName);
  const requestToken = ++buzzVibrancyRequest;

  // Buzz Light and Buzz Dark use the same native material. Rebuilding the
  // NSVisualEffectView on every mode change briefly clears the layer behind
  // the webview and makes the new CSS theme appear late. Keep the installed
  // layer and let applyTheme swap only the color tokens.
  if (buzz && buzzVibrancyEnabled && buzzVibrancyReady) {
    maybeEnableBuzzTranslucent(themeName, requestToken);
    return;
  }

  // A new request is in flight — the vibrancy layer's readiness for it is not
  // yet known, so any stale "ready" from a prior request must not gate this one.
  buzzVibrancyReady = false;

  if (!isTauri()) {
    // Web/dev preview: no native vibrancy layer exists, so translucency would
    // show raw page background. Keep it off; the opaque gradient stands in.
    setBuzzTranslucent(false);
    return;
  }

  try {
    await invokeTauri<void>("set_window_vibrancy", {
      enabled: buzz,
      material: BUZZ_VIBRANCY_MATERIAL,
    });
    // A newer theme change superseded this request while the IPC was in flight;
    // that later call owns the current translucency state, so don't clobber it.
    if (requestToken !== buzzVibrancyRequest) return;
    buzzVibrancyEnabled = buzz;
    // Native layer is installed. Record readiness and try to enable translucency
    // — but only if `applyBuzzSidebar` has already installed the Buzz gradient
    // vars. If that effect hasn't landed yet (the IPC won the race), it will
    // call maybeEnableBuzzTranslucent itself once the marker is applied.
    if (buzz && isMacPlatform()) {
      buzzVibrancyReady = true;
      maybeEnableBuzzTranslucent(themeName, requestToken);
    }
  } catch (error) {
    console.warn("set_window_vibrancy failed", error);
    if (requestToken !== buzzVibrancyRequest) return;
    // Vibrancy failed — don't go transparent or we'd show through to nothing.
    buzzVibrancyEnabled = false;
    setBuzzTranslucent(false);
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

    const accent =
      window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
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
async function applyTheme(
  name: SyntaxThemeName,
): Promise<{ isDark: boolean } | null> {
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
  // The Buzz gradient vars are now installed. If the vibrancy layer already
  // resolved for the current request (the IPC won the race against this theme
  // load), enable translucency now — otherwise applyBuzzVibrancy does it. This
  // is the second half of the two-effect handshake; the token guards against a
  // superseding theme switch.
  maybeEnableBuzzTranslucent(name, buzzVibrancyRequest);

  // Apply the accent synchronously in the same batch as the theme vars so the
  // browser paints the new theme + accent together. Doing this in a later
  // microtask (e.g. the caller's `.then`) let the previous accent flash on the
  // new theme for a frame — the flicker seen when switching to Buzz. Buzz
  // themes resolve to the neutral accent regardless of the stored value.
  applyAccentColor(
    resolveEffectiveAccent(
      name,
      window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT,
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

  return { isDark };
}

export function ThemeProvider({
  children,
  defaultTheme = "buzz",
}: ThemeProviderProps) {
  // Apply cached vars synchronously before first render
  const [selectedTheme, setSelectedTheme] = useState<string>(() => {
    applyCachedVars();
    return readStoredTheme(defaultTheme);
  });
  const [isDark, setIsDark] = useState<boolean>(() => {
    return document.documentElement.classList.contains("dark");
  });
  const [isLoading, setIsLoading] = useState(true);
  const loadingRef = useRef<string | null>(null);
  const [accentColor, setAccentColorState] = useState<string>(() => {
    return window.localStorage.getItem(ACCENT_STORAGE_KEY) ?? DEFAULT_ACCENT;
  });
  const [followSystem, setFollowSystemState] = useState<boolean>(() => {
    const stored = window.localStorage.getItem(FOLLOW_SYSTEM_KEY);
    if (stored !== null) return stored === "true";
    // Fresh profiles (no saved theme) default to System mode so the Buzz
    // default tracks the OS light/dark scheme. Profiles that picked a theme
    // before this toggle existed keep their fixed theme until they opt in.
    return window.localStorage.getItem(THEME_STORAGE_KEY) === null;
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
        setIsLoading(false);
      }
    });
  }, [effectiveTheme]);

  useEffect(() => {
    if (!isValidThemeName(effectiveTheme)) return;
    void applyBuzzVibrancy(effectiveTheme);
  }, [effectiveTheme]);

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

  const value: ThemeContextValue = {
    themeName: effectiveTheme,
    selectedThemeName: selectedTheme,
    isDark,
    isLoading,
    accentColor,
    followSystem,
    hasPair,
    setTheme,
    setAccentColor,
    setFollowSystem,
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
