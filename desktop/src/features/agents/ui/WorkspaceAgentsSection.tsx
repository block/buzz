import * as React from "react";

import { useAgentAvailabilityLookup } from "@/features/agents/lib/useAgentAvailability";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type {
  ManagedAgent,
  PresenceStatus,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { AgentRuntimeAvatarControl } from "./AgentRuntimeAvatarControl";
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";
import {
  describeWorkspaceAgent,
  resolveWorkspaceAgentAvailability,
  resolveWorkspaceOwnerLabel,
  selectWorkspaceAgents,
} from "./workspaceAgents";

type WorkspaceAgentsSectionProps = {
  error: Error | null;
  /** Managed agents must be known before dedupe, or own agents flash here. */
  isLoading: boolean;
  managedAgents: ManagedAgent[];
  /** `undefined` until the relay directory has loaded. */
  relayAgents: RelayAgent[] | undefined;
  onOpenAgentProfile: (pubkey: string) => void;
};

/** Read-only view of agents other workspace members run; no lifecycle controls. */
export function WorkspaceAgentsSection({
  error,
  isLoading,
  managedAgents,
  relayAgents,
  onOpenAgentProfile,
}: WorkspaceAgentsSectionProps) {
  const agents = React.useMemo(
    () => selectWorkspaceAgents(relayAgents, managedAgents),
    [relayAgents, managedAgents],
  );
  const agentPubkeys = React.useMemo(
    () => agents.map((agent) => agent.pubkey),
    [agents],
  );
  // One profile batch covers card avatars and "Managed by" owner names.
  const profilePubkeys = React.useMemo(
    () =>
      agents.flatMap((agent) =>
        agent.ownerPubkey ? [agent.pubkey, agent.ownerPubkey] : [agent.pubkey],
      ),
    [agents],
  );
  const profiles = useUsersBatchQuery(profilePubkeys).data?.profiles;
  const { getAvailability } = useAgentAvailabilityLookup(agentPubkeys);
  const isPending = isLoading || relayAgents === undefined;

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-workspace"
    >
      <SectionHeader
        title="Workspace agents"
        description="Agents run by other members of this workspace."
      />

      {isPending ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-20"
            footerTitleWidthClass="w-28"
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-16"
            footerTitleWidthClass="w-24"
          />
        </div>
      ) : agents.length > 0 ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <WorkspaceAgentCard
              agent={agent}
              availability={resolveWorkspaceAgentAvailability(
                getAvailability(agent.pubkey),
                agent.status,
              )}
              avatarUrl={
                profiles?.[normalizePubkey(agent.pubkey)]?.avatarUrl ?? null
              }
              key={agent.pubkey}
              ownerLabel={resolveWorkspaceOwnerLabel(
                agent.ownerPubkey,
                agent.ownerPubkey
                  ? profiles?.[normalizePubkey(agent.ownerPubkey)]
                  : undefined,
              )}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          No agents from other workspace members yet.
        </p>
      )}

      {error ? (
        <p className="w-full rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function WorkspaceAgentCard({
  agent,
  availability,
  avatarUrl,
  ownerLabel,
  onOpenAgentProfile,
}: {
  agent: RelayAgent;
  availability: PresenceStatus | undefined;
  avatarUrl: string | null;
  ownerLabel: string | null;
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  return (
    <AgentIdentityCard
      ariaLabel={`${agent.name} agent profile`}
      avatar={
        // Another member's agent has no Start/Stop here; `isActive` only
        // selects the presence-dot face over the Start badge (see the control's
        // prop comment), so `onStart` can never fire.
        <AgentRuntimeAvatarControl
          activeTestId={`workspace-agent-presence-${agent.pubkey}`}
          availability={availability}
          avatarUrl={avatarUrl}
          isActive
          isStarting={false}
          label={agent.name}
          startTestId={`workspace-agent-start-${agent.pubkey}`}
          onStart={() => {}}
        />
      }
      avatarUrl={avatarUrl}
      dataTestId={`workspace-agent-${agent.pubkey}`}
      label={agent.name}
      subtitle={describeWorkspaceAgent(agent, ownerLabel)}
      onClick={() => onOpenAgentProfile(agent.pubkey)}
    />
  );
}
