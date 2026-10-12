import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";

import {
  getSignerToastPrefs,
  setSignerToastPrefs,
  subscribeSignerToastPrefs,
  type SignerToastMode,
  type SignerToastPosition,
} from "@/platform/web/signer/signerToastPrefs";
import { SegmentedControl } from "@/shared/ui/segmented-control";

import { SettingsOptionRow } from "./SettingsOptionGroup";

const MODE_OPTIONS = [
  { value: "always", label: "Always" },
  { value: "problems", label: "Problems" },
  { value: "off", label: "Off" },
] as const satisfies readonly { value: SignerToastMode; label: string }[];
const POSITION_OPTIONS = [
  { value: "top", label: "Top" },
  { value: "bottom", label: "Bottom" },
] as const satisfies readonly { value: SignerToastPosition; label: string }[];

const MODE_COPY: Record<SignerToastMode, string> = {
  always: "Show “Waiting for your signer…” whenever a request takes more than a moment.",
  problems: "Only when a request has waited over 5 seconds or the signer needs you to approve.",
  off: "Hidden. Still shown when the signer needs your approval to finish an action.",
};

/** Web only: the vanilla-DOM signer status toast (platform/web/signer/signerStatus.ts). */
export function SignerToastSetting() {
  const prefs = React.useSyncExternalStore(subscribeSignerToastPrefs, getSignerToastPrefs, getSignerToastPrefs);
  if (isTauri()) return null;
  return (
    <>
      <SettingsOptionRow data-testid="signer-toast-mode-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">Show signer messages</p>
          <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
            {MODE_COPY[prefs.mode]}
          </p>
        </div>
        <SegmentedControl
          legend="Show signer messages"
          onValueChange={(mode) => setSignerToastPrefs({ mode })}
          optionTestIdPrefix="signer-toast-mode"
          options={MODE_OPTIONS}
          testId="signer-toast-mode-control"
          value={prefs.mode}
        />
      </SettingsOptionRow>
      <SettingsOptionRow data-testid="signer-toast-position-row">
        <div className="min-w-0">
          <p className="text-sm font-medium">Signer message position</p>
          <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
            Bottom sits above the tab bar, composer and terminal keys.
          </p>
        </div>
        <SegmentedControl
          legend="Signer message position"
          onValueChange={(position) => setSignerToastPrefs({ position })}
          optionTestIdPrefix="signer-toast-position"
          options={POSITION_OPTIONS}
          size="compact"
          testId="signer-toast-position-control"
          value={prefs.position}
        />
      </SettingsOptionRow>
    </>
  );
}
