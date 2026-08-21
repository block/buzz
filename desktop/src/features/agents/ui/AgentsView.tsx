import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { getSharedChannelIds } from "@/features/agents/lib/agentAutocompleteEligibility";
import { availableRelayAgents } from "@/features/agents/lib/availableRelayAgents";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { PageHeader } from "@/shared/ui/PageHeader";
import { RelayAgentsSection } from "./RelayAgentsSection";

export function AgentsView() {
  const { openProfilePanel } = useProfilePanel();
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();

  const sharedChannelIds = React.useMemo(
    () => getSharedChannelIds(channelsQuery.data),
    [channelsQuery.data],
  );
  const agents = React.useMemo(
    () =>
      availableRelayAgents(
        relayAgentsQuery.data,
        sharedChannelIds,
        identityQuery.data?.pubkey,
      ),
    [identityQuery.data?.pubkey, relayAgentsQuery.data, sharedChannelIds],
  );

  return (
    <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
      <div
        className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]"
        data-testid="agents-page-content"
      >
        <PageHeader
          description="Agents available to you on this relay."
          title="Your agents"
        />
        <RelayAgentsSection
          agents={agents}
          error={
            relayAgentsQuery.error instanceof Error
              ? relayAgentsQuery.error
              : null
          }
          isLoading={
            identityQuery.isPending ||
            channelsQuery.isPending ||
            relayAgentsQuery.isPending
          }
          onOpenAgentProfile={(pubkey, options) => {
            openProfilePanel?.(pubkey, options);
          }}
        />
      </div>
    </div>
  );
}
