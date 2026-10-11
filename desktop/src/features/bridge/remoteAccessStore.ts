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
  /** Direct agent management is on (the relay path doesn't need the listener). */
  agents: boolean;
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

/** Settings → Buzz Term as the Rust terminal backend needs it; read there at every remote launch (no process-wide flag). */
export function termConfigArgs() {
  const t = getTermPreferences();
  return { herdr: t.sessionHost === "herdr", shell: t.shell, cwd: t.cwd, scrollback: t.scrollback };
}
export async function pushTermConfig(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("remote_access_set_term_config", termConfigArgs());
  } catch {
    /* older build without the command */
  }
}
/** "Allow remote agent management" + signature lifetime, pushed to the listener (the switch is re-checked per request). */
export async function pushAgentConfig(): Promise<void> {
  if (!isTauri()) return;
  const p = getRemotePrefs();
  try {
    set(await invoke<RemoteAccessStatus>("remote_access_set_agents", { on: p.agentManagement, ttlS: p.sigTtlS }));
  } catch {
    /* older build without the command */
  }
}

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

let startupSynced = false;
export async function refreshRemoteAccess(): Promise<void> {
  if (!isTauri()) return;
  try {
    set(await invoke<RemoteAccessStatus>("remote_access_status"));
  } catch {
    /* command unavailable (older build) */
    return;
  }
  // Once per launch: "Allow remote agent management" is a saved setting, so bring the direct listener back up after a restart.
  if (!startupSynced) {
    startupSynced = true;
    await pushAgentConfig();
    if (getRemotePrefs().agentManagement) await syncListener().catch(() => {});
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
  await pushAgentConfig(); // before the listener starts, so it comes up with the right switch and lifetime
  await syncListener();
  emit();
}
/** Signature lifetime for direct connections (max 24 h): applies to connections signed from now on. */
export function setSignatureTtl(seconds: number): void {
  if ((SIG_TTL_OPTIONS_S as readonly number[]).includes(seconds)) localStorage.setItem(PREF_TTL, String(seconds));
  void pushAgentConfig();
  emit();
}
/** Turn terminals on/off; terminals need the tailnet listener, which this starts if needed. */
export async function setTerminalsAllowed(on: boolean): Promise<void> {
  if (on && !status?.tailscaleUp) throw new Error(status?.reason ?? "Remote terminals need Tailscale running on this computer.");
  await pushTermConfig();
  if (on && !status?.enabled) await setRemoteAccessEnabled(true);
  await setRemoteTerminals(on);
  if (!on) await syncListener();
}

export async function setRemoteAccessEnabled(enabled: boolean): Promise<void> {
  if (enabled) {
    await pushTermConfig();
    await pushAgentConfig();
  }
  set(await invoke<RemoteAccessStatus>("remote_access_set_enabled", { enabled, desktopId: desktopId() }));
}

export async function setRemoteTerminals(on: boolean): Promise<void> {
  set(await invoke<RemoteAccessStatus>("remote_access_set_terminals", { on }));
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

// The Rust backend reads Settings → Buzz Term (session host, shell, default directory, scrollback) at every remote launch from
// the last pushed copy. Push it at startup and whenever any of those settings change, and re-publish the heartbeat (it
// advertises the session host and session name so the web Buzz term page can show the effective setting).
if (isTauri()) {
  let last = JSON.stringify(termConfigArgs());
  void pushTermConfig();
  void pushAgentConfig();
  subscribeTermPreferences(() => {
    emit();
    const now = JSON.stringify(termConfigArgs());
    if (now !== last) {
      last = now;
      void pushTermConfig();
    }
  });
}
