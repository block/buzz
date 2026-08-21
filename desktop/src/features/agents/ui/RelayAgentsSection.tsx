import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";

export function RelayAgentsSection({
  agents,
  error,
  isLoading,
  onOpenAgentProfile,
}: {
  agents: readonly RelayAgent[];
  error: Error | null;
  isLoading: boolean;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  return (
    <section
      className="relative space-y-4"
      data-testid="relay-agents-directory"
    >
      {isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <IdentityCardSkeleton />
          <IdentityCardSkeleton />
          <IdentityCardSkeleton />
        </div>
      ) : null}

      {!isLoading && agents.length > 0 ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <RelayAgentCard
              agent={agent}
              key={agent.pubkey}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : null}

      {!isLoading && agents.length === 0 && !error ? (
        <p className="rounded-2xl border border-border/70 bg-muted/40 px-4 py-6 text-sm text-muted-foreground">
          No relay agents are available to your identity yet.
        </p>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function RelayAgentCard({
  agent,
  onOpenAgentProfile,
}: {
  agent: RelayAgent;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const statusLabel =
    agent.status === "online"
      ? "Online"
      : agent.status === "away"
        ? "Away"
        : "Offline";

  return (
    <AgentIdentityCard
      ariaLabel={`${agent.name} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`relay-agent-row-${agent.pubkey}`}
      label={agent.name}
      modelLabel={agent.agentType || null}
      onClick={() => onOpenAgentProfile(agent.pubkey)}
      statusBadge={
        <Badge variant={agent.status === "online" ? "success" : "secondary"}>
          {statusLabel}
        </Badge>
      }
    />
  );
}
