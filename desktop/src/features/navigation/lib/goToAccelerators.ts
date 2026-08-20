/**
 * Pure resolution logic for the ⌘G "Go to" palette. No React, no icons, no
 * feature-manifest coupling — safe to unit test in isolation. The palette UI
 * and the `useGoToPalette` hook layer icons, navigation, and state on top.
 */

export type GoToAcceleratorItem = {
  label: string;
  /** Stable, hand-assigned ⌘/Ctrl+letter accelerator (single letter). */
  mnemonic: string;
  /** Extra terms matched by the filter alongside the label. */
  keywords?: readonly string[];
  /** Preview-feature id gating this item; omit for always-on areas. */
  feature?: string;
};

/**
 * Keep only areas whose gating feature is enabled. Always-on areas (no
 * `feature`) are retained. `isFeatureEnabled` mirrors `useFeatureEnabled`
 * (stable/unknown features fail open).
 */
export function selectEnabledDestinations<T extends GoToAcceleratorItem>(
  items: readonly T[],
  isFeatureEnabled: (feature: string) => boolean,
): T[] {
  return items.filter(
    (item) => item.feature == null || isFeatureEnabled(item.feature),
  );
}

/** Case-insensitive substring match over label + keywords. */
export function filterGoToDestinations<T extends GoToAcceleratorItem>(
  items: readonly T[],
  query: string,
): T[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) {
    return [...items];
  }

  return items.filter((item) => {
    if (item.label.toLowerCase().includes(needle)) {
      return true;
    }
    return (item.keywords ?? []).some((keyword) =>
      keyword.toLowerCase().includes(needle),
    );
  });
}

/**
 * Resolve a bare digit ("1"–"9") to the destination at that 1-based position
 * in the currently visible list. Returns null for non-digits or out-of-range.
 */
export function resolveDigitAccelerator<T extends GoToAcceleratorItem>(
  visible: readonly T[],
  key: string,
): T | null {
  if (!/^[1-9]$/.test(key)) {
    return null;
  }
  return visible[Number.parseInt(key, 10) - 1] ?? null;
}

/**
 * Resolve a ⌘/Ctrl+letter chord to the destination with that mnemonic. Matches
 * against the full enabled list (not the filtered view) so the chord is a
 * stable global accelerator regardless of the current query.
 */
export function resolveMnemonicAccelerator<T extends GoToAcceleratorItem>(
  enabled: readonly T[],
  letter: string,
): T | null {
  const normalized = letter.trim().toLowerCase();
  if (normalized.length !== 1) {
    return null;
  }
  return (
    enabled.find((item) => item.mnemonic.toLowerCase() === normalized) ?? null
  );
}
