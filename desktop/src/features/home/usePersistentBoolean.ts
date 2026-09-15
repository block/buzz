import * as React from "react";

/**
 * A boolean state hook that persists its value to `localStorage`.
 *
 * - On first mount, the stored value is read and used as the initial state.
 *   If no stored value exists (or parsing fails), `defaultValue` is used.
 * - Whenever the value changes, the new value is written back to storage.
 *
 * This follows the same localStorage pattern as `useFeedItemState` — a
 * versioned key prefix, defensive `try/catch` around storage access, and
 * SSR-safe guards (`typeof window`).
 */
export function usePersistentBoolean(
  storageKey: string,
  defaultValue: boolean,
): [boolean, React.Dispatch<React.SetStateAction<boolean>>] {
  const [value, setValue] = React.useState<boolean>(() => {
    if (typeof window === "undefined") return defaultValue;
    try {
      const raw = window.localStorage.getItem(storageKey);
      if (raw === null) return defaultValue;
      return raw === "true";
    } catch {
      return defaultValue;
    }
  });

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(storageKey, String(value));
    } catch {
      // Storage may be unavailable (private browsing, quota, etc.) —
      // silently degrade to in-memory-only persistence.
    }
  }, [storageKey, value]);

  return [value, setValue];
}
