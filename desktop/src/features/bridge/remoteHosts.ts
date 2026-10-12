// Web side of "which desktops can I use for remote agent management / remote terminal" (pure logic + tiny prefs store).
import { HOST_STALE_S, nowS, type RemoteInfo } from "./protocol";
import type { BridgeState } from "./webClient";

export type Feature = "agents" | "terminal";
export type RemoteHost = {
  id: string;
  name: string;
  url: string;
  /** Seconds since this host was last heard from; null = never. */
  ageS: number | null;
  /** Not seen for over a day: shown greyed out, not hidden. */
  stale: boolean;
  /** Feature enabled in that desktop's own client. */
  supports: Record<Feature, boolean>;
  /** The desktop has a direct (tailnet) listener up. */
  hasDirect: boolean;
  /** Advertised signature lifetime (s). */
  sigTtlS: number | null;
};

/** Hosts from the signed heartbeat. (One heartbeat per member today, so at most one host; shape is per-host for per-device heartbeats.) */
export function hostsFrom(b: Pick<BridgeState, "heartbeat" | "lastHeardTs">, now = nowS()): RemoteHost[] {
  const r: RemoteInfo | undefined = b.heartbeat?.remote;
  if (!r) return [];
  const ageS = b.lastHeardTs === null ? null : Math.max(0, now - b.lastHeardTs);
  return [
    {
      id: r.desktopId,
      name: r.name || "your desktop",
      url: r.url,
      ageS,
      stale: ageS === null || ageS > HOST_STALE_S,
      supports: { agents: r.agents === true, terminal: r.terminals === true && r.status === "on" },
      hasDirect: r.status === "on" && !!r.url,
      sigTtlS: r.sigTtlS ?? null,
    },
  ];
}

export type Reach = "unknown" | "checking" | "reachable" | "unreachable";

/** A browser can't read Tailscale state: try to open the host's tailnet address. Resolves true if the socket opens in time. */
export function probeHost(
  url: string,
  timeoutMs = 3500,
  make: (u: string) => { close(): void; onopen: null | (() => void); onerror: null | (() => void); onclose: null | (() => void) } = (u) =>
    new WebSocket(u) as never,
): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    let ws: ReturnType<typeof make> | null = null;
    const end = (ok: boolean) => {
      if (done) return;
      done = true;
      clearTimeout(t);
      try {
        ws?.close();
      } catch {}
      resolve(ok);
    };
    const t = setTimeout(() => end(false), timeoutMs);
    try {
      ws = make(url);
    } catch {
      return end(false);
    }
    ws.onopen = () => end(true);
    ws.onerror = () => end(false);
    ws.onclose = () => end(false);
  });
}

/** Which transport a feature will use right now. Terminals never fall back. */
export function transportFor(feature: Feature, host: RemoteHost | undefined, reach: Reach): "direct" | "relay" | "unavailable" {
  if (!host || !host.supports[feature]) return "unavailable";
  if (host.hasDirect && reach === "reachable") return "direct";
  return feature === "agents" ? "relay" : "unavailable";
}

// ── per-device web preferences ───────────────────────────────────────────────────────────
const KEY = "buzz-web-remote-prefs";
export type WebRemotePrefs = {
  agentsOn: boolean;
  terminalOn: boolean;
  agentsHost: string | null;
  terminalHost: string | null;
  /** The user switched the terminal off on purpose: never turn it back on automatically. */
  terminalOptOut: boolean;
};
const DEFAULT: WebRemotePrefs = { agentsOn: false, terminalOn: false, agentsHost: null, terminalHost: null, terminalOptOut: false };
const subs = new Set<() => void>();
let cached: WebRemotePrefs | null = null;
export function getWebRemotePrefs(): WebRemotePrefs {
  if (cached) return cached;
  try {
    cached = { ...DEFAULT, ...JSON.parse(localStorage.getItem(KEY) || "{}") };
  } catch {
    cached = DEFAULT;
  }
  return cached as WebRemotePrefs;
}
export function setWebRemotePrefs(p: Partial<WebRemotePrefs>) {
  cached = { ...getWebRemotePrefs(), ...p };
  try {
    localStorage.setItem(KEY, JSON.stringify(cached));
  } catch {}
  for (const f of subs) f();
}
export function subscribeWebRemotePrefs(f: () => void) {
  subs.add(f);
  return () => void subs.delete(f);
}

/** The one specific thing that is missing, from what the desktop advertises and what this browser can reach. */
export function whyTermUnavailable(host: RemoteHost | undefined, hasRemoteInfo: boolean, reach: Reach): string | null {
  if (!hasRemoteInfo) return "Your desktop hasn't reported remote access yet. Make sure the desktop app is open, signed in and up to date.";
  if (!host) return null;
  if (!host.supports.terminal)
    return host.hasDirect
      ? "Allow remote terminal is off on your desktop (Settings → Buzz Term)."
      : "Your desktop's remote access isn't running (Tailscale may be off, or Allow remote access is off).";
  if (host.stale) return "Your desktop hasn't been seen in over a day.";
  if (reach === "unreachable") return "This browser can't reach your desktop over your tailnet. Connect this device to Tailscale.";
  return null;
}

