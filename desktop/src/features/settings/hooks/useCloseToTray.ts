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

  const setEnabled = React.useCallback((next: boolean) => {
    setEnabledState(next);
    setCloseToTrayPref(next);
    void applyCloseToTray(next);
  }, []);

  return { enabled, setEnabled };
}
