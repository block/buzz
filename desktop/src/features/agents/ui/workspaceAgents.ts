import type { PresenceStatus, RelayAgent } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

/**
 * Relay-discovered agents run by other workspace members, for the read-only
 * "Workspace agents" section of the Agents screen.
 *
 * Excludes every pubkey the viewer already manages (those render as persona /
 * custom / unknown cards), collapses duplicate relay entries, substitutes the
 * canonical truncated pubkey for a blank name so every card has a label, and
 * sorts by that label. The managed side is structurally typed on `{ pubkey }`
 * so node unit tests don't need full `ManagedAgent` values.
 */
export function selectWorkspaceAgents(
  relayAgents: readonly RelayAgent[] | undefined,
  managedAgents: readonly { pubkey: string }[] | undefined,
): RelayAgent[] {
  const excluded = new Set(
    (managedAgents ?? []).map((agent) => normalizePubkey(agent.pubkey)),
  );
  const workspace: RelayAgent[] = [];
  for (const agent of relayAgents ?? []) {
    const pubkey = normalizePubkey(agent.pubkey);
    if (excluded.has(pubkey)) continue;
    excluded.add(pubkey);
    const name = agent.name.trim() || truncatePubkey(agent.pubkey);
    workspace.push(name === agent.name ? agent : { ...agent, name });
  }
  return workspace.sort((left, right) => left.name.localeCompare(right.name));
}

/** Card second line: owner, runtime, and channel count, dot-separated. */
export function describeWorkspaceAgent(
  agent: Pick<RelayAgent, "agentType" | "channels">,
  ownerLabel: string | null,
): string {
  const count = agent.channels.length;
  return [
    ownerLabel ? `Managed by ${ownerLabel}` : null,
    agent.agentType.trim() || null,
    `in ${count} channel${count === 1 ? "" : "s"}`,
  ]
    .filter((part) => part !== null)
    .join(" · ");
}

/** "Managed by" label: profile display name, kind-0 name, then truncated key. */
export function resolveWorkspaceOwnerLabel(
  ownerPubkey: string | null,
  summary:
    | { displayName?: string | null; name?: string | null }
    | null
    | undefined,
): string | null {
  if (!ownerPubkey) return null;
  return (
    summary?.displayName?.trim() ||
    summary?.name?.trim() ||
    truncatePubkey(ownerPubkey)
  );
}

/**
 * Relay presence is the availability authority (see docs/agent-availability.md);
 * the directory's own status snapshot only fills in while that read is unknown.
 */
export function resolveWorkspaceAgentAvailability(
  presence: PresenceStatus | undefined,
  status: RelayAgent["status"],
): PresenceStatus | undefined {
  if (presence) return presence;
  return status === "unknown" ? undefined : status;
}
