import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { Badge } from "@/shared/ui/badge";
import { AgentIdentityCard } from "./AgentIdentityCard";

const CARD_GRID_CLASS =
  "w-full mx-auto grid max-w-[996px] grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-center gap-3";

/**
 * Agents that announced themselves on the relay (kind:10100) but are not
 * managed by this desktop — e.g. harnesses running on a server. Read-only:
 * their lifecycle is owned by wherever they run, so there are no start/stop
 * controls; the cards open the agent's profile panel.
 */
export function RelayAgentsSection({
  managedPubkeys,
  onOpenAgentProfile,
}: {
  managedPubkeys: ReadonlySet<string>;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const [isCollapsed, setIsCollapsed] = React.useState(false);

  const externalAgents = React.useMemo(
    () =>
      (relayAgentsQuery.data ?? [])
        .filter((agent) => !managedPubkeys.has(agent.pubkey))
        .sort((left, right) => left.name.localeCompare(right.name)),
    [relayAgentsQuery.data, managedPubkeys],
  );

  if (externalAgents.length === 0) return null;

  return (
    <div className="w-full space-y-2" data-testid="relay-agents-section">
      <button
        className="group flex items-center gap-2 rounded-md px-1 py-1 text-left transition-colors hover:bg-muted/50"
        onClick={() => setIsCollapsed((prev) => !prev)}
        type="button"
      >
        {isCollapsed ? (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
        ) : (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <span className="text-sm font-medium">Relay agents</span>
        <span className="text-xs text-muted-foreground">
          ({externalAgents.length})
        </span>
      </button>
      {!isCollapsed ? (
        <div className={CARD_GRID_CLASS}>
          {externalAgents.map((agent) => (
            <RelayAgentCard
              agent={agent}
              key={agent.pubkey}
              onOpenAgentProfile={onOpenAgentProfile}
            />
          ))}
        </div>
      ) : null}
    </div>
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
  const title = profileQuery.data?.displayName?.trim() || agent.name;

  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl ?? null}
      dataTestId={`relay-agent-${agent.pubkey}`}
      label={title}
      modelLabel="Runs externally"
      onClick={() => onOpenAgentProfile(agent.pubkey)}
      statusBadge={
        agent.status === "online" ? (
          <Badge className="gap-1" variant="success">
            Online
          </Badge>
        ) : (
          <Badge className="gap-1" variant="secondary">
            {agent.status === "away" ? "Away" : "Offline"}
          </Badge>
        )
      }
    />
  );
}
