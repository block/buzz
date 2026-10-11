import * as React from "react";

import {
  getRemoteAccess,
  setRemoteAccessEnabled,
  setRemoteTerminals,
  subscribeRemoteAccess,
} from "@/features/bridge/remoteAccessStore";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function RemoteAccessSettingsCard() {
  const status = React.useSyncExternalStore(subscribeRemoteAccess, getRemoteAccess);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
    setBusy(false);
  };
  if (!status) return null;
  const canEnable = status.tailscaleUp || status.enabled;
  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup data-testid="remote-access-card" title="Remote access">
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-access-switch">
              Allow remote access
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              Lets your own web app reach this Mac through your Tailscale network. Nothing is exposed to the LAN or the internet, and Tailscale itself is never started or changed.
              {!status.tailscaleUp && !status.enabled ? ` ${status.reason ?? "Tailscale isn't running on this Mac."}` : ""}
            </p>
          </div>
          <Switch
            checked={status.enabled}
            data-testid="remote-access-toggle"
            disabled={busy || !canEnable}
            id="remote-access-switch"
            onCheckedChange={(on) => void run(() => setRemoteAccessEnabled(on))}
          />
        </SettingsOptionRow>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-terminals-switch">
              Allow remote terminals
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              Lets your web app open a shell on this Mac. Every terminal needs a fresh approval in your signer. Turning this off closes open remote terminals.
              {status.enabled ? ` ${status.openTerminals} open now.` : " Requires remote access."}
            </p>
          </div>
          <Switch
            checked={status.terminals}
            data-testid="remote-terminals-toggle"
            disabled={busy || !status.enabled}
            id="remote-terminals-switch"
            onCheckedChange={(on) => void run(() => setRemoteTerminals(on))}
          />
        </SettingsOptionRow>
      </SettingsOptionGroup>
      {error && <p className="text-sm text-destructive">{error}</p>}
      {status.enabled && status.url && (
        <p className="text-xs text-muted-foreground/70">Serving on {status.url}</p>
      )}
    </div>
  );
}
