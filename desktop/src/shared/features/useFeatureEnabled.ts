import { useSyncExternalStore, useCallback, useEffect } from "react";
import { getFeature } from "./manifest";
import { resolveEnabled } from "./resolveEnabled";
import { getOverrides, setOverride, OVERRIDES_KEY } from "./store";

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
  cachedRaw = null;
  cachedParsed = null;
  for (const listener of listeners) listener();
}

// useSyncExternalStore requires getSnapshot to return a referentially stable
// value when nothing has changed. Returning `JSON.stringify(getOverrides())`
// fresh on every render would produce a new string each tick → infinite
// re-render. We cache the serialized form and only mint a new parsed object
// when the serialized form changes.

let cachedRaw: string | null = null;
let cachedParsed: Record<string, boolean> | null = null;
const emptyOverrides = Object.freeze({}) as Record<string, boolean>;

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
 * Buzz is a Tauri desktop app and does not currently SSR. Returning an
 * explicit empty-state snapshot is safer than omitting this argument: under
 * any future test harness or SSR experiment, the hook returns "no overrides"
 * instead of throwing.
 */
const getServerSnapshot = (): string => "{}";

function getParsedSnapshot(): Record<string, boolean> {
  getSnapshot();
  return cachedParsed ?? emptyOverrides;
}

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
 * Returns whether a feature is enabled.
 *
 * The manifest (`preview-features.json`) lists ONLY preview features:
 *
 * - in manifest (preview): true only if the user opted in via overrides
 * - NOT in manifest (stable): always true (fail-open)
 *
 * Membership in the manifest signals "this needs gating"; absence means
 * "just render it." A stray `<FeatureGate feature="removed-id">` will never
 * hide UI.
 */
export function useFeatureEnabled(featureId: string): boolean {
  const overrides = useFeatureSnapshot();

  const feature = getFeature(featureId);
  if (!feature) {
    if (import.meta.env.DEV) {
      console.warn(
        `[FeatureFlags] Unknown feature id: "${featureId}". Check preview-features.json.`,
      );
    }
    return true;
  }

  return resolveEnabled(featureId, overrides);
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
    // No-op for stable features (not in manifest) and preview features
    // that ARE enabled. Manifest membership = preview by definition.
    if (!feature || enabled) return;
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

export { resolveEnabled } from "./resolveEnabled";
