import type { ManagedAgent, PresenceStatus } from "@/shared/api/types";

export function isLiveAgentPresence(
  presence: PresenceStatus | undefined,
): boolean {
  return presence === "online" || presence === "away";
}

/**
 * Whether mention preparation must invoke the managed-agent start/deploy
 * command.
 *
 * Presence wins over the local record: the same signing identity may already
 * be served by an externally hosted harness while Desktop still has a stopped
 * local record. Starting that record would create two subscribers and two
 * replies for one mention.
 */
export function shouldStartManagedAgentForMention(
  agent: Pick<ManagedAgent, "backend" | "status">,
  presence: PresenceStatus | undefined,
): boolean {
  if (isLiveAgentPresence(presence)) {
    return false;
  }

  if (agent.backend.type === "provider") {
    return agent.status !== "deployed";
  }

  return agent.status !== "running" && agent.status !== "deployed";
}
