import { useSyncExternalStore, useCallback } from "react";
import { getFeature } from "./manifest";
import { resolveEnabled } from "./resolveEnabled";
import { getOverrides, setOverride } from "./store";

// ---------------------------------------------------------------------------
// Reactive store — components re-render when overrides change
// ---------------------------------------------------------------------------

type Listener = () => void;
const listeners = new Set<Listener>();

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Notify all subscribers that feature state changed */
export function emitChange(): void {
  // Invalidate cached snapshot
  cachedRaw = null;
  cachedParsed = null;
  for (const listener of listeners) listener();
}

// ---------------------------------------------------------------------------
// Cached snapshot — avoids JSON.parse on every render per hook instance
// ---------------------------------------------------------------------------

let cachedRaw: string | null = null;
let cachedParsed: Record<string, boolean> | null = null;

function getSnapshot(): string {
  const raw = JSON.stringify(getOverrides());
  if (raw !== cachedRaw) {
    cachedRaw = raw;
    cachedParsed = JSON.parse(raw) as Record<string, boolean>;
  }
  return raw;
}

function getParsedSnapshot(): Record<string, boolean> {
  // Ensure snapshot is fresh
  getSnapshot();
  return cachedParsed!;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Returns the current parsed feature overrides.
 * Reactive — re-renders when any feature toggle changes.
 * Use this in components that need the full state (e.g. SettingsView filtering).
 */
export function useFeatureSnapshot(): Record<string, boolean> {
  useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return getParsedSnapshot();
}

/**
 * Returns whether a feature is enabled given its tier and user overrides.
 *
 * - stable: always true
 * - preview: true only if user opted in
 * - unstable: true only if user opted in
 */
export function useFeatureEnabled(featureId: string): boolean {
  const overrides = useFeatureSnapshot();

  const feature = getFeature(featureId);
  if (!feature) {
    if (import.meta.env.DEV) {
      console.warn(
        `[FeatureFlags] Unknown feature id: "${featureId}". Check features.json.`,
      );
    }
    return false;
  }

  return resolveEnabled(feature.tier, featureId, overrides);
}

/**
 * Hook to toggle a feature override. Returns [enabled, toggle].
 */
export function useFeatureToggle(
  featureId: string,
): [boolean, (enabled: boolean) => void] {
  const enabled = useFeatureEnabled(featureId);

  const toggle = useCallback(
    (value: boolean) => {
      setOverride(featureId, value);
      emitChange();
    },
    [featureId],
  );

  return [enabled, toggle];
}

// Re-export for consumers that imported from here
export { resolveEnabled } from "./resolveEnabled";
