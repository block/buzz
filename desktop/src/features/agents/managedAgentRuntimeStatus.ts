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

export function findManagedAgentRuntime(
  runtimes: readonly ManagedAgentRuntimeStatus[],
  pubkey: string,
  relayUrl: string,
): ManagedAgentRuntimeStatus | undefined {
  const normalizedPubkey = pubkey.toLowerCase();
  return runtimes.find(
    (runtime) =>
      runtime.pubkey.toLowerCase() === normalizedPubkey &&
      (runtime.relayUrl === relayUrl || runtime.requestedRelayUrl === relayUrl),
  );
}
