// Desktop-side state of slice B (Rust `remote_access_*` commands). The Settings card and the bridge heartbeat share it.
import { invoke, isTauri } from "@tauri-apps/api/core";

import { getTermPreferences, subscribeTermPreferences } from "@/features/terminal/termPreferences";
import { SIG_TTL_DEFAULT_S, SIG_TTL_OPTIONS_S, type RemoteInfo } from "./protocol";

export type RemoteAccessStatus = {
  tailscaleUp: boolean;
  reason: string | null;
  dnsName: string | null;
  enabled: boolean;
  terminals: boolean;
  url: string | null;
  openTerminals: number;
};

// Desktop-side preferences for "Allow remote agent management" (frontend-persisted; relay commands are gated on it).
const PREF_AGENTS = "buzz-remote-agent-mgmt";
const PREF_TTL = "buzz-remote-sig-ttl";
export type RemotePrefs = { agentManagement: boolean; sigTtlS: number };
export function getRemotePrefs(): RemotePrefs {
  try {
    const ttl = Number(localStorage.getItem(PREF_TTL));
    return {
      agentManagement: localStorage.getItem(PREF_AGENTS) === "1",
      sigTtlS: (SIG_TTL_OPTIONS_S as readonly number[]).includes(ttl) ? ttl : SIG_TTL_DEFAULT_S,
    };
  } catch {
    return { agentManagement: false, sigTtlS: SIG_TTL_DEFAULT_S };
  }
}
export const isAgentManagementAllowed = () => getRemotePrefs().agentManagement;

const DEVICE_KEY = "buzz-bridge-device-id";
const POLL_MS = 10_000;

let status: RemoteAccessStatus | null = null;
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;

const emit = () => {
  for (const l of listeners) l();
};
const set = (s: RemoteAccessStatus) => {
  status = s;
  emit();
};
const herdr = () => getTermPreferences().sessionHost === "herdr";

export function desktopId(): string {
  try {
    let id = localStorage.getItem(DEVICE_KEY);
    if (!id) {
      id = crypto.randomUUID();
      localStorage.setItem(DEVICE_KEY, id);
    }
    return id;
  } catch {
    return "ephemeral";
  }
}

export async function refreshRemoteAccess(): Promise<void> {
  if (!isTauri()) return;
  try {
    set(await invoke<RemoteAccessStatus>("remote_access_status"));
  } catch {
    /* command unavailable (older build) */
  }
}

export function getRemoteAccess(): RemoteAccessStatus | null {
  return status;
}

export function subscribeRemoteAccess(fn: () => void): () => void {
  listeners.add(fn);
  if (!timer && isTauri()) {
    void refreshRemoteAccess();
    timer = setInterval(() => void refreshRemoteAccess(), POLL_MS);
  }
  return () => {
    listeners.delete(fn);
    if (listeners.size === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
}

/** The tailnet listener runs when either feature wants it and Tailscale is up; agent management alone still works via the relay. */
async function syncListener(): Promise<void> {
  const p = getRemotePrefs();
  const s = status;
  if (!s) return;
  const want = s.tailscaleUp && (p.agentManagement || s.terminals);
  if (want !== s.enabled) await setRemoteAccessEnabled(want);
  else emit();
}
export async function setAgentManagement(on: boolean): Promise<void> {
  localStorage.setItem(PREF_AGENTS, on ? "1" : "0");
  await syncListener();
  emit();
}
export function setSignatureTtl(seconds: number): void {
  if ((SIG_TTL_OPTIONS_S as readonly number[]).includes(seconds)) localStorage.setItem(PREF_TTL, String(seconds));
  emit();
}
/** Turn terminals on/off; terminals need the tailnet listener, which this starts if needed. */
export async function setTerminalsAllowed(on: boolean): Promise<void> {
  if (on && !status?.tailscaleUp) throw new Error(status?.reason ?? "Remote terminals need Tailscale running on this computer.");
  if (on && !status?.enabled) await setRemoteAccessEnabled(true);
  await setRemoteTerminals(on);
  if (!on) await syncListener();
}

export async function setRemoteAccessEnabled(enabled: boolean): Promise<void> {
  set(await invoke<RemoteAccessStatus>("remote_access_set_enabled", { enabled, desktopId: desktopId(), herdr: herdr() }));
}

export async function setRemoteTerminals(on: boolean): Promise<void> {
  set(await invoke<RemoteAccessStatus>("remote_access_set_terminals", { on, herdr: herdr() }));
}

/** What the bridge heartbeat advertises (undefined = this build/desktop has no slice B). */
export function remoteInfoForHeartbeat(): RemoteInfo | undefined {
  const s = status;
  if (!s) return undefined;
  const label = s.dnsName?.split(".")[0] ?? "this Mac";
  const prefs = getRemotePrefs();
  return {
    desktopId: desktopId(),
    name: label,
    url: s.url ?? "",
    status: s.enabled && s.url ? "on" : "off",
    ...(s.enabled ? {} : { reason: s.tailscaleUp ? undefined : (s.reason ?? "Tailscale isn't running on this Mac.") }),
    terminals: s.terminals,
    termHost: herdr() ? "herdr" : "shell",
    ...(herdr() ? { termSession: getTermPreferences().sessionName } : {}),
    agents: prefs.agentManagement,
    sigTtlS: prefs.sigTtlS,
  };
}

// The Rust backend only learns shell-vs-herdr when a switch is flipped (the `herdr` argument). Re-push it whenever
// Settings → Buzz Term → Session host changes while remote access is on, so the next remote terminal uses the new mode;
// the heartbeat also advertises it (termHost / termSession) so the web Buzz term page can show the effective setting.
if (isTauri()) {
  let lastHost = herdr();
  subscribeTermPreferences(() => {
    const now = herdr();
    const hostChanged = now !== lastHost;
    lastHost = now;
    emit(); // re-publish the heartbeat (session name or host may have changed)
    if (hostChanged && status?.enabled) void setRemoteTerminals(status.terminals).catch(() => {});
  });
}
