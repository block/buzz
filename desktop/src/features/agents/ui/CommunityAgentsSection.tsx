import * as React from "react";

import { useAgentAvailabilityLookup } from "@/features/agents/lib/useAgentAvailability";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { communityAgentChannelSummary } from "./communityAgents";
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";

type CommunityAgentsSectionProps = {
  agents: RelayAgent[];
  channelIdToName: Readonly<Record<string, string>>;
  error: Error | null;
  isLoading: boolean;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
};

export function CommunityAgentsSection({
  agents,
  channelIdToName,
  error,
  isLoading,
  onOpenAgentProfile,
}: CommunityAgentsSectionProps) {
  const isArchived = useIsArchivedPredicate();
  const visibleAgents = React.useMemo(
    () => agents.filter((agent) => !isArchived(agent.pubkey)),
    [agents, isArchived],
  );
  const pubkeys = React.useMemo(
    () => visibleAgents.map((agent) => agent.pubkey),
    [visibleAgents],
  );
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const { getAvailability } = useAgentAvailabilityLookup(pubkeys);
  const profiles = profilesQuery.data?.profiles ?? {};

  return (
    <section
      aria-busy={isLoading}
      className="relative space-y-4"
      data-testid="community-agents"
    >
      <SectionHeader
        title="Community agents"
        description="Agents available in this community but not managed on this device."
      />

      {isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-24"
            footerTitleWidthClass="w-32"
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-20"
            footerTitleWidthClass="w-28"
          />
        </div>
      ) : null}

      {!isLoading && visibleAgents.length > 0 ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          {visibleAgents.map((agent) => {
            const profile = profiles[normalizePubkey(agent.pubkey)];
            const availability = getAvailability(agent.pubkey);
            const label = profile?.displayName?.trim() || agent.name;

            return (
              <AgentIdentityCard
                ariaLabel={`${label} agent profile`}
                avatar={
                  <div className="relative h-24 w-24">
                    <ProfileAvatar
                      avatarUrl={profile?.avatarUrl ?? null}
                      className="h-full w-full border-[3px] border-background bg-muted shadow-none"
                      iconClassName="h-8 w-8"
                      label={label}
                      shape="squircle"
                    />
                    {availability ? (
                      <span
                        aria-label={`${label}: ${getPresenceLabel(availability)}`}
                        className="absolute -right-0.5 -bottom-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-background"
                        data-testid={`community-agent-presence-${agent.pubkey}`}
                        role="img"
                      >
                        <PresenceDot
                          className="h-4 w-4"
                          status={availability}
                        />
                      </span>
                    ) : null}
                  </div>
                }
                dataTestId={`community-agent-${agent.pubkey}`}
                key={agent.pubkey}
                label={label}
                subtitle={communityAgentChannelSummary(agent, channelIdToName)}
                onClick={() => onOpenAgentProfile(agent.pubkey)}
              />
            );
          })}
        </div>
      ) : null}

      {!isLoading && !error && visibleAgents.length === 0 ? (
        <p className="rounded-2xl border border-dashed border-border/60 px-4 py-6 text-sm text-muted-foreground">
          No other agents are available in this community.
        </p>
      ) : null}

      {error ? (
        <p
          className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          role="alert"
        >
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
