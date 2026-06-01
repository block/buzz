import type * as React from "react";
import { Activity, Bot, CircleDot, Octagon, X } from "lucide-react";
import { toast } from "sonner";

import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { cancelManagedAgentTurn } from "@/shared/api/agentControl";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { useStickToBottom } from "@/shared/hooks/useStickToBottom";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  canResetWidth: boolean;
  channel: Channel;
  canInterruptTurn: boolean;
  isWorking: boolean;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  widthPx: number;
};

export function AgentSessionThreadPanel({
  agent,
  canResetWidth,
  canInterruptTurn,
  channel,
  isWorking,
  onClose,
  onResetWidth,
  onResizeStart,
  widthPx,
}: AgentSessionThreadPanelProps) {
  const isLive = isManagedAgentActive(agent);
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay);

  const { ref: scrollRef, onScroll } = useStickToBottom<HTMLDivElement>();

  async function handleInterruptTurn() {
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

  return (
    <>
      {isOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(PANEL_BASE_CLASS, isOverlay && PANEL_OVERLAY_CLASS)}
        data-testid="agent-session-thread-panel"
        style={{ width: `${widthPx}px` }}
      >
        {!isOverlay && (
          <button
            aria-label="Resize agent session panel"
            className="group absolute inset-y-0 left-0 z-20 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="agent-session-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80" />
          </button>
        )}

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 z-40 h-[76px] bg-background/75 backdrop-blur-md after:absolute after:bottom-0 after:left-0 after:top-10 after:w-px after:bg-border/80 supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35"
          />
        ) : null}

        <div
          className={cn(
            "z-50 flex cursor-default select-none items-center gap-3 px-4",
            isOverlay
              ? "relative min-h-[44px] shrink-0 border-b border-border/70 bg-background/80 py-2.5 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
              : "absolute inset-x-0 top-11 min-h-[32px] py-[4px]",
          )}
          data-tauri-drag-region
        >
          <Bot className="h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-sm font-semibold tracking-tight">
              {agent.name}
            </h2>
            <div className="flex items-center gap-1.5">
              <Activity className="h-3 w-3 text-muted-foreground" />
              <p className="truncate text-xs text-muted-foreground">
                Agent activity log
              </p>
            </div>
          </div>
          {isLive ? (
            <Badge className="shrink-0 gap-1" variant="default">
              <CircleDot className="h-3 w-3" />
              Live
            </Badge>
          ) : (
            <Badge className="shrink-0" variant="secondary">
              Idle
            </Badge>
          )}
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Stop current agent turn"
                data-testid="agent-session-stop-turn"
                disabled={!canInterruptTurn || !isLive || !isWorking}
                onClick={() => {
                  void handleInterruptTurn();
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                <Octagon className="h-3.5 w-3.5" />
                Stop
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="text-xs">
              {isWorking
                ? canInterruptTurn
                  ? "Interrupt the current ACP turn without stopping the agent process."
                  : "This agent cannot be interrupted from this workspace."
                : "No active turn to interrupt."}
            </TooltipContent>
          </Tooltip>
          <Button
            aria-label="Close activity panel"
            data-testid="agent-session-close"
            onClick={onClose}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div
          ref={scrollRef}
          onScroll={onScroll}
          className={cn(
            "min-h-0 flex-1 overflow-y-auto px-3 pb-4",
            isOverlay ? "pt-4" : "pt-[76px]",
          )}
        >
          <ManagedAgentSessionPanel
            agent={agent}
            channelId={channel.id}
            className="border-0 bg-transparent p-0 shadow-none"
            emptyDescription={`Mention ${agent.name} in the channel to see its work here.`}
            showHeader={false}
            showRaw={false}
          />
        </div>
      </aside>
    </>
  );
}
