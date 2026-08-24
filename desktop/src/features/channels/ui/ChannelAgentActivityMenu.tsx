import * as React from "react";
import { Activity, Bot, Loader2 } from "lucide-react";

import { useChannelWorkingAgentPubkeys } from "@/features/agents/agentWorkingSignal";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { AgentHandoffDialog } from "@/features/agents/ui/AgentHandoffDialog";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type ChannelAgentActivityMenuProps = {
  agents: readonly ChannelAgentSessionAgent[];
  channelId: string;
  compact: boolean;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  openAgentSessionPubkey: string | null;
};

export function ChannelAgentActivityMenu({
  agents,
  channelId,
  compact,
  onOpenAgentSession,
  openAgentSessionPubkey,
}: ChannelAgentActivityMenuProps) {
  const workingPubkeys = useChannelWorkingAgentPubkeys(channelId);
  const workingSet = new Set(workingPubkeys.map(normalizePubkey));
  const ownerPubkeys = React.useMemo(
    () =>
      agents.flatMap((agent) => (agent.ownerPubkey ? [agent.ownerPubkey] : [])),
    [agents],
  );
  const ownerProfiles = useUsersBatchQuery(ownerPubkeys).data?.profiles;
  const selectedPubkey = openAgentSessionPubkey
    ? normalizePubkey(openAgentSessionPubkey)
    : null;
  const [handoffAgent, setHandoffAgent] =
    React.useState<ChannelAgentSessionAgent | null>(null);

  if (agents.length === 0) {
    return null;
  }

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label="Open agent activity"
            data-testid="channel-agent-activity-menu-trigger"
            size={compact ? "icon" : "sm"}
            title="Agent activity"
            type="button"
            variant={selectedPubkey ? "secondary" : "outline"}
          >
            <Activity />
            {compact ? null : <span>Activity</span>}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-64">
          <DropdownMenuLabel>Agent activity</DropdownMenuLabel>
          {agents.map((agent) => {
            const pubkey = normalizePubkey(agent.pubkey);
            const isWorking = workingSet.has(pubkey);
            const isSelected = selectedPubkey === pubkey;
            return (
              <React.Fragment key={agent.pubkey}>
                <DropdownMenuItem
                  className="gap-2"
                  data-testid={`channel-agent-activity-item-${agent.pubkey}`}
                  onSelect={() => onOpenAgentSession(agent.pubkey, channelId)}
                >
                  <Bot className="text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate">
                    {agent.name}
                    {agent.deleted ? " [已删除]" : ""} ·{" "}
                    {agent.ownerPubkey
                      ? (ownerProfiles?.[agent.ownerPubkey.toLowerCase()]
                          ?.displayName ??
                        `用户 ${truncatePubkey(agent.ownerPubkey)}`)
                      : "未知用户"}{" "}
                    · {truncatePubkey(agent.pubkey)}
                  </span>
                  {isWorking ? (
                    <span className="flex shrink-0 items-center gap-1.5 text-xs text-primary">
                      <Loader2 className="animate-spin" />
                      Working
                    </span>
                  ) : isSelected ? (
                    <span className="shrink-0 text-xs text-muted-foreground">
                      Open
                    </span>
                  ) : null}
                </DropdownMenuItem>
                <DropdownMenuItem
                  className="gap-2 pl-9"
                  data-testid={`channel-agent-handoff-item-${agent.pubkey}`}
                  onSelect={(event) => {
                    event.preventDefault();
                    setHandoffAgent(agent);
                  }}
                >
                  <span className="min-w-0 flex-1 truncate">
                    Handoff history
                  </span>
                </DropdownMenuItem>
              </React.Fragment>
            );
          })}
        </DropdownMenuContent>
      </DropdownMenu>
      {handoffAgent ? (
        <AgentHandoffDialog
          agent={{ name: handoffAgent.name, pubkey: handoffAgent.pubkey }}
          availableAgents={agents}
          channelId={channelId}
          history=""
          initialMode="received"
          onOpenChange={(nextOpen) => {
            if (!nextOpen) {
              setHandoffAgent(null);
            }
          }}
          open
        />
      ) : null}
    </>
  );
}
