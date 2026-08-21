import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { getSharedChannelIds } from "@/features/agents/lib/agentAutocompleteEligibility";
import { availableRelayAgents } from "@/features/agents/lib/availableRelayAgents";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { PageHeader } from "@/shared/ui/PageHeader";
import { CopyButton } from "./CopyButton";
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
  const userPubkey = identityQuery.data?.pubkey ?? null;
  const userNpub = userPubkey ? safeNpub(userPubkey) : null;

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
        {userPubkey ? (
          <section
            aria-labelledby="buzz-user-public-key-title"
            className="overflow-hidden rounded-3xl border border-primary/30 bg-primary/5 px-5 py-5 shadow-sm [@container(min-width:40rem)]:px-7 [@container(min-width:40rem)]:py-6"
            data-testid="buzz-user-public-key"
          >
            <div className="flex flex-col gap-5 [@container(min-width:40rem)]:flex-row [@container(min-width:40rem)]:items-start [@container(min-width:40rem)]:justify-between">
              <div className="min-w-0 flex-1">
                <h2
                  className="text-xs font-semibold uppercase tracking-widest text-primary"
                  id="buzz-user-public-key-title"
                >
                  Your Buzz public key
                </h2>
                <p
                  className="mt-3 break-all font-mono text-xl font-semibold leading-tight tracking-tight text-foreground [@container(min-width:40rem)]:text-3xl [@container(min-width:58rem)]:text-4xl"
                  data-testid="buzz-user-npub"
                >
                  {userNpub ?? userPubkey}
                </p>
              </div>
              <div className="flex shrink-0 flex-wrap gap-2">
                <CopyButton label="Copy npub" value={userNpub ?? userPubkey} />
                {userNpub ? (
                  <CopyButton label="Copy hex" value={userPubkey} />
                ) : null}
              </div>
            </div>
            <p className="mt-4 text-sm text-muted-foreground">
              This is your public identifier. Your private key is never shown.
            </p>
          </section>
        ) : null}
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
