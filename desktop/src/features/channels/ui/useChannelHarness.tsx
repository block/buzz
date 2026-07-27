import * as React from "react";
import { TerminalSquare } from "lucide-react";
import { toast } from "sonner";

import { HarnessModeView } from "@/features/agents/ui/HarnessModeView";
import { resolveThreadHarnessAgentPubkey } from "@/features/agents/ui/threadHarnessTarget";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

type HarnessSend = (
  content: string,
  mentionPubkeys: string[],
  mediaTags?: string[][],
  channelId?: string | null,
) => Promise<void>;

type UseChannelHarnessOptions = {
  activeChannel: Channel | null;
  composerDisabled: boolean;
  currentPubkey?: string;
  harnessOpen: boolean;
  isSending: boolean;
  onSend: HarnessSend;
  profiles?: UserProfileLookup;
  typingPubkeys: readonly string[];
  activeChannelId: string | null;
  agentSessionAgents: readonly { pubkey: string }[];
  onHarnessOpenChange?: (open: boolean) => void;
  onOpenHarnessForAgent?: (
    agentPubkey: string,
    channelId?: string | null,
  ) => void;
  selectedAgent:
    | (React.ComponentProps<typeof HarnessModeView>["agent"] & {
        canInterruptTurn: boolean;
      })
    | null;
  threadHeadMessage: TimelineMessage | null;
  threadMessages?: readonly MainTimelineEntry[];
};

/**
 * Harness-mode wiring for the channel pane.
 *
 * Extracted from `ChannelPane` to keep that file under the desktop file-size
 * guard, and because none of this is channel-pane concern beyond being where
 * the thread and the agent list happen to meet.
 */
export function useChannelHarness({
  activeChannel,
  composerDisabled,
  currentPubkey,
  harnessOpen,
  isSending,
  onSend,
  profiles,
  typingPubkeys,
  activeChannelId,
  agentSessionAgents,
  onHarnessOpenChange,
  onOpenHarnessForAgent,
  selectedAgent,
  threadHeadMessage,
  threadMessages,
}: UseChannelHarnessOptions) {
  const enterHarness = React.useCallback(() => {
    onHarnessOpenChange?.(true);
  }, [onHarnessOpenChange]);

  const exitHarness = React.useCallback(() => {
    onHarnessOpenChange?.(false);
  }, [onHarnessOpenChange]);

  // Every message in the open thread, head first — the shared input for the
  // history rail and the injected transcript rows.
  const threadTimeline = React.useMemo(() => {
    if (!threadHeadMessage) {
      return null;
    }
    return [
      threadHeadMessage,
      ...(threadMessages ?? []).map((entry) => entry.message),
    ];
  }, [threadHeadMessage, threadMessages]);

  // The thread's affordance targets whichever known agent the thread involves.
  // Scanning head-first keeps it pointed at the agent the thread started with
  // as replies accumulate.
  const threadAgentPubkey = React.useMemo(() => {
    if (!onOpenHarnessForAgent || !threadTimeline) {
      return null;
    }
    return resolveThreadHarnessAgentPubkey({
      messages: threadTimeline,
      agentPubkeys: agentSessionAgents.map((agent) => agent.pubkey),
    });
  }, [agentSessionAgents, onOpenHarnessForAgent, threadTimeline]);

  const threadAction = React.useMemo(() => {
    if (!threadAgentPubkey) {
      return null;
    }
    return (
      <Button
        aria-label="Open harness mode"
        data-testid="thread-enter-harness"
        onClick={() =>
          onOpenHarnessForAgent?.(threadAgentPubkey, activeChannelId)
        }
        size="icon"
        title="Harness mode — full-screen agent session"
        type="button"
        variant="ghost"
      >
        <TerminalSquare />
      </Button>
    );
  }, [activeChannelId, onOpenHarnessForAgent, threadAgentPubkey]);

  // The agent's own replies feed the transcript; human messages feed the
  // history rail. Splitting here avoids showing the agent's output twice.
  const agentMessages = React.useMemo(
    () => threadTimeline?.filter((message) => message.isAgent),
    [threadTimeline],
  );

  const humanMessages = React.useMemo(
    () => threadTimeline?.filter((message) => !message.isAgent),
    [threadTimeline],
  );

  // Mirrors the agent-session panel's stop-turn affordance: the relay only
  // forwards the control frame, so success means "signal sent", not "stopped".
  const cancelTurn = React.useCallback(async () => {
    if (!selectedAgent || !activeChannel) {
      return;
    }
    try {
      await cancelManagedAgentTurn(selectedAgent.pubkey, activeChannel.id);
      toast.success(
        `Stop signal sent to ${selectedAgent.name}. It may take a moment to respond.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to stop ${selectedAgent.name}'s current turn.`,
      );
    }
  }, [activeChannel, selectedAgent]);

  const overlay =
    harnessOpen && selectedAgent && activeChannel ? (
      <HarnessModeView
        agent={selectedAgent}
        agentMessages={agentMessages}
        canCancelTurn={selectedAgent.canInterruptTurn}
        channelId={activeChannel.id}
        channelName={activeChannel.name}
        composerDisabled={composerDisabled}
        currentUserPubkey={currentPubkey ?? null}
        isSending={isSending}
        onCancelTurn={cancelTurn}
        onExit={exitHarness}
        onSend={onSend}
        profiles={profiles}
        threadMessages={humanMessages}
        typingPubkeys={typingPubkeys}
      />
    ) : null;

  return { enterHarness, overlay, threadAction };
}
