import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { WebRemoteAgentManagement, WebRemoteTerminal } from "@/features/bridge/ui/WebRemoteSettings";

import {
  getRemoteAccess,
  getRemotePrefs,
  setAgentManagement,
  setSignatureTtl,
  setTerminalSignatureTtl,
  setTerminalsAllowed,
  subscribeRemoteAccess,
} from "@/features/bridge/remoteAccessStore";
import { SIG_TTL_OPTIONS_S } from "@/features/bridge/protocol";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export const REMOTE_AGENT_MGMT_LABEL = "Allow remote agent management";
export const REMOTE_AGENT_MGMT_DESC =
  "Lets your signed-in devices list your agents and start, stop, message and create agents on this computer, over your tailnet when reachable or via the relay otherwise. On a direct (tailnet) connection it also allows full agent management: create, rename, change provider, model, persona and working directory, and delete. Does not allow terminals or file access. A signed connection stays valid for the duration you choose below, then your signer is asked again.";
export const REMOTE_TERMINAL_LABEL = "Allow remote terminal";
export const REMOTE_TERMINAL_DESC =
  "Lets your signed-in devices open a shell, or attach to a herdr session, on this computer over your tailnet only. A signed terminal stays valid for the duration you choose below; after that, the next launch asks your signer again. Off by default. Turning off closes open terminals.";

export const ttlLabel = (s: number) => (s < 3600 ? `${s / 60} minutes` : s === 3600 ? "1 hour" : `${s / 3600} hours`);

function useRemote() {
  const status = React.useSyncExternalStore(subscribeRemoteAccess, getRemoteAccess);
  const [, bump] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => subscribeRemoteAccess(bump), []);
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
  return { status, error, busy, run, prefs: getRemotePrefs() };
}

/** Settings → Agents. */
export function RemoteAgentManagementCard() {
  if (!isTauri()) return <WebRemoteAgentManagement />;
  return <DesktopRemoteAgentManagementCard />;
}
function DesktopRemoteAgentManagementCard() {
  const { status, error, busy, run, prefs } = useRemote();
  if (!status) return null;
  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup data-testid="remote-access-card" title="Remote agent management">
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-access-switch">
              {REMOTE_AGENT_MGMT_LABEL}
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              {REMOTE_AGENT_MGMT_DESC}
              {prefs.agentManagement && !status.tailscaleUp ? " Tailscale isn't running here, so only the relay path is available." : ""}
            </p>
          </div>
          <Switch
            checked={prefs.agentManagement}
            data-testid="remote-access-toggle"
            disabled={busy}
            id="remote-access-switch"
            onCheckedChange={(on) => void run(() => setAgentManagement(on))}
          />
        </SettingsOptionRow>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-sig-ttl">
              Signed connection lasts
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              After this, a device must sign a fresh challenge with your signer before it can manage agents on a direct connection.
            </p>
          </div>
          <select
            className="rounded-md border bg-background px-2 py-1 text-sm"
            data-testid="remote-sig-ttl"
            disabled={!prefs.agentManagement}
            id="remote-sig-ttl"
            onChange={(e) => setSignatureTtl(Number(e.target.value))}
            value={prefs.sigTtlS}
          >
            {SIG_TTL_OPTIONS_S.map((s) => (
              <option key={s} value={s}>
                {ttlLabel(s)}
              </option>
            ))}
          </select>
        </SettingsOptionRow>
      </SettingsOptionGroup>
      {error && <p className="text-sm text-destructive">{error}</p>}
      {status.enabled && status.url && <p className="text-xs text-muted-foreground/70">Direct connection on {status.url}</p>}
    </div>
  );
}

/** Settings → Buzz Term. */
export function RemoteTerminalCard() {
  if (!isTauri()) return <WebRemoteTerminal />;
  return <DesktopRemoteTerminalCard />;
}
function DesktopRemoteTerminalCard() {
  const { status, error, busy, run, prefs } = useRemote();
  if (!status) return null;
  const canEnable = status.tailscaleUp || status.terminals;
  return (
    <div className="min-w-0 space-y-3">
      <SettingsOptionGroup data-testid="remote-terminal-card" title="Remote terminal">
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-terminals-switch">
              {REMOTE_TERMINAL_LABEL}
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              {REMOTE_TERMINAL_DESC}
              {status.terminals ? ` ${status.openTerminals} open now.` : ""}
              {!canEnable ? ` ${status.reason ?? "Tailscale isn't running on this computer."}` : ""}
            </p>
          </div>
          <Switch
            checked={status.terminals}
            data-testid="remote-terminals-toggle"
            disabled={busy || !canEnable}
            id="remote-terminals-switch"
            onCheckedChange={(on) => void run(() => setTerminalsAllowed(on))}
          />
        </SettingsOptionRow>
        <SettingsOptionRow>
          <div className="min-w-0">
            <label className="text-sm font-medium" htmlFor="remote-term-sig-ttl">
              Signed connection lasts
            </label>
            <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
              After this, a device must sign a fresh challenge with your signer before it can open another terminal.
            </p>
          </div>
          <select
            className="rounded-md border bg-background px-2 py-1 text-sm"
            data-testid="remote-term-sig-ttl"
            disabled={!status.terminals}
            id="remote-term-sig-ttl"
            onChange={(e) => setTerminalSignatureTtl(Number(e.target.value))}
            value={prefs.termSigTtlS}
          >
            {SIG_TTL_OPTIONS_S.map((s) => (
              <option key={s} value={s}>
                {ttlLabel(s)}
              </option>
            ))}
          </select>
        </SettingsOptionRow>
      </SettingsOptionGroup>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}
