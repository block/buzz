import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

export type AgentCommunityAvailability =
  | "Here"
  | "Starting"
  | "Listening"
  | "Waking"
  | "Recovering"
  | "Legacy runtime active"
  | "Manual stop required"
  | "Needs setup on this device"
  | "Failed"
  | "Stopped";

export type ManagedAgentRuntimePresentation = {
  label: AgentCommunityAvailability;
  detail: string | null;
  variant: "default" | "secondary" | "warning" | "destructive";
};
const RUNTIME_PRESENTATION: Record<
  ManagedAgentRuntimeStatus["lifecycle"],
  ManagedAgentRuntimePresentation
> = {
  starting: {
    label: "Starting",
    detail: "Launching the persistent runtime.",
    variant: "secondary",
  },
  listening: {
    label: "Listening",
    detail: "The runtime is connected and opening its workspace.",
    variant: "secondary",
  },
  waking: {
    label: "Waking",
    detail: "The agent is reconnecting.",
    variant: "secondary",
  },
  ready: { label: "Here", detail: null, variant: "default" },
  recovering: {
    label: "Recovering",
    detail: "Restoring durable inbox, assignment, and job state.",
    variant: "warning",
  },
  legacy_runtime_active: {
    label: "Legacy runtime active",
    detail: "Stop the legacy runtime before starting the persistent runtime.",
    variant: "warning",
  },
  manual_legacy_stop_required: {
    label: "Manual stop required",
    detail:
      "This older runtime cannot be verified safely. Stop it manually before restarting.",
    variant: "destructive",
  },
  failed: {
    label: "Failed",
    detail: "Could not connect",
    variant: "destructive",
  },
  stopped: { label: "Stopped", detail: "Stopped by you", variant: "secondary" },
};

export function managedAgentRuntimePresentation(
  runtime: ManagedAgentRuntimeStatus,
): ManagedAgentRuntimePresentation {
  if (!runtime.localSetup) {
    return {
      label: "Needs setup on this device",
      detail: "Set up this agent on this device to start it.",
      variant: "secondary",
    };
  }
  const presentation = RUNTIME_PRESENTATION[runtime.lifecycle];
  return runtime.lifecycle === "failed" && runtime.error
    ? { ...presentation, detail: runtime.error }
    : presentation;
}

export function agentCommunityAvailability(
  runtime: ManagedAgentRuntimeStatus,
): AgentCommunityAvailability {
  return managedAgentRuntimePresentation(runtime).label;
}

export function agentCommunityStatusDetail(
  runtime: ManagedAgentRuntimeStatus,
): string | null {
  return managedAgentRuntimePresentation(runtime).detail;
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
  start: "Start",
  stop: "Stop",
  restart: "Restart",
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

export function findManagedAgentRuntime(
  runtimes: readonly ManagedAgentRuntimeStatus[],
  pubkey: string,
  relayUrl: string,
): ManagedAgentRuntimeStatus | undefined {
  const normalizedPubkey = pubkey.toLowerCase();
  // Backend rows carry the canonical pair URL; the caller passes the
  // community's stored URL, which may differ in spelling (localhost vs
  // 127.0.0.1, default port, trailing slash). Compare canonically, keeping
  // the exact-string checks as a fallback for unparsable stored URLs.
  const canonical = canonicalRelayUrl(relayUrl);
  return runtimes.find(
    (runtime) =>
      runtime.pubkey.toLowerCase() === normalizedPubkey &&
      (runtime.relayUrl === relayUrl ||
        runtime.requestedRelayUrl === relayUrl ||
        (canonical !== null && runtime.relayUrl === canonical)),
  );
}
