import { ArrowLeft, CircleDot, Octagon, X } from "lucide-react";
import { toast } from "sonner";

import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { useStickToBottom } from "@/shared/hooks/useStickToBottom";
import { cn } from "@/shared/lib/cn";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  auxiliaryPanelContentPaddingClass,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  OverlayPanelBackdrop,
  PANEL_ENTER_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  channel: Channel | null;
  canInterruptTurn: boolean;
  isWorking: boolean;
  layout?: "standalone" | "split";
  isSinglePanelView?: boolean;
  profiles?: UserProfileLookup;
  onBackToProfile: () => void;
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
  onBackToProfile,
  onClose,
  widthPx,
  transparentChrome = false,
}: AgentSessionThreadPanelProps) {
  const isLive = isManagedAgentActive(agent);
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const { ref: scrollRef, onScroll } = useStickToBottom<HTMLDivElement>();

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
    <div className="ml-auto flex shrink-0 items-center gap-2">
      {isLive && isWorking ? (
        <Badge className="shrink-0 gap-1 px-2 py-0 text-2xs" variant="default">
          <CircleDot className="h-2.5 w-2.5" />
          Live
        </Badge>
      ) : null}
      {isLive && isWorking ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Stop current agent turn"
              className="h-6 px-2 text-2xs"
              data-testid="agent-session-stop-turn"
              disabled={!canInterruptTurn}
              onClick={() => {
                void handleInterruptTurn();
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              <Octagon className="h-4 w-4" />
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
      <Button
        aria-label="Close activity panel"
        data-testid="agent-session-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );

  const agentHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup>
        <Button
          aria-label="Back from activity"
          className="shrink-0"
          data-testid="agent-session-back"
          onClick={onBackToProfile}
          size="icon"
          type="button"
          variant="outline"
        >
          <ArrowLeft />
        </Button>
        <AuxiliaryPanelTitle>Activity</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
      {agentHeaderActions}
    </>
  );

  const agentBody = (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className={cn(
        "min-h-0 flex-1 overflow-y-auto px-3 pb-4",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && (isFloatingOverlay ? "pt-4" : "pt-[3.25rem]"),
      )}
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
        showHeader={false}
        showRaw={false}
      />
    </div>
  );

  if (isSplitLayout) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <AuxiliaryPanelHeader transparent={transparentChrome}>
          {agentHeaderContent}
        </AuxiliaryPanelHeader>
        {agentBody}
      </div>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_ENTER_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="agent-session-thread-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`,
        }}
      >
        {!isOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 z-40 h-[3.25rem] bg-background/75 backdrop-blur-md supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35"
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[3.25rem] min-h-[3.25rem] shrink-0 gap-2.5 bg-background/80 px-4 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:pl-6 sm:pr-3 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55`
              : "relative z-50 min-h-[3.25rem] shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
          )}
          data-tauri-drag-region
        >
          {agentHeaderContent}
        </div>

        {agentBody}
      </aside>
    </>
  );
}
