import { ChevronDown, ChevronRight } from "lucide-react";

import { useUserProfileQuery } from "@/features/profile/hooks";
import type { RelayAgent } from "@/shared/api/types";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { AgentIdentityCard } from "./AgentIdentityCard";
import { OtherSetupAgentMarker } from "./OtherSetupAgentMarker";

export const RUNNING_ELSEWHERE_GROUP_KEY = "__running_elsewhere__";

/**
 * Agents the viewer owns (verified NIP-OA profile plus owner policy) that this
 * device does not manage: provider-deployed from another machine, or run by a
 * service the owner uses. Read-only: no Start, Stop or edit controls, because
 * this device holds neither the agent's key nor its runtime. A card opens the
 * exact-key profile, which already supports relay-only identities.
 */
export function RunningElsewhereAgentsGroup({
  agents,
  collapsed,
  gridClassName,
  onOpenAgentProfile,
  onToggle,
}: {
  agents: readonly RelayAgent[];
  collapsed: ReadonlySet<string>;
  gridClassName: string;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onToggle: (key: string) => void;
}) {
  if (agents.length === 0) return null;
  const isCollapsed = collapsed.has(RUNNING_ELSEWHERE_GROUP_KEY);
  return (
    <div className="w-full space-y-2" data-testid="running-elsewhere-agents">
      <button
        className="group flex items-center gap-2 rounded-md px-1 py-1 text-left transition-colors hover:bg-muted/50"
        onClick={() => onToggle(RUNNING_ELSEWHERE_GROUP_KEY)}
        type="button"
      >
        {isCollapsed ? (
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
        ) : (
          <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
        )}
        <span className="text-sm font-medium">Running elsewhere</span>
        <span className="text-xs text-muted-foreground">({agents.length})</span>
      </button>
      {!isCollapsed ? (
        <div className={gridClassName}>
          {agents.map((agent) => (
            <RunningElsewhereAgentCard
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

function RunningElsewhereAgentCard({
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
  const title = agent.name;
  return (
    <AgentIdentityCard
      ariaLabel={`${title} agent profile`}
      avatarUrl={profileQuery.data?.avatarUrl}
      dataTestId={`running-elsewhere-agent-${agent.pubkey}`}
      footerAccessory={
        <OtherSetupAgentMarker
          testId={`running-elsewhere-marker-${agent.pubkey}`}
        />
      }
      label={title}
      subtitle={profileQuery.data?.about?.trim() || null}
      onClick={() => {
        onOpenAgentProfile(agent.pubkey);
      }}
    />
  );
}
