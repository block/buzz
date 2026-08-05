/**
 * Shared device-level scale preference: snap to presets, persist, notify,
 * optional CSS custom property on `<html>`.
 */

export type ScalePreferenceOptions = {
  storageKey: string;
  cssVar?: string;
  defaultScale?: number;
  presets?: readonly number[];
  /**
   * When true (default), the CSS variable is removed at the default scale so
   * the stylesheet can fall back with `var(--x, 1)`. When false, the variable
   * is always written — preferred for `calc(... * var(--x))` consumers that
   * should never depend on a missing custom property.
   */
  clearCssVarAtDefault?: boolean;
};

export type ScalePreference = {
  STORAGE_KEY: string;
  CSS_VAR: string | null;
  DEFAULT: number;
  MIN: number;
  MAX: number;
  PRESETS: readonly number[];
  normalize: (scale: number) => number;
  formatPercent: (scale: number) => string;
  presetIndex: (scale: number) => number;
  get: () => number;
  set: (scale: number) => void;
  applyCurrent: () => void;
  subscribe: (listener: () => void) => () => void;
  getSnapshot: () => number;
  getServerSnapshot: () => number;
};

/** Fallback ladder when a store does not pass `presets` (matches Appearance). */
const DEFAULT_PRESETS = [
  0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2, 2.5, 3, 4, 5,
] as const;

export function createScalePreference(
  options: ScalePreferenceOptions,
): ScalePreference {
  const DEFAULT = options.defaultScale ?? 1;
  const PRESETS = options.presets ?? DEFAULT_PRESETS;
  const MIN = PRESETS[0] ?? 0.75;
  const MAX = PRESETS[PRESETS.length - 1] ?? 1.5;
  const STORAGE_KEY = options.storageKey;
  const CSS_VAR = options.cssVar ?? null;
  const clearCssVarAtDefault = options.clearCssVarAtDefault ?? true;
  const listeners = new Set<() => void>();

  function normalize(scale: number): number {
    if (!Number.isFinite(scale)) {
      return DEFAULT;
    }
    let best = PRESETS[0] ?? DEFAULT;
    let bestDistance = Number.POSITIVE_INFINITY;
    for (const preset of PRESETS) {
      const distance = Math.abs(preset - scale);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = preset;
      }
    }
    return best;
  }

  function readStored(): number {
    try {
      const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
      if (!raw) {
        return DEFAULT;
      }
      return normalize(Number.parseFloat(raw));
    } catch {
      return DEFAULT;
    }
  }

  let value = readStored();

  function applyToDocument(scale: number): void {
    if (!CSS_VAR || typeof document === "undefined") {
      return;
    }
    if (scale === DEFAULT && clearCssVarAtDefault) {
      document.documentElement.style.removeProperty(CSS_VAR);
      return;
    }
    document.documentElement.style.setProperty(CSS_VAR, String(scale));
  }

  function persist(scale: number): void {
    try {
      if (scale === DEFAULT) {
        globalThis.localStorage?.removeItem(STORAGE_KEY);
      } else {
        globalThis.localStorage?.setItem(STORAGE_KEY, String(scale));
      }
    } catch {
      // Persistence is best-effort.
    }
  }

  function notify(): void {
    for (const listener of listeners) {
      listener();
    }
  }

  function set(scale: number): void {
    const next = normalize(scale);
    if (next === value) {
      applyToDocument(next);
      return;
    }
    value = next;
    applyToDocument(next);
    persist(next);
    notify();
  }

  // Apply restored value once the document exists (module may load early).
  applyToDocument(value);

  return {
    STORAGE_KEY,
    CSS_VAR,
    DEFAULT,
    MIN,
    MAX,
    PRESETS,
    normalize,
    formatPercent(scale: number) {
      return `${Math.round(normalize(scale) * 100)}%`;
    },
    presetIndex(scale: number) {
      const target = normalize(scale);
      const index = PRESETS.indexOf(target);
      return index >= 0 ? index : 0;
    },
    get() {
      return value;
    },
    set,
    applyCurrent() {
      applyToDocument(value);
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot() {
      return value;
    },
    getServerSnapshot() {
      return DEFAULT;
    },
  };
}
