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
  runtime: Pick<ManagedAgentRuntimeStatus, "pubkey" | "relayUrl">,
): string {
  return JSON.stringify([runtime.pubkey, runtime.relayUrl]);
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
 * Canonicalize a relay URL the way the backend keys runtime pairs. Host
 * spellings remain distinct because the relay authority is the community:
 * `localhost`, `127.*`, and `::1` must never select one another's process.
 * DNS case/default ports and a root slash are syntax-only; non-root paths,
 * queries, and meaningful trailing slashes are preserved.
 */
export function canonicalRelayUrl(raw: string): string | null {
  const input = raw.trim();
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return null;
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") return null;
  if (url.username !== "" || url.password !== "" || input.includes("#"))
    return null;
  const host = url.hostname.toLowerCase();
  const defaultPort = url.protocol === "ws:" ? "80" : "443";
  const port = url.port && url.port !== defaultPort ? `:${url.port}` : "";
  const path = url.pathname === "/" ? "" : url.pathname;
  const query = url.search || (url.href.endsWith("?") ? "?" : "");
  return `${url.protocol}//${host}${port}${path}${query}`;
}

/**
 * Bestie's Rust scope check intentionally retains buzz-core's legacy
 * loopback-folding equivalence. Keep its React Query cache key aligned without
 * reusing that broader equivalence for managed-runtime identity.
 */
export function canonicalBestieRelayUrl(raw: string): string | null {
  const input = raw.trim();
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return null;
  }
  if (url.protocol !== "ws:" && url.protocol !== "wss:") return null;
  if (url.username !== "" || url.password !== "" || input.includes("#"))
    return null;
  let host = url.hostname.toLowerCase();
  if (host === "localhost" || host === "[::1]" || host.startsWith("127.")) {
    host = "127.0.0.1";
  }
  const port = url.port ? `:${url.port}` : "";
  const query = url.search || (url.href.endsWith("?") ? "?" : "");
  return `${url.protocol}//${host}${port}${url.pathname}${query}`.replace(
    /\/+$/,
    "",
  );
}

export function findManagedAgentRuntime(
  runtimes: readonly ManagedAgentRuntimeStatus[],
  pubkey: string,
  relayUrl: string,
): ManagedAgentRuntimeStatus | undefined {
  const normalizedPubkey = pubkey.toLowerCase();
  // Backend rows carry the canonical pair URL; compare syntax-equivalent
  // spellings while preserving distinct host authorities.
  const canonical = canonicalRelayUrl(relayUrl);
  return runtimes.find(
    (runtime) =>
      runtime.pubkey.toLowerCase() === normalizedPubkey &&
      (runtime.relayUrl === relayUrl ||
        runtime.requestedRelayUrl === relayUrl ||
        (canonical !== null && runtime.relayUrl === canonical)),
  );
}
