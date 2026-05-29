import * as React from "react";

const STORAGE_KEY = "sprout:last-runtime";
const LEGACY_STORAGE_KEY = "sprout:last-runtime-provider";

export function useLastRuntime(): {
  lastRuntimeId: string | null;
  setLastRuntime: (id: string) => void;
} {
  const [lastRuntimeId, setLastRuntimeId] = React.useState<string | null>(
    () => {
      try {
        return (
          localStorage.getItem(STORAGE_KEY) ??
          localStorage.getItem(LEGACY_STORAGE_KEY)
        );
      } catch {
        return null;
      }
    },
  );

  const setLastRuntime = React.useCallback((id: string) => {
    setLastRuntimeId(id);
    try {
      localStorage.setItem(STORAGE_KEY, id);
      localStorage.removeItem(LEGACY_STORAGE_KEY);
    } catch {
      // localStorage full — ignore
    }
  }, []);

  return { lastRuntimeId, setLastRuntime };
}
