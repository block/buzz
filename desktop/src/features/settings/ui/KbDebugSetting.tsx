import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";

import { isKbDebugOn, setKbDebug, subscribeKbDebug } from "@/platform/web/kbDebug/kbDebug";
import { Switch } from "@/shared/ui/switch";

import { SettingsOptionRow } from "./SettingsOptionGroup";

/** Web only: the keyboard / viewport / Buzz Term latency diagnostics overlay (also ?kbdebug=1). */
export function KbDebugSetting() {
  const on = React.useSyncExternalStore(subscribeKbDebug, isKbDebugOn, isKbDebugOn);
  if (isTauri()) return null;
  return (
    <SettingsOptionRow data-testid="kbdebug-row">
      <div className="min-w-0">
        <label className="text-sm font-medium" htmlFor="kbdebug-switch">
          Diagnostics overlay
        </label>
        <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
          Shows live screen, keyboard and Buzz Term timing numbers for troubleshooting. Saved on this device.
        </p>
      </div>
      <Switch checked={on} data-testid="kbdebug-toggle" id="kbdebug-switch" onCheckedChange={(v) => setKbDebug(v)} />
    </SettingsOptionRow>
  );
}
