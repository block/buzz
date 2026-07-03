import * as React from "react";
import { Octagon, Settings, TerminalSquare } from "lucide-react";
import { toast } from "sonner";

import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { useStickToBottom } from "@/shared/hooks/useStickToBottom";
import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { AuxiliaryPanelBody } from "@/shared/layout/AuxiliaryPanel";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import { Button } from "@/shared/ui/button";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  channel: Channel | null;
  canInterruptTurn: boolean;
  isWorking: boolean;
  layout?: "standalone" | "split";
  isSinglePanelView?: boolean;
  profiles?: UserProfileLookup;
  /**
   * Fired by the header back arrow. Restores the pane this panel replaced
   * (thread or profile) via the captured return target — see
   * useChannelAgentSessions.backFromAgentSession. Omit when there is no
   * target (composer/no-pane open, direct/restored URL): the arrow hides
   * and the close affordance is the fallback.
   */
  onBack?: () => void;
  onClose: () => void;
  widthPx: number;
  transparentChrome?: boolean;
};

export function AgentSessionThreadPanel({
  agent,
  canInterruptTurn,
  channel,
  isWorking,
  layout = "standalone",
  isSinglePanelView = false,
  profiles,
  onBack,
  onClose,
  widthPx,
  transparentChrome = false,
}: AgentSessionThreadPanelProps) {
  const isLive = isManagedAgentActive(agent);
  const isOverlay = useIsThreadPanelOverlay();
  const canStopCurrentTurn = isWorking && canInterruptTurn;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const { ref: scrollRef, onScroll } = useStickToBottom<HTMLDivElement>();
  const rawFeedScopeKey = `${agent.pubkey}:${channel?.id ?? "all"}`;
  const [rawFeedState, setRawFeedState] = React.useState(() => ({
    scopeKey: rawFeedScopeKey,
    show: false,
  }));
  const showRawFeed =
    rawFeedState.scopeKey === rawFeedScopeKey && rawFeedState.show;
  const handleRawFeedChange = React.useCallback(
    (checked: boolean) => {
      setRawFeedState({ scopeKey: rawFeedScopeKey, show: checked });
    },
    [rawFeedScopeKey],
  );
  async function handleInterruptTurn() {
    if (!channel) {
      return;
    }

    try {
      await cancelManagedAgentTurn(agent.pubkey, channel.id);
      toast.success(
        `Stop signal sent to ${agent.name}. It may take a moment to respond.`,
      );
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Failed to stop ${agent.name}'s current turn.`,
      );
    }
  }

  const agentHeaderActions = (
    <AuxiliaryPanelHeaderActions>
      {isLive ? (
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Open activity settings"
              className="relative"
              data-testid="agent-session-settings-menu-trigger"
              size="icon"
              title="Activity settings"
              type="button"
              variant="ghost"
            >
              <Settings />
              {canStopCurrentTurn ? (
                <span
                  aria-hidden="true"
                  className="absolute right-1 bottom-1 h-2 w-2 rounded-full bg-primary ring-2 ring-background"
                  data-testid="agent-session-settings-live-badge"
                />
              ) : null}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className="min-w-56"
            onCloseAutoFocus={(event) => event.preventDefault()}
          >
            <DropdownMenuCheckboxItem
              checked={showRawFeed}
              className="items-start gap-3"
              data-testid="agent-session-toggle-raw-feed"
              onCheckedChange={handleRawFeedChange}
              title={
                showRawFeed
                  ? "Hide raw JSON-RPC payloads."
                  : channel
                    ? "Show raw JSON-RPC payloads for this channel."
                    : "Show raw JSON-RPC payloads for this agent."
              }
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm font-medium">
                  <TerminalSquare className="h-4 w-4 text-muted-foreground" />
                  Raw
                </span>
                <span className="mt-0.5 block text-xs text-muted-foreground">
                  Show raw JSON-RPC activity.
                </span>
              </span>
            </DropdownMenuCheckboxItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              className="items-start gap-3"
              data-testid="agent-session-stop-turn"
              disabled={!canStopCurrentTurn}
              onSelect={() => {
                void handleInterruptTurn();
              }}
              title={
                canStopCurrentTurn
                  ? "Interrupt the current ACP turn without stopping the agent process."
                  : isWorking
                    ? "Only locally managed agents can be interrupted from this workspace."
                    : "Available while the agent is working."
              }
            >
              <Octagon className="mt-0.5 h-4 w-4 text-muted-foreground" />
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium">
                  Stop current turn
                </span>
                {!canStopCurrentTurn ? (
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    {isWorking
                      ? "Only available for locally managed agents."
                      : "Available while the agent is working."}
                  </span>
                ) : null}
              </span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </AuxiliaryPanelHeaderActions>
  );

  const agentHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup
        backButtonAriaLabel="Back from activity"
        backButtonTestId="agent-session-back"
        onBack={onBack}
      >
        <AuxiliaryPanelTitle>
          {showRawFeed ? "Raw ACP Activity" : "Activity"}
        </AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
      {agentHeaderActions}
    </>
  );

  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="agent-session-thread-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={layout !== "split" && !isOverlay}
          backdropSurface="soft"
          inset={layout !== "split" ? "wide" : "default"}
        >
          {agentHeaderContent}
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        ref={scrollRef}
        onScroll={onScroll}
        className="overflow-y-auto px-3 pb-4"
        panelPadding
      >
        <ManagedAgentSessionPanel
          agent={agent}
          channelId={channel?.id ?? null}
          className="border-0 bg-transparent p-0 shadow-none"
          emptyDescription={
            channel
              ? `Mention ${agent.name} in the channel to see its work here.`
              : `Mention ${agent.name} in any channel to see its work here.`
          }
          profiles={profiles}
          rawLayout="exclusive"
          showHeader={false}
          showRaw={showRawFeed}
        />
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
