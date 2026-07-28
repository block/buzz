import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RelayAgent } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { AgentIdentityCard } from "./AgentIdentityCard";

type ConnectedRelayAgentsSectionProps = {
  agents: RelayAgent[];
  error: Error | null;
  isLoading: boolean;
  onOpenAgentProfile: (pubkey: string) => void;
};

const CARD_GRID_CLASS =
  "mx-auto grid w-full max-w-[996px] grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-center gap-3";

export function ConnectedRelayAgentsSection({
  agents,
  error,
  isLoading,
  onOpenAgentProfile,
}: ConnectedRelayAgentsSectionProps) {
  if (!isLoading && agents.length === 0 && !error) return null;

  return (
    <section className="space-y-4" data-testid="connected-relay-agents">
      <div className="mx-auto w-full max-w-[996px]">
        <h2 className="text-base font-semibold">Connected agents</h2>
        <p className="text-sm text-muted-foreground">
          Agents connected through this community relay.
        </p>
      </div>

      {isLoading ? (
        <div className={CARD_GRID_CLASS}>
          {["first", "second", "third", "fourth"].map((key) => (
            <IdentityCardSkeleton key={key} />
          ))}
        </div>
      ) : (
        <div className={CARD_GRID_CLASS}>
          {agents.map((agent) => (
            <ConnectedRelayAgentCard
              agent={agent}
              key={agent.pubkey}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      )}

      {error ? (
        <p className="mx-auto w-full max-w-[996px] rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function ConnectedRelayAgentCard({
  agent,
  onOpenAgentProfile,
}: {
  agent: RelayAgent;
  onOpenAgentProfile: (pubkey: string) => void;
}) {
  const profileQuery = useUserProfileQuery(agent.pubkey);
  const title = profileQuery.data?.displayName?.trim() || agent.name;

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`relay-agent-${agent.pubkey}`}
      label={title}
      modelLabel="Relay agent"
      onClick={() => onOpenAgentProfile(agent.pubkey)}
      statusBadge={
        <Badge variant={agent.status === "online" ? "success" : "secondary"}>
          {agent.status === "online" ? "Connected" : agent.status}
        </Badge>
      }
    />
  );
}
