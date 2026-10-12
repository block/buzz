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
  getReachMap,
  getWebRemotePrefs,
  hostsFrom,
  probeHost,
  setWebRemotePrefs,
  subscribeReach,
  subscribeWebRemotePrefs,
  updateReach,
  transportFor,
} from "../remoteHosts";
import { useQueryClient } from "@tanstack/react-query";
import { directAgentsFor } from "../directAgents";
import { acquireProxyDirect } from "../remoteCmd";
import { useBridgeAgents } from "../useBridgeAgents";
import { useDirectState } from "./DirectAgentsPanel";

export function useRemoteHosts(opts?: { sync?: boolean }) {
  const pubkey = useIdentityQuery().data?.pubkey;
  return useRemoteHostsFor(useBridgeAgents(pubkey), opts);
}

/** Same as useRemoteHosts, for a caller that already holds the bridge state (avoids a second relay subscription). */
export function useRemoteHostsFor(b: ReturnType<typeof useBridgeAgents>, opts?: { sync?: boolean }) {
  const prefs = React.useSyncExternalStore(subscribeWebRemotePrefs, getWebRemotePrefs);
  const hosts = React.useMemo(() => hostsFrom(b), [b.heartbeat, b.lastHeardTs]); // eslint-disable-line react-hooks/exhaustive-deps
  const reachKey = (h: RemoteHost) => `${h.id}|${h.url}`;
  const reach = React.useSyncExternalStore(subscribeReach, getReachMap);
  React.useEffect(() => {
    let dead = false;
    const run = async () => {
      for (const h of hosts) {
        if (!h.hasDirect || h.stale) {
          updateReach(reachKey(h), () => "unreachable");
          continue;
        }
        updateReach(reachKey(h), (r) => (r === "reachable" ? "reachable" : "checking"));
        const ok = await probeHost(h.url);
        if (!dead) updateReach(reachKey(h), () => (ok ? "reachable" : "unreachable"));
      }
    };
    void run();
    const t = window.setInterval(run, 20_000);
    return () => {
      dead = true;
      window.clearInterval(t);
    };
  }, [hosts]);
  const reachOf = (h: RemoteHost): Reach => reach[reachKey(h)] ?? "unknown";
  const selected = (f: Feature) => {
    const id = f === "agents" ? prefs.agentsHost : prefs.terminalHost;
    return hosts.find((h) => h.id === id && h.supports[f]);
  };
  // Keep prefs honest once the heartbeat is known (not while loading): terminal can't stay on without a reachable terminal host,
  // agent management can't stay on without a host that supports it.
  // Only the one always-mounted watcher mirrors reachability into the prefs: several copies (each with its own view of the
  // heartbeat) would fight over the same two switches.
  React.useEffect(() => {
    if (!opts?.sync || hosts.length === 0) return;
    if (prefs.terminalOn) {
      const h = selected("terminal");
      if (!h || reachOf(h) === "unreachable") setWebRemotePrefs({ terminalOn: false });
    }
    if (prefs.agentsOn && !selected("agents")) setWebRemotePrefs({ agentsOn: false });
    // Buzz term is on by default: once the desktop advertises terminals and its tailnet address answers, turn it on here
    // (and pick that desktop) unless the user switched it off themselves. Without this the page stayed on its
    // "isn't available" instructions even though the desktop was ready.
    if (!prefs.terminalOn && !prefs.terminalOptOut) {
      const ready = hosts.find((h) => h.supports.terminal && !h.stale && reachOf(h) === "reachable");
      if (ready) setWebRemotePrefs({ terminalOn: true, terminalHost: ready.id });
    }
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
                Using the relay — slower, and without create, rename or edit. Connect to your tailnet for a direct connection.
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
            setWebRemotePrefs({
              terminalOn: on,
              terminalOptOut: !on,
              ...(on && !prefs.terminalHost ? { terminalHost: reachable[0]?.id ?? null } : {}),
            })
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

/** Web: binds the desktop command proxy to the agents host's shared direct client (one signed connection for the Agents page, Settings and the controls). */
export function useBindDirectProxy() {
  const rh = useRemoteHosts();
  const host = rh.selected("agents") ?? rh.hosts.find((h) => h.supports.agents);
  const direct = React.useMemo(
    () => (host?.hasDirect && rh.b.available ? directAgentsFor({ desktopId: host.id, url: host.url }, rh.b.signJson) : null),
    [host?.id, host?.url, host?.hasDirect, rh.b.available], // eslint-disable-line react-hooks/exhaustive-deps
  );
  React.useEffect(() => acquireProxyDirect(direct), [direct]);
  return direct;
}

/** Mounted in the left nav on web: keeps the switches consistent with what hosts offer, and exposes what the nav may show. */
export function WebRemoteWatcher() {
  const rh = useRemoteHosts({ sync: true });
  const queryClient = useQueryClient();
  const host = rh.selected("agents") ?? rh.hosts.find((h) => h.supports.agents);
  const direct = React.useMemo(
    () => (host?.hasDirect && rh.b.available ? directAgentsFor({ desktopId: host.id, url: host.url }, rh.b.signJson) : null),
    [host?.id, host?.url, host?.hasDirect, rh.b.available], // eslint-disable-line react-hooks/exhaustive-deps
  );
  React.useEffect(() => acquireProxyDirect(direct), [direct]);
  // The desktop's data replaces the web's empty defaults the moment the connection goes live (and back when it ends).
  const live = useDirectState(direct).live;
  React.useEffect(() => {
    void queryClient.invalidateQueries({ predicate: (q) => /agent|persona|runtime|acp|harness|presence/i.test(String(q.queryKey[0])) });
  }, [live, direct, queryClient]);
  return null;
}
/** Content of the page a greyed-out nav row opens on web/mobile: what is needed, and the exact settings to turn on. */
export const BUZZ_TERM_UNAVAILABLE = {
  title: "Buzz term isn't available",
  summary: "Remote terminal is off, or no desktop is reachable over your tailnet.",
  steps: [
    "On your desktop, turn on Settings → Buzz Term → Allow remote terminal.",
    "Connect your phone and Mac to your tailnet (Tailscale).",
    "Here, open Settings → Buzz Term, turn on Allow remote terminal and pick that desktop.",
  ],
};
// TODO(difference 4): once full gating exists (tailnet reachability per host, signature lifetime, last-seen > 1 day), derive the
// per-row reason from it so the page can say which requirement is missing. Today: availability = switch on + a host selected.
export function useWebRemoteNav(): { agents: boolean; terminal: boolean } {
  const prefs = React.useSyncExternalStore(subscribeWebRemotePrefs, getWebRemotePrefs);
  return { agents: prefs.agentsOn && !!prefs.agentsHost, terminal: prefs.terminalOn && !!prefs.terminalHost };
}
