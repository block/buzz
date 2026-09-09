import { AlertTriangle } from "lucide-react";

import type { RecentAgentTurnFailure } from "@/features/agents/recentAgentTurnFailuresStore";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const dispositionCopy: Record<RecentAgentTurnFailure["disposition"], string> = {
  retrying: "Retrying automatically",
  dead_lettered: "Stopped after multiple attempts",
  action_required: "Action required",
  respawning: "Restarting the agent",
  stopped: "Stopped",
};

type AgentTurnFailureStatusProps = {
  agents: BotActivityAgent[];
  failure: RecentAgentTurnFailure;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
};

export function AgentTurnFailureStatus({
  agents,
  failure,
  onOpenAgentSession,
  profiles,
}: AgentTurnFailureStatusProps) {
  const agent = agents.find(
    (candidate) =>
      candidate.pubkey.toLowerCase() === failure.agentPubkey.toLowerCase(),
  );
  const name = agent?.name ?? "Agent";

  return (
    <button
      aria-label={`${name} failed. ${dispositionCopy[failure.disposition]}. View activity.`}
      className="flex min-w-0 max-w-full items-center gap-2 text-left text-xs text-destructive transition-opacity hover:opacity-80"
      data-testid="agent-turn-failure-status"
      onClick={() => onOpenAgentSession(failure.agentPubkey, failure.channelId)}
      title={failure.error}
      type="button"
    >
      <UserAvatar
        avatarUrl={
          profiles?.[failure.agentPubkey.toLowerCase()]?.avatarUrl ?? null
        }
        displayName={name}
        shape="squircle"
        size="xs"
      />
      <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
      <span className="min-w-0 truncate">
        <span className="font-medium">{name} couldn&apos;t finish</span>
        <span className="text-muted-foreground">
          {" "}
          · {dispositionCopy[failure.disposition]}
          {failure.attempt ? ` · attempt ${failure.attempt}` : ""}
        </span>
      </span>
    </button>
  );
}
