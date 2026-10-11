// Web/mobile Settings: "Allow remote agent management" and "Allow remote terminal" with a host picker under each switch.
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "@/features/settings/ui/SettingsOptionGroup";
import {
  REMOTE_AGENT_MGMT_DESC,
  REMOTE_AGENT_MGMT_LABEL,
  REMOTE_TERMINAL_DESC,
  REMOTE_TERMINAL_LABEL,
} from "@/features/settings/ui/RemoteAccessSettingsCard";
import {
  type Feature,
  type Reach,
  type RemoteHost,
  getWebRemotePrefs,
  hostsFrom,
  probeHost,
  setWebRemotePrefs,
  subscribeWebRemotePrefs,
  transportFor,
} from "../remoteHosts";
import { useBridgeAgents } from "../useBridgeAgents";

export function useRemoteHosts() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const prefs = React.useSyncExternalStore(subscribeWebRemotePrefs, getWebRemotePrefs);
  const hosts = React.useMemo(() => hostsFrom(b), [b.heartbeat, b.lastHeardTs]); // eslint-disable-line react-hooks/exhaustive-deps
  const [reach, setReach] = React.useState<Record<string, Reach>>({});
  React.useEffect(() => {
    let dead = false;
    const run = async () => {
      for (const h of hosts) {
        if (!h.hasDirect || h.stale) {
          setReach((r) => ({ ...r, [h.id]: "unreachable" }));
          continue;
        }
        setReach((r) => ({ ...r, [h.id]: r[h.id] === "reachable" ? "reachable" : "checking" }));
        const ok = await probeHost(h.url);
        if (!dead) setReach((r) => ({ ...r, [h.id]: ok ? "reachable" : "unreachable" }));
      }
    };
    void run();
    const t = window.setInterval(run, 20_000);
    return () => {
      dead = true;
      window.clearInterval(t);
    };
  }, [hosts]);
  const reachOf = (h: RemoteHost): Reach => reach[h.id] ?? "unknown";
  const selected = (f: Feature) => {
    const id = f === "agents" ? prefs.agentsHost : prefs.terminalHost;
    return hosts.find((h) => h.id === id && h.supports[f]);
  };
  // Terminal can't stay on without a reachable terminal host.
  React.useEffect(() => {
    if (!prefs.terminalOn) return;
    const h = selected("terminal");
    if (!h || reachOf(h) === "unreachable") setWebRemotePrefs({ terminalOn: false });
  }); // eslint-disable-line react-hooks/exhaustive-deps
  return { b, hosts, prefs, reachOf, selected };
}

const label: Record<string, string> = { direct: "Direct", relay: "Via relay", unavailable: "Unavailable" };

function HostList(props: { feature: Feature; hosts: RemoteHost[]; reachOf: (h: RemoteHost) => Reach; prefs: ReturnType<typeof getWebRemotePrefs> }) {
  const { feature, reachOf, prefs } = props;
  const hosts = props.hosts.filter((h) => h.supports[feature]);
  const current = feature === "agents" ? prefs.agentsHost : prefs.terminalHost;
  if (hosts.length === 0)
    return (
      <p className="text-sm text-muted-foreground/70" data-testid={`remote-hosts-none-${feature}`}>
        No computer has {feature === "agents" ? `“${REMOTE_AGENT_MGMT_LABEL}”` : `“${REMOTE_TERMINAL_LABEL}”`} turned on in its desktop settings.
      </p>
    );
  return (
    <ul className="space-y-1" data-testid={`remote-hosts-${feature}`}>
      {hosts.map((h) => {
        const t = transportFor(feature, h, reachOf(h));
        return (
          <li key={h.id} style={{ opacity: h.stale ? 0.5 : 1 }} data-testid="remote-host" data-stale={h.stale ? "1" : "0"}>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name={`host-${feature}`}
                checked={current === h.id}
                onChange={() => setWebRemotePrefs(feature === "agents" ? { agentsHost: h.id } : { terminalHost: h.id })}
              />
              <span>{h.name}</span>
              <span data-testid="remote-host-path" className="rounded border px-1 text-xs">
                {label[t]}
              </span>
              {h.stale && <span className="text-xs">not seen in over a day</span>}
            </label>
            {feature === "agents" && t === "relay" && (
              <p className="ml-6 text-xs text-muted-foreground/70" data-testid="remote-host-relay-note">
                Using the relay — slower, and without create/edit/delete. Connect to your tailnet for a direct connection.
              </p>
            )}
          </li>
        );
      })}
    </ul>
  );
}

export function WebRemoteAgentManagement() {
  const { hosts, prefs, reachOf } = useRemoteHosts();
  const available = hosts.some((h) => h.supports.agents);
  const turn = (on: boolean) => {
    const first = hosts.find((h) => h.supports.agents && !h.stale) ?? hosts.find((h) => h.supports.agents);
    setWebRemotePrefs({ agentsOn: on, ...(on && !prefs.agentsHost && first ? { agentsHost: first.id } : {}) });
  };
  return (
    <SettingsOptionGroup data-testid="web-remote-agents" title="Remote agent management">
      <SettingsOptionRow>
        <div className="min-w-0">
          <label className="text-sm font-medium" htmlFor="web-remote-agents-switch">
            {REMOTE_AGENT_MGMT_LABEL}
          </label>
          <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
            {REMOTE_AGENT_MGMT_DESC}
          </p>
        </div>
        <Switch
          checked={prefs.agentsOn && available}
          data-testid="web-remote-agents-toggle"
          disabled={!available}
          id="web-remote-agents-switch"
          onCheckedChange={turn}
        />
      </SettingsOptionRow>
      {prefs.agentsOn && available && (
        <div className="px-4 pb-3">
          <HostList feature="agents" hosts={hosts} prefs={prefs} reachOf={reachOf} />
        </div>
      )}
    </SettingsOptionGroup>
  );
}

export function WebRemoteTerminal() {
  const { hosts, prefs, reachOf } = useRemoteHosts();
  const capable = hosts.filter((h) => h.supports.terminal);
  const reachable = capable.filter((h) => reachOf(h) === "reachable");
  const canTurnOn = reachable.length > 0;
  return (
    <SettingsOptionGroup data-testid="web-remote-terminal" title="Remote terminal">
      <SettingsOptionRow>
        <div className="min-w-0">
          <label className="text-sm font-medium" htmlFor="web-remote-terminal-switch">
            {REMOTE_TERMINAL_LABEL}
          </label>
          <p className="text-sm font-normal text-muted-foreground/70" data-settings-subcopy>
            {REMOTE_TERMINAL_DESC}
            {!canTurnOn && " Terminals need a direct connection over your tailnet; no host is reachable right now."}
          </p>
        </div>
        <Switch
          checked={prefs.terminalOn && canTurnOn}
          data-testid="web-remote-terminal-toggle"
          disabled={!canTurnOn}
          id="web-remote-terminal-switch"
          onCheckedChange={(on) =>
            setWebRemotePrefs({ terminalOn: on, ...(on && !prefs.terminalHost ? { terminalHost: reachable[0]?.id ?? null } : {}) })
          }
        />
      </SettingsOptionRow>
      {prefs.terminalOn && canTurnOn && (
        <div className="px-4 pb-3">
          <HostList feature="terminal" hosts={hosts} prefs={prefs} reachOf={reachOf} />
        </div>
      )}
    </SettingsOptionGroup>
  );
}
