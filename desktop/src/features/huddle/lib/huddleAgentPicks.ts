import {
  isManagedAgentActive,
  managedAgentPresenceStatus,
} from "@/features/agents/lib/managedAgentControlActions";
import type {
  ManagedAgent,
  PresenceLookup,
  PresenceStatus,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type HuddleAgentPick = {
  pubkey: string;
  name: string;
  presence: PresenceStatus;
};

export type HuddleAgentPickList = {
  picks: HuddleAgentPick[];
  /** Rendered when `picks` is empty — separates "nothing is alive" from
   *  "everything alive is already here", which read identically before. */
  emptyMessage: string;
};

/**
 * Which agents this huddle can still add.
 *
 * A huddle reaches an agent entirely over the relay — `add_agent_to_huddle`
 * posts kind:9000 membership, the agent's own subscription picks the channel
 * up, transcripts go out as kind:9 and replies come back the same way — so
 * where the agent's process runs never enters into it. The only disqualifier
 * is being dead.
 *
 * That makes `status === "running"` the wrong question: it is the LOCAL
 * process table, and every provider-backed record is permanently `"deployed"`
 * instead (rule 14, `features/agents/AGENTS.md`), so filtering on it hid the
 * whole remote fleet behind "No running agents found." Liveness is
 * `managedAgentPresenceStatus` — the control plane for a local record, relay
 * presence for a remote one — gated by `isManagedAgentActive` so a stopped
 * local agent, which that helper answers `"online"` for unconditionally,
 * stays out.
 */
export function huddleAgentPicks({
  agents,
  presenceLookup,
  currentAgentPubkeys,
}: {
  agents: readonly ManagedAgent[];
  presenceLookup: PresenceLookup | null | undefined;
  currentAgentPubkeys: readonly string[];
}): HuddleAgentPickList {
  const alive: HuddleAgentPick[] = agents.flatMap((agent) => {
    if (!isManagedAgentActive(agent)) return [];
    const presence = managedAgentPresenceStatus(agent, presenceLookup);
    return presence === "offline"
      ? []
      : [{ pubkey: agent.pubkey, name: agent.name, presence }];
  });

  const joined = new Set(currentAgentPubkeys.map(normalizePubkey));
  const picks = alive.filter(
    (pick) => !joined.has(normalizePubkey(pick.pubkey)),
  );

  return {
    picks,
    emptyMessage:
      alive.length > 0
        ? "All online agents are already in this huddle."
        : "No online agents found.",
  };
}
