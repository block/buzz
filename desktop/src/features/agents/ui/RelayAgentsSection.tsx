import * as React from "react";
import { BadgeCheck } from "lucide-react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";

type RelayAgentsSectionProps = {
  /** Local managed agents; relay records for these pubkeys are shown above. */
  managedAgents: ManagedAgent[];
  onOpenAgentProfile: (pubkey: string) => void;
};

function audienceLabel(agent: RelayAgent): string {
  switch (agent.respondTo) {
    case "anyone":
      return "anyone";
    case "allowlist":
      return "allowlist";
    case "owner-only":
      return "owner only";
    default:
      return "no audience set";
  }
}

function channelsLabel(agent: RelayAgent): string {
  const count = agent.channelIds.length;
  if (count === 0) return "No channels";
  return count === 1 ? "1 channel" : `${count} channels`;
}

/**
 * Read-only directory of agents hosted outside this app (kind:10100 records
 * published on the relay). These agents run elsewhere — there is deliberately
 * no start/edit/delete here: recreating them locally would mint duplicate
 * identities. Records whose pubkey matches a local managed agent are shown in
 * the managed section instead; archived identities are hidden entirely.
 */
export function RelayAgentsSection({
  managedAgents,
  onOpenAgentProfile,
}: RelayAgentsSectionProps) {
  const relayAgentsQuery = useRelayAgentsQuery();
  const isArchived = useIsArchivedPredicate();
  const identityQuery = useIdentityQuery();
  const selfPubkey = identityQuery.data?.pubkey?.toLowerCase() ?? null;

  const managedPubkeys = React.useMemo(
    () => new Set(managedAgents.map((agent) => agent.pubkey.toLowerCase())),
    [managedAgents],
  );

  const remoteAgents = React.useMemo(
    () =>
      (relayAgentsQuery.data ?? [])
        .filter((agent) => !managedPubkeys.has(agent.pubkey.toLowerCase()))
        .filter((agent) => !isArchived(agent.pubkey))
        .sort((a, b) => a.name.localeCompare(b.name)),
    [relayAgentsQuery.data, managedPubkeys, isArchived],
  );

  const profilesQuery = useUsersBatchQuery(
    remoteAgents.map((agent) => agent.pubkey),
    { enabled: remoteAgents.length > 0 },
  );

  if (remoteAgents.length === 0) return null;

  return (
    <section className="relative space-y-4" data-testid="agents-on-this-relay">
      <SectionHeader
        description="Agents hosted elsewhere that operate in this community. Read-only — manage them where they run."
        title="On this relay"
      />
      <div className={IDENTITY_CARD_GRID_CLASS}>
        {remoteAgents.map((agent) => {
          const summary =
            profilesQuery.data?.profiles[agent.pubkey.toLowerCase()] ?? null;
          const label = summary?.displayName?.trim() || agent.name;
          const ownedByMe =
            selfPubkey !== null &&
            summary?.ownerPubkey?.toLowerCase() === selfPubkey;

          return (
            <AgentIdentityCard
              ariaLabel={`Open ${label} profile`}
              avatarUrl={summary?.avatarUrl}
              dataTestId={`relay-agent-card-${agent.pubkey}`}
              key={agent.pubkey}
              label={label}
              modelLabel={`${channelsLabel(agent)} · ${audienceLabel(agent)}`}
              onClick={() => onOpenAgentProfile(agent.pubkey)}
              statusBadge={
                ownedByMe ? (
                  <Badge className="gap-1" variant="success">
                    <BadgeCheck className="h-3 w-3" />
                    Owned by you
                  </Badge>
                ) : null
              }
            />
          );
        })}
      </div>
    </section>
  );
}
