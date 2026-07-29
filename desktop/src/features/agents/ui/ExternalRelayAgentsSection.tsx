import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RelayAgent } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";

import { AgentIdentityCard } from "./AgentIdentityCard";

const EXTERNAL_AGENT_CARD_GRID_CLASS =
  "mx-auto grid w-full max-w-[996px] grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-center gap-3";

type ExternalRelayAgentsSectionProps = {
  agents: RelayAgent[];
  error: Error | null;
  isLoading: boolean;
  onOpenProfile: (pubkey: string) => void;
};

export function ExternalRelayAgentsSection({
  agents,
  error,
  isLoading,
  onOpenProfile,
}: ExternalRelayAgentsSectionProps) {
  if (!isLoading && !error && agents.length === 0) {
    return null;
  }

  return (
    <section className="relative space-y-4" data-testid="external-relay-agents">
      <SectionHeader
        action={
          !isLoading && !error ? (
            <Badge variant="outline">{agents.length}</Badge>
          ) : null
        }
        className="mx-auto w-full max-w-[996px]"
        description="Agents hosted outside this Mac that you can interact with through the Relay."
        title="Relay agents"
      />

      {error ? (
        <p
          className="mx-auto w-full max-w-[996px] rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          role="alert"
        >
          Could not load Relay agents: {error.message}
        </p>
      ) : null}

      {isLoading ? (
        <div className={EXTERNAL_AGENT_CARD_GRID_CLASS}>
          <IdentityCardSkeleton footerSubtitleWidthClass="w-20" />
          <IdentityCardSkeleton footerSubtitleWidthClass="w-24" />
          <IdentityCardSkeleton footerSubtitleWidthClass="w-16" />
        </div>
      ) : null}

      {!isLoading && !error ? (
        <div className={EXTERNAL_AGENT_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <ExternalRelayAgentCard
              agent={agent}
              key={agent.pubkey.toLowerCase()}
              onOpenProfile={onOpenProfile}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function ExternalRelayAgentCard({
  agent,
  onOpenProfile,
}: {
  agent: RelayAgent;
  onOpenProfile: (pubkey: string) => void;
}) {
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const channelCount = agent.channelIds.length;

  return (
    <AgentIdentityCard
      ariaLabel={`${agent.name} Relay agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`external-relay-agent-${agent.pubkey}`}
      label={agent.name}
      modelLabel={`External · ${
        agent.registryState === "failed" ? "Runner failed · " : ""
      }${channelCount} channel${channelCount === 1 ? "" : "s"}`}
      onClick={() => onOpenProfile(agent.pubkey)}
      statusBadge={
        <Badge
          className="mt-1 w-fit gap-1.5 normal-case tracking-normal"
          variant={
            agent.status === "online"
              ? "success"
              : agent.status === "away"
                ? "warning"
                : "secondary"
          }
        >
          <PresenceDot className="h-2 w-2" status={agent.status} />
          {getPresenceLabel(agent.status)}
        </Badge>
      }
    />
  );
}
