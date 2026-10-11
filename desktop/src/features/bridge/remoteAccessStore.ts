// Desktop-side state of slice B (Rust `remote_access_*` commands). The Settings card and the bridge heartbeat share it.
import { invoke, isTauri } from "@tauri-apps/api/core";

import { getTermPreferences } from "@/features/terminal/termPreferences";
import type { RemoteInfo } from "./protocol";

export type RemoteAccessStatus = {
  tailscaleUp: boolean;
  reason: string | null;
  dnsName: string | null;
  enabled: boolean;
  terminals: boolean;
  url: string | null;
  openTerminals: number;
};

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
  return {
    desktopId: desktopId(),
    name: label,
    url: s.url ?? "",
    status: s.enabled && s.url ? "on" : "off",
    ...(s.enabled ? {} : { reason: s.tailscaleUp ? undefined : (s.reason ?? "Tailscale isn't running on this Mac.") }),
    terminals: s.terminals,
  };
}
