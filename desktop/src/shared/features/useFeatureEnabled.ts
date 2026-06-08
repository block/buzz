import { useSyncExternalStore, useCallback, useEffect } from "react";
import { getFeature } from "./manifest";
import { resolveEnabled } from "./resolveEnabled";
import { getOverrides, setOverride, OVERRIDES_KEY } from "./store";

// ---------------------------------------------------------------------------
// Reactive store — components re-render when overrides change
// ---------------------------------------------------------------------------

type Listener = () => void;
const listeners = new Set<Listener>();

function subscribe(listener: Listener): () => void {
  listeners.add(listener);

  // Cross-window sync: another window writing the overrides key in
  // localStorage fires a "storage" event in this window. Mirror the
  // pattern used by useChannelSections / useChannelStars / useChannelMutes /
  // useThreadFollows.
  const handleStorage = (event: StorageEvent) => {
    if (event.key === OVERRIDES_KEY) {
      emitChange();
    }
  };
  window.addEventListener("storage", handleStorage);

  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", handleStorage);
  };
}

/** Notify all subscribers that feature state changed */
export function emitChange(): void {
  // Invalidate cached snapshot
  cachedRaw = null;
  cachedParsed = null;
  for (const listener of listeners) listener();
}

// ---------------------------------------------------------------------------
// Cached snapshot
//
// useSyncExternalStore requires getSnapshot to return a referentially stable
// value when nothing has changed. Returning `JSON.stringify(getOverrides())`
// fresh on every render would produce a new string each tick → infinite
// re-render. We cache the serialized form and only mint a new parsed object
// when the serialized form changes.
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

/**
 * Server-side snapshot for useSyncExternalStore.
 *
 * Sprout is a Tauri desktop app and does not currently SSR. Returning an
 * explicit empty-state snapshot is safer than omitting this argument: under
 * any future test harness or SSR experiment, the hook returns "no overrides"
 * instead of throwing.
 */
const getServerSnapshot = (): string => "{}";

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
  useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
  return getParsedSnapshot();
}

/**
 * Returns whether a feature is enabled given its tier and user overrides.
 *
 * - stable: always true
 * - preview: true only if user opted in
 * - unknown id: fail-open (returns true). Manifest membership signals "this
 *   needs gating"; absence means "just render it." A stray `<FeatureGate>`
 *   pointing at a removed id should not hide UI. Dev mode still logs a
 *   `console.warn` so typos surface during development.
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
    return true;
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

/**
 * Fires a sonner toast.warning when a preview feature is currently disabled.
 *
 * Usage: drop in at the top of a route component to give users hitting a
 * direct link to a disabled preview feature a hint about how to surface it.
 *
 *   function PulseRouteComponent() {
 *     usePreviewFeatureWarning("pulse");
 *     return <PulseScreen />;
 *   }
 *
 * Stays a no-op for stable features and for preview features that ARE enabled.
 */
export function usePreviewFeatureWarning(featureId: string): void {
  const enabled = useFeatureEnabled(featureId);
  const feature = getFeature(featureId);

  useEffect(() => {
    if (feature?.tier !== "preview" || enabled) return;
    let cancelled = false;
    void import("sonner").then(({ toast }) => {
      if (cancelled) return;
      toast.warning(
        `${feature.name} is a preview feature. Enable it in Settings → Experiments to surface it in your sidebar.`,
      );
    });
    return () => {
      cancelled = true;
    };
  }, [feature, enabled]);
}

// Re-export for consumers that imported from here
export { resolveEnabled } from "./resolveEnabled";
