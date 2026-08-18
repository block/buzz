import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

export type AgentCommunityAvailability =
  | "Here"
  | "Waking"
  | "Needs setup on this device"
  | "Unavailable";

export function agentCommunityAvailability(
  runtime: ManagedAgentRuntimeStatus,
): AgentCommunityAvailability {
  if (!runtime.localSetup) return "Needs setup on this device";

  switch (runtime.lifecycle) {
    case "starting":
    case "listening":
    case "waking":
      return "Waking";
    case "ready":
      return "Here";
    case "failed":
    case "stopped":
      return "Unavailable";
  }
}

export function agentCommunityStatusDetail(
  runtime: ManagedAgentRuntimeStatus,
): string | null {
  if (!runtime.localSetup)
    return "Set up this agent on this device to start it.";
  if (runtime.lifecycle === "stopped") return "Stopped by you";
  if (runtime.lifecycle === "failed")
    return runtime.error ?? "Could not connect";
  return null;
}

export function managedAgentRuntimeKey(
  runtime: Pick<
    ManagedAgentRuntimeStatus,
    "pubkey" | "relayUrl" | "requestedRelayUrl"
  >,
): string {
  const requestedRelay = runtime.requestedRelayUrl ?? runtime.relayUrl;
  return JSON.stringify([
    runtime.pubkey.toLowerCase(),
    connectionTargetUrl(requestedRelay) ?? requestedRelay,
  ]);
}

export type ManagedAgentPairAction = "start" | "stop" | "restart";

/** Menu action for one agent+community pair. A missing runtime row means the
 * pair is not running here, so the only sensible action is to start it. */
export function managedAgentPairAction(
  runtime: ManagedAgentRuntimeStatus | undefined,
): ManagedAgentPairAction {
  if (!runtime || runtime.lifecycle === "stopped") return "start";
  if (runtime.lifecycle === "failed") return "restart";
  return "stop";
}

export const MANAGED_AGENT_PAIR_ACTION_LABELS: Record<
  ManagedAgentPairAction,
  string
> = {
  start: "Start Agent",
  stop: "Stop Agent",
  restart: "Restart Agent",
};

/**
 * Canonicalize a relay URL the way the backend keys runtime pairs, so a
 * stored community URL (e.g. `ws://localhost:3000`) matches backend rows
 * (`ws://127.0.0.1:3000`). Mirrors buzz-core's `normalize_relay_url`
 * (`crates/buzz-core/src/relay.rs`): lowercase host, loopback hosts folded
 * to 127.0.0.1, default ports and root-path trailing slash stripped.
 * Returns null when the URL cannot be parsed as ws/wss.
 */
export function canonicalRelayUrl(raw: string): string | null {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    return null;
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") return null;
  let host = url.hostname.toLowerCase();
  if (host === "localhost" || host === "[::1]" || host.startsWith("127.")) {
    host = "127.0.0.1";
  }
  const defaultPort = url.protocol === "ws:" ? "80" : "443";
  const port = url.port && url.port !== defaultPort ? `:${url.port}` : "";
  const path = url.pathname === "/" ? "" : url.pathname;
  // The backend trims trailing slashes from the final rendered URL.
  return `${url.protocol}//${host}${port}${path}${url.search}`.replace(
    /\/+$/,
    "",
  );
}

/**
 * Comparable connection target mirroring the backend's tenancy authority
 * (buzz-core's `tenant::normalize_host`): lowercase host, strip an explicit
 * default port and the FQDN root dot, fold the root-path slash - WITHOUT
 * folding loopback spellings, which are distinct tenants on a host-scoped
 * relay. Returns null when the URL cannot be parsed as ws/wss, or when it
 * carries userinfo or a fragment; callers then fall back to exact comparison.
 */
export function connectionTargetUrl(raw: string): string | null {
  let url: URL;
  try {
    url = new URL(raw.trim());
  } catch {
    return null;
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") return null;
  if (url.username || url.password || url.hash) return null;
  const host = url.hostname.toLowerCase().replace(/\.$/, "");
  const defaultPort = url.protocol === "ws:" ? "80" : "443";
  const port = url.port && url.port !== defaultPort ? `:${url.port}` : "";
  const path = url.pathname === "/" ? "" : url.pathname;
  return `${url.protocol}//${host}${port}${path}${url.search}`;
}

/** Match relay connection authorities without canonical loopback folding. */
export function connectionTargetsMatch(left: string, right: string): boolean {
  const leftTarget = connectionTargetUrl(left);
  const rightTarget = connectionTargetUrl(right);
  if (leftTarget !== null && rightTarget !== null) {
    return leftTarget === rightTarget;
  }
  return left.trim() === right.trim();
}

export function findManagedAgentRuntime(
  runtimes: readonly ManagedAgentRuntimeStatus[],
  pubkey: string,
  relayUrl: string,
): ManagedAgentRuntimeStatus | undefined {
  const normalizedPubkey = pubkey.toLowerCase();
  const canonical = canonicalRelayUrl(relayUrl);
  return runtimes.find((runtime) => {
    if (runtime.pubkey.toLowerCase() !== normalizedPubkey) return false;
    // A row carrying the actual dial spelling is authoritative: canonical
    // matching would alias distinct loopback tenants that share one runtime
    // key, letting the wrong community's card claim - and stop - this child.
    if (runtime.requestedRelayUrl != null) {
      return connectionTargetsMatch(runtime.requestedRelayUrl, relayUrl);
    }
    // Legacy rows without the dial spelling keep the canonical fallback
    // (exact-string check first, for unparsable stored URLs).
    return (
      runtime.relayUrl === relayUrl ||
      (canonical !== null && runtime.relayUrl === canonical)
    );
  });
}
