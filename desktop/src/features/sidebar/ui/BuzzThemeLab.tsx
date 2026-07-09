import * as React from "react";
import { Paintbrush, RotateCcw, X } from "lucide-react";
import { invokeTauri } from "@/shared/api/tauri";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { Button } from "@/shared/ui/button";

/**
 * Buzz Theme Lab - a dev-only floating panel for tuning the left-nav Buzz
 * theme knobs live.
 *
 * Every control writes a CSS custom property straight onto the document root
 * as an inline style, so the sidebar updates instantly and what you see maps
 * 1:1 to the values you can commit into `theme.css`. Nothing here persists -
 * reloading the app drops all overrides. Use "Copy CSS" to grab the tuned set.
 *
 * Only rendered under the Buzz / Buzz Dark themes, and only in dev builds
 * (see the guard at the AppShell mount site).
 */

type VarFormat = "hex" | "hsl";

type Knob = {
  /** CSS custom property name (without the leading `--`). */
  cssVar: string;
  label: string;
  /** How the value is stored in `theme.css`: raw hex vs an HSL triplet. */
  format: VarFormat;
  /** Fallback hex used when the var is unset/`inherit`, for the picker. */
  fallback: string;
};

type Group = {
  title: string;
  knobs: Knob[];
};

// Light and dark carry different gradient defaults (see theme.css). The other
// knobs share defaults but each variant is tuned independently at runtime.
function groupsFor(isDark: boolean): Group[] {
  return [
    {
      title: "Gradient",
      knobs: [
        {
          cssVar: "buzz-gradient-top",
          label: "Top",
          format: "hex",
          fallback: isDark ? "#2b2b18" : "#e6e6b6",
        },
        {
          cssVar: "buzz-gradient-bottom",
          label: "Bottom",
          format: "hex",
          fallback: isDark ? "#1b2530" : "#c4d0da",
        },
      ],
    },
    {
      title: "Row text",
      knobs: [
        {
          cssVar: "buzz-channel-fg",
          label: "Channels",
          format: "hex",
          fallback: isDark ? "#c8cff0" : "#4c5163",
        },
        {
          cssVar: "buzz-dm-fg",
          label: "Direct messages",
          format: "hex",
          fallback: isDark ? "#c8cff0" : "#4c5163",
        },
        {
          cssVar: "buzz-nav-fg",
          label: "Inbox / Agents nav",
          format: "hex",
          fallback: isDark ? "#c8cff0" : "#4c5163",
        },
      ],
    },
    {
      title: "Hover",
      knobs: [
        {
          cssVar: "buzz-hover-fill",
          label: "Hover fill",
          format: "hex",
          fallback: "#ffffff",
        },
      ],
    },
    {
      title: "Active row",
      knobs: [
        {
          cssVar: "buzz-active-fill",
          label: "Active fill",
          format: "hsl",
          fallback: "#ffffff",
        },
        {
          cssVar: "buzz-active-foreground",
          label: "Active text",
          format: "hsl",
          fallback: isDark ? "#ffffff" : "#24292f",
        },
      ],
    },
  ];
}

// --- color helpers -----------------------------------------------------------

function clampByte(n: number): number {
  return Math.max(0, Math.min(255, Math.round(n)));
}

function toHex2(n: number): string {
  return clampByte(n).toString(16).padStart(2, "0");
}

/** Convert an HSL triplet string ("H S% L%") to a #rrggbb hex string. */
function hslTripletToHex(triplet: string): string | null {
  const m = /^\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%\s*$/.exec(triplet);
  if (!m) return null;
  const h = parseFloat(m[1]) / 360;
  const s = parseFloat(m[2]) / 100;
  const l = parseFloat(m[3]) / 100;

  if (s === 0) {
    const v = clampByte(l * 255);
    return `#${toHex2(v)}${toHex2(v)}${toHex2(v)}`;
  }

  const hue = (p: number, q: number, t: number): number => {
    let tt = t;
    if (tt < 0) tt += 1;
    if (tt > 1) tt -= 1;
    if (tt < 1 / 6) return p + (q - p) * 6 * tt;
    if (tt < 1 / 2) return q;
    if (tt < 2 / 3) return p + (q - p) * (2 / 3 - tt) * 6;
    return p;
  };

  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const r = hue(p, q, h + 1 / 3);
  const g = hue(p, q, h);
  const b = hue(p, q, h - 1 / 3);
  return `#${toHex2(r * 255)}${toHex2(g * 255)}${toHex2(b * 255)}`;
}

/** Convert #rrggbb to an HSL triplet string ("H S% L%"). */
function hexToHslTriplet(hex: string): string {
  const m = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})/i.exec(hex);
  if (!m) return "0 0% 0%";
  const r = parseInt(m[1], 16) / 255;
  const g = parseInt(m[2], 16) / 255;
  const b = parseInt(m[3], 16) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return `0 0% ${(l * 100).toFixed(1)}%`;
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
  else if (max === g) h = ((b - r) / d + 2) / 6;
  else h = ((r - g) / d + 4) / 6;
  return `${(h * 360).toFixed(1)} ${(s * 100).toFixed(2)}% ${(l * 100).toFixed(1)}%`;
}

/** Read the current picker-friendly hex for a knob from the live document. */
function readKnobHex(knob: Knob): string {
  const root = document.documentElement;
  const raw = getComputedStyle(root)
    .getPropertyValue(`--${knob.cssVar}`)
    .trim();
  if (!raw || raw === "inherit") return knob.fallback;
  if (knob.format === "hex") {
    return /^#[0-9a-f]{6}$/i.test(raw) ? raw : knob.fallback;
  }
  return hslTripletToHex(raw) ?? knob.fallback;
}

/** Store the value in the format `theme.css` expects for this knob. */
function knobStoreValue(knob: Knob, hex: string): string {
  return knob.format === "hex" ? hex : hexToHslTriplet(hex);
}

// --- translucency + vibrancy -------------------------------------------------

// Default continuous translucency knobs. `glassIntensity` controls how much
// frosted glass is applied (0 = off, 1 = full), `mix` crossfades the surfaces
// between transparent/frosted and the solid Buzz gradient.
const DEFAULT_GLASS_INTENSITY = 1;
const DEFAULT_TRANSLUCENCY_MIX = 0.7;
const FROST_WASH_ALPHA = 0.08;

// macOS NSVisualEffectMaterial presets exposed by the `set_window_vibrancy`
// command. The native effect view makes the window transparent and supplies
// the frosted-glass layer behind the translucent CSS surface.
const VIBRANCY_MATERIALS = [
  "sidebar",
  "hud-window",
  "under-window-background",
  "fullscreen-ui",
  "header-view",
  "popover",
  "menu",
  "titlebar",
] as const;

type VibrancyMaterial = (typeof VIBRANCY_MATERIALS)[number];

function clampUnit(n: number): number {
  return Math.max(0, Math.min(1, n));
}

function formatPercent(n: number): string {
  return `${Number((n * 100).toFixed(2))}%`;
}

function setTranslucencyStyleVars(
  root: HTMLElement,
  glassIntensity: number,
  mix: number,
) {
  const clampedGlass = clampUnit(glassIntensity);
  const clampedMix = clampUnit(mix);
  const glassAmount = (1 - clampedMix) * clampedGlass;
  root.style.setProperty("background-color", "transparent");
  root.style.setProperty("background-image", "none");
  root.style.setProperty("--buzz-glass-intensity", String(clampedGlass));
  root.style.setProperty("--buzz-translucency-mix", String(clampedMix));
  root.style.setProperty(
    "--buzz-translucency-gradient-alpha",
    formatPercent(clampedMix),
  );
  root.style.setProperty(
    "--buzz-translucency-wash-alpha",
    formatPercent(glassAmount * FROST_WASH_ALPHA),
  );
}

function clearTranslucencyStyleVars(root: HTMLElement) {
  root.style.removeProperty("background-color");
  root.style.removeProperty("background-image");
  root.style.removeProperty("--buzz-glass-intensity");
  root.style.removeProperty("--buzz-translucency-mix");
  root.style.removeProperty("--buzz-translucency-gradient-alpha");
  root.style.removeProperty("--buzz-translucency-wash-alpha");
}

async function applyVibrancy(enabled: boolean, material: VibrancyMaterial) {
  try {
    await invokeTauri<void>("set_window_vibrancy", { enabled, material });
  } catch (error) {
    // Non-Tauri (browser dev) or unsupported platform - the CSS preview still
    // applies; just log so it is not silent.
    console.warn("set_window_vibrancy failed", error);
  }
}

// --- component ---------------------------------------------------------------

export function BuzzThemeLab() {
  const { isDark, themeName } = useTheme();
  const isBuzz = themeName === "buzz" || themeName === "buzz-dark";
  const [open, setOpen] = React.useState(false);
  // Bump to force pickers to re-read after a reset.
  const [revision, setRevision] = React.useState(0);
  const [copied, setCopied] = React.useState(false);
  const [glassIntensity, setGlassIntensity] = React.useState(
    DEFAULT_GLASS_INTENSITY,
  );
  const [material, setMaterial] = React.useState<VibrancyMaterial>("sidebar");
  const [mix, setMix] = React.useState(DEFAULT_TRANSLUCENCY_MIX);
  const glassOn = glassIntensity > 0;

  const groups = React.useMemo(() => groupsFor(isDark), [isDark]);

  const setVar = React.useCallback((knob: Knob, hex: string) => {
    document.documentElement.style.setProperty(
      `--${knob.cssVar}`,
      knobStoreValue(knob, hex),
    );
  }, []);

  const reset = React.useCallback(() => {
    const root = document.documentElement;
    for (const group of groups) {
      for (const knob of group.knobs) {
        root.style.removeProperty(`--${knob.cssVar}`);
      }
    }
    root.removeAttribute("data-buzz-translucent");
    clearTranslucencyStyleVars(root);
    setGlassIntensity(DEFAULT_GLASS_INTENSITY);
    setMaterial("sidebar");
    setMix(DEFAULT_TRANSLUCENCY_MIX);
    if (DEFAULT_GLASS_INTENSITY > 0) {
      root.setAttribute("data-buzz-translucent", "");
      setTranslucencyStyleVars(
        root,
        DEFAULT_GLASS_INTENSITY,
        DEFAULT_TRANSLUCENCY_MIX,
      );
      void applyVibrancy(true, "sidebar");
    } else {
      void applyVibrancy(false, "sidebar");
    }
    setRevision((n) => n + 1);
  }, [groups]);

  const changeGlassIntensity = React.useCallback((intensity: number) => {
    setGlassIntensity(clampUnit(intensity));
  }, []);

  const changeMaterial = React.useCallback((mat: VibrancyMaterial) => {
    setMaterial(mat);
  }, []);

  const changeMix = React.useCallback((m: number) => {
    setMix(m);
  }, []);

  React.useEffect(() => {
    if (!isBuzz) return;

    const root = document.documentElement;
    if (glassOn) {
      root.setAttribute("data-buzz-translucent", "");
      setTranslucencyStyleVars(root, glassIntensity, mix);
      return;
    }

    root.removeAttribute("data-buzz-translucent");
    clearTranslucencyStyleVars(root);
  }, [glassIntensity, glassOn, isBuzz, mix]);

  React.useEffect(() => {
    if (!isBuzz) return;
    void applyVibrancy(glassOn, material);
  }, [glassOn, isBuzz, material]);

  // Always tear translucency down on unmount so app teardown restores the
  // opaque window.
  React.useEffect(() => {
    return () => {
      const root = document.documentElement;
      root.removeAttribute("data-buzz-translucent");
      clearTranslucencyStyleVars(root);
      void applyVibrancy(false, "sidebar");
    };
  }, []);

  React.useEffect(() => {
    if (isBuzz) return;

    const root = document.documentElement;
    root.removeAttribute("data-buzz-translucent");
    clearTranslucencyStyleVars(root);
    setGlassIntensity(DEFAULT_GLASS_INTENSITY);
    void applyVibrancy(false, "sidebar");
  }, [isBuzz]);

  const copyCss = React.useCallback(async () => {
    const variant = isDark
      ? ":root[data-buzz-sidebar].dark"
      : ":root[data-buzz-sidebar]";
    const lines = groups.flatMap((group) =>
      group.knobs.map((knob) => {
        const hex = readKnobHex(knob);
        const value = knobStoreValue(knob, hex);
        return `  --${knob.cssVar}: ${value}; /* ${hex} */`;
      }),
    );
    const clampedGlass = clampUnit(glassIntensity);
    const clampedMix = clampUnit(mix);
    const glassAmount = (1 - clampedMix) * clampedGlass;
    const translucencyCss = `:root[data-buzz-translucent] {\n  --buzz-glass-intensity: ${clampedGlass};\n  --buzz-translucency-mix: ${clampedMix};\n  --buzz-translucency-gradient-alpha: ${formatPercent(clampedMix)};\n  --buzz-translucency-wash-alpha: ${formatPercent(glassAmount * FROST_WASH_ALPHA)};\n  /* macOS vibrancy material: ${material}; glass enabled: ${glassOn} */\n}`;
    const css = `${variant} {\n${lines.join("\n")}\n}\n\n${translucencyCss}`;
    try {
      await navigator.clipboard.writeText(css);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard blocked - surface the CSS so it is still grabbable.
      window.prompt("Copy the tuned Buzz CSS:", css);
    }
  }, [glassIntensity, glassOn, groups, isDark, material, mix]);

  if (!isBuzz) return null;

  if (!open) {
    return (
      <button
        type="button"
        aria-label="Open Buzz Theme Lab"
        data-testid="buzz-theme-lab-toggle"
        onClick={() => setOpen(true)}
        className="fixed bottom-4 right-4 z-50 flex h-10 w-10 items-center justify-center rounded-full border border-border bg-background text-foreground shadow-lg transition-colors hover:bg-accent"
      >
        <Paintbrush className="h-4 w-4" />
      </button>
    );
  }

  return (
    <div
      data-testid="buzz-theme-lab"
      className="fixed bottom-4 right-4 z-50 flex max-h-[80vh] w-72 flex-col overflow-hidden rounded-lg border border-border bg-background text-foreground shadow-xl"
    >
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <Paintbrush className="h-4 w-4" />
          Buzz Theme Lab
          <span className="text-2xs font-normal text-muted-foreground">
            {isDark ? "dark" : "light"}
          </span>
        </div>
        <button
          type="button"
          aria-label="Close Buzz Theme Lab"
          onClick={() => setOpen(false)}
          className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto px-3 py-2">
        {groups.map((group) => (
          <div key={group.title} className="mb-3 last:mb-0">
            <div className="mb-1 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              {group.title}
            </div>
            <div className="flex flex-col gap-1.5">
              {group.knobs.map((knob) => (
                <KnobRow
                  key={knob.cssVar}
                  knob={knob}
                  revision={revision}
                  onChange={(hex) => setVar(knob, hex)}
                />
              ))}
            </div>
          </div>
        ))}

        <div className="mb-1 mt-1 border-t border-border pt-3">
          <div className="mb-1 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            Translucency
          </div>
          <div className="flex flex-col gap-1.5 text-xs">
            <label className="flex items-center gap-2">
              <span className="flex-1">Glass intensity</span>
              <input
                type="range"
                min={0}
                max={1}
                step={0.02}
                value={glassIntensity}
                onChange={(e) => changeGlassIntensity(Number(e.target.value))}
                className="w-24"
                aria-label="Glass intensity"
                data-testid="buzz-glass-intensity-slider"
              />
              <span className="w-8 text-right font-mono text-2xs">
                {Math.round(glassIntensity * 100)}
              </span>
            </label>
            <label className="flex items-center gap-2">
              <span className="flex-1">Gradient opacity</span>
              <input
                type="range"
                min={0}
                max={1}
                step={0.02}
                value={mix}
                onChange={(e) => changeMix(Number(e.target.value))}
                disabled={!glassOn}
                className="w-24 disabled:opacity-50"
                aria-label="Gradient opacity"
              />
              <span className="w-8 text-right font-mono text-2xs">
                {Math.round(mix * 100)}
              </span>
            </label>
            <label className="flex items-center gap-2">
              <span className="flex-1">Vibrancy material</span>
              <select
                value={material}
                onChange={(e) =>
                  changeMaterial(e.target.value as VibrancyMaterial)
                }
                disabled={!glassOn}
                className="rounded border border-border bg-transparent px-1 py-0.5 text-2xs disabled:opacity-50"
              >
                {VIBRANCY_MATERIALS.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
            <p className="text-2xs leading-tight text-muted-foreground">
              macOS only. Glass intensity is the frosted effect amount; 0 turns
              it off. Gradient opacity crossfades the whole left nav toward the
              solid Buzz gradient. Rail, gutters, top chrome, header, and footer
              share these settings so the nav reads as one pane.
            </p>
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 border-t border-border px-3 py-2">
        <Button
          size="sm"
          variant="secondary"
          className="flex-1"
          onClick={copyCss}
        >
          {copied ? "Copied!" : "Copy CSS"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Reset overrides"
          onClick={reset}
        >
          <RotateCcw className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function KnobRow({
  knob,
  revision,
  onChange,
}: {
  knob: Knob;
  revision: number;
  onChange: (hex: string) => void;
}) {
  const [hex, setHex] = React.useState(() => readKnobHex(knob));

  // Re-sync from the live document when a reset bumps the revision. `knob` is
  // stable for a mounted row (keyed by cssVar), so revision is the only trigger.
  // biome-ignore lint/correctness/useExhaustiveDependencies: resync only on reset
  React.useEffect(() => {
    setHex(readKnobHex(knob));
  }, [revision]);

  const apply = (next: string) => {
    setHex(next);
    onChange(next);
  };

  return (
    <label className="flex items-center gap-2 text-xs">
      <input
        type="color"
        value={hex}
        onChange={(e) => apply(e.target.value)}
        className="h-6 w-6 shrink-0 cursor-pointer rounded border border-border bg-transparent p-0"
        aria-label={knob.label}
      />
      <span className="flex-1 truncate">{knob.label}</span>
      <input
        type="text"
        value={hex}
        onChange={(e) => {
          const v = e.target.value;
          setHex(v);
          if (/^#[0-9a-f]{6}$/i.test(v)) onChange(v);
        }}
        className="w-[4.5rem] rounded border border-border bg-transparent px-1 py-0.5 text-2xs font-mono uppercase"
        spellCheck={false}
      />
    </label>
  );
}
