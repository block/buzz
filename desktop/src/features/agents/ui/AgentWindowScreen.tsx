import * as React from "react";
import { CircleDot, Octagon } from "lucide-react";
import { toast } from "sonner";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  useActiveAgentTurns,
  useActiveAgentTurnsBridge,
} from "@/features/agents/activeAgentTurnsStore";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { useChannelsQuery } from "@/features/channels/hooks";
import { mergeAgentNamesIntoProfiles } from "@/features/channels/ui/useChannelActivityTyping";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelType, ManagedAgent } from "@/shared/api/types";
import { useStickToBottom } from "@/shared/hooks/useStickToBottom";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentWindowScreenProps = {
  channelId: string;
  agentPubkey: string;
  initialAgentName: string;
  initialChannelName: string;
  channelType: ChannelType | null;
};

/**
 * Chrome-less conversation view that fills its own OS window (opened via
 * `openAgentConversationWindow`). Renders the agent's live activity log and a
 * composer wired to the normal channel send flow, with the target agent always
 * mentioned so it responds.
 */
export function AgentWindowScreen({
  channelId,
  agentPubkey,
  initialAgentName,
  initialChannelName,
  channelType,
}: AgentWindowScreenProps) {
  const { ref: scrollRef, onScroll } = useStickToBottom<HTMLDivElement>();

  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();

  const managedAgents = React.useMemo(
    () => managedAgentsQuery.data ?? [],
    [managedAgentsQuery.data],
  );
  const relayAgents = React.useMemo(
    () => relayAgentsQuery.data ?? [],
    [relayAgentsQuery.data],
  );

  // Prefer live agent metadata, falling back to the snapshot passed at open.
  const liveManagedAgent = React.useMemo(
    () =>
      managedAgents.find(
        (entry) =>
          normalizePubkey(entry.pubkey) === normalizePubkey(agentPubkey),
      ) ?? null,
    [managedAgents, agentPubkey],
  );
  const agentName = liveManagedAgent?.name ?? initialAgentName;
  const agentStatus: ManagedAgent["status"] =
    liveManagedAgent?.status ?? "running";
  const canInterruptTurn = liveManagedAgent?.backend.type !== "provider";

  const sessionAgent = React.useMemo(
    () => ({ pubkey: agentPubkey, name: agentName, status: agentStatus }),
    [agentPubkey, agentName, agentStatus],
  );

  // This window owns its own observer subscription for the target agent.
  useManagedAgentObserverBridge(
    React.useMemo(
      () => [{ pubkey: agentPubkey, status: agentStatus }],
      [agentPubkey, agentStatus],
    ),
  );

  useActiveAgentTurnsBridge(
    React.useMemo(
      () => [{ pubkey: agentPubkey, status: agentStatus }],
      [agentPubkey, agentStatus],
    ),
  );
  const activeTurns = useActiveAgentTurns(agentPubkey);
  const isWorking = React.useMemo(
    () => activeTurns.some((turn) => turn.channelId === channelId),
    [activeTurns, channelId],
  );

  const channel = React.useMemo(
    () =>
      (channelsQuery.data ?? []).find((entry) => entry.id === channelId) ??
      null,
    [channelsQuery.data, channelId],
  );
  const channelName = channel?.name ?? initialChannelName;

  const profilesQuery = useUsersBatchQuery(
    React.useMemo(
      () =>
        [agentPubkey, identityQuery.data?.pubkey].filter(
          (value): value is string => Boolean(value),
        ),
      [agentPubkey, identityQuery.data?.pubkey],
    ),
  );
  const profiles = React.useMemo(
    () =>
      mergeAgentNamesIntoProfiles(
        { ...(profilesQuery.data?.profiles ?? {}) },
        managedAgents,
        relayAgents,
      ),
    [profilesQuery.data?.profiles, managedAgents, relayAgents],
  );
  const agentAvatarUrl =
    profiles[normalizePubkey(agentPubkey)]?.avatarUrl ?? null;

  const sendMessageMutation = useSendMessageMutation(
    channel,
    identityQuery.data,
  );

  // Anything sent from this window is addressed to the agent: always include
  // its pubkey as a mention so it responds, on top of MessageComposer's normal
  // mention flow.
  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const merged = Array.from(
        new Set([
          ...mentionPubkeys.map(normalizePubkey),
          normalizePubkey(agentPubkey),
        ]),
      );
      await sendMessageMutation.mutateAsync({
        content,
        mentionPubkeys: merged,
        mediaTags,
      });
    },
    [sendMessageMutation, agentPubkey],
  );

  const composerDisabled =
    !channel ||
    !identityQuery.data ||
    channel.archivedAt !== null ||
    channel.channelType === "forum";

  async function handleInterruptTurn() {
    try {
      await cancelManagedAgentTurn(agentPubkey, channelId);
      toast.success(
        `Stop signal sent to ${agentName}. It may take a moment to respond.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to stop ${agentName}'s current turn.`,
      );
    }
  }

  const isLive = isManagedAgentActive(sessionAgent);

  return (
    <div
      className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background"
      data-testid="agent-window-screen"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-border/60 bg-muted/30 px-3 py-2">
        <UserAvatar
          avatarUrl={agentAvatarUrl}
          className="!h-7 !w-7 shrink-0 text-[10px]"
          displayName={agentName}
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold leading-tight">
            {agentName}
          </p>
          <p className="truncate text-[11px] leading-tight text-muted-foreground">
            #{channelName}
          </p>
        </div>
        {isLive && isWorking ? (
          <Badge
            className="shrink-0 gap-1 px-2 py-0 text-[10px]"
            variant="default"
          >
            <CircleDot className="h-2.5 w-2.5" />
            Live
          </Badge>
        ) : null}
        {isLive && isWorking ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Stop current agent turn"
                className="h-7 px-2 text-[11px]"
                data-testid="agent-window-stop"
                disabled={!canInterruptTurn}
                onClick={() => {
                  void handleInterruptTurn();
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                <Octagon className="h-3 w-3" />
                Stop
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="text-xs">
              {canInterruptTurn
                ? "Interrupt the current ACP turn without stopping the agent process."
                : "This agent cannot be interrupted from this workspace."}
            </TooltipContent>
          </Tooltip>
        ) : null}
      </header>

      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-3"
      >
        <ManagedAgentSessionPanel
          agent={sessionAgent}
          channelId={channelId}
          className="border-0 bg-transparent p-0 shadow-none"
          emptyDescription={`Send a message below to start working with ${agentName}.`}
          profiles={profiles}
          showHeader={false}
          showRaw={false}
        />
      </div>

      <div className="shrink-0 border-t border-border/60">
        <MessageComposer
          channelId={channelId}
          channelName={channelName}
          channelType={channelType}
          disabled={composerDisabled}
          draftKey={`agent-window:${channelId}:${normalizePubkey(agentPubkey)}`}
          isSending={sendMessageMutation.isPending}
          onSend={handleSend}
          placeholder={
            channel?.archivedAt
              ? "This channel is archived."
              : `Message ${agentName}`
          }
          profiles={profiles}
        />
      </div>
    </div>
  );
}
