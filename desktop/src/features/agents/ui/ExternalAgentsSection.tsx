import * as React from "react";
import { KeyRound, Pencil } from "lucide-react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import {
  getSharedChannelIds,
  getVisibleExternalAgents,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUserProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { AgentIdentityCard } from "./AgentIdentityCard";
import {
  ExternalAgentIdentityDialog,
  useExternalAgentIdentityStatus,
} from "./ExternalAgentIdentityDialog";
import { AGENT_CARD_GRID_COLUMNS_CLASS } from "./UnifiedAgentsSection";
import { Button } from "@/shared/ui/button";

type ExternalAgentsSectionProps = {
  managedAgentPubkeys: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

export function ExternalAgentsSection({
  managedAgentPubkeys,
  onOpenAgentProfile,
}: ExternalAgentsSectionProps) {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const sharedChannelIds = React.useMemo(
    () => getSharedChannelIds(channelsQuery.data),
    [channelsQuery.data],
  );
  const agents = React.useMemo(
    () =>
      getVisibleExternalAgents({
        currentPubkey: identityQuery.data?.pubkey,
        managedAgentPubkeys,
        relayAgents: relayAgentsQuery.data,
        sharedChannelIds,
      }),
    [
      identityQuery.data?.pubkey,
      managedAgentPubkeys,
      relayAgentsQuery.data,
      sharedChannelIds,
    ],
  );

  if (agents.length === 0) return null;

  return (
    <section className="space-y-3" data-testid="external-agents-section">
      <div>
        <h2 className="text-sm font-semibold">Connected agents</h2>
        <p className="text-xs text-muted-foreground">
          Agents that run outside Buzz Desktop and are available to you.
        </p>
      </div>
      <div
        className={`grid w-full justify-start gap-3 ${AGENT_CARD_GRID_COLUMNS_CLASS}`}
      >
        {agents.map((agent) => (
          <ExternalAgentCard
            agent={agent}
            key={agent.pubkey}
            onOpenAgentProfile={onOpenAgentProfile}
          />
        ))}
      </div>
    </section>
  );
}

function ExternalAgentCard({
  agent,
  onOpenAgentProfile,
}: {
  agent: RelayAgent;
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const identityStatusQuery = useExternalAgentIdentityStatus(agent.pubkey);
  const [identityDialogOpen, setIdentityDialogOpen] = React.useState(false);
  const title = profileQuery.data?.displayName?.trim() || agent.name;
  const agentType = agent.agentType.trim();
  const modelLabel = agentType
    ? `${agentType} · managed externally`
    : "Managed externally";

  return (
    <>
      <AgentIdentityCard
        actions={
          <Button
            aria-label={
              identityStatusQuery.data?.linked
                ? `Edit ${title} profile`
                : `Link ${title} identity`
            }
            onClick={() => setIdentityDialogOpen(true)}
            size="icon"
            type="button"
            variant="ghost"
          >
            {identityStatusQuery.data?.linked ? (
              <Pencil className="h-4 w-4" />
            ) : (
              <KeyRound className="h-4 w-4" />
            )}
          </Button>
        }
        ariaLabel={`${title} connected agent profile`}
        avatarUrl={profileQuery.data?.avatarUrl}
        dataTestId={`external-agent-${agent.pubkey}`}
        label={title}
        modelLabel={modelLabel}
        onClick={() => onOpenAgentProfile(agent.pubkey)}
      />
      <ExternalAgentIdentityDialog
        name={title}
        onOpenChange={setIdentityDialogOpen}
        open={identityDialogOpen}
        profile={profileQuery.data}
        pubkey={agent.pubkey}
      />
    </>
  );
}
