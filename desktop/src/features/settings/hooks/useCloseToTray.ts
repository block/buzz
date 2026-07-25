import * as React from "react";

import {
  applyCloseToTray,
  getCloseToTrayPref,
  setCloseToTrayPref,
} from "../lib/closeToTray";

/**
 * Reads/writes the "Keep Buzz running in the tray" preference. Persists to
 * localStorage and pushes the value to the Tauri backend on every change.
 */
export function useCloseToTray() {
  const [enabled, setEnabledState] = React.useState(getCloseToTrayPref);
  const [isUpdating, setIsUpdating] = React.useState(false);

  const setEnabled = React.useCallback(async (next: boolean) => {
    const previous = getCloseToTrayPref();
    setIsUpdating(true);
    const applied = await applyCloseToTray(next);
    if (applied) {
      setEnabledState(next);
      setCloseToTrayPref(next);
    } else {
      setEnabledState(previous);
    }
    setIsUpdating(false);
  }, []);

  return { enabled, isUpdating, setEnabled };
}
