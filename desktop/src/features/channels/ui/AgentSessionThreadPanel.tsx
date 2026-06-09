import type * as React from "react";
import { CircleDot, Octagon, X } from "lucide-react";
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
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { ChannelAgentSessionAgent } from "./useChannelAgentSessions";

type AgentSessionThreadPanelProps = {
  agent: ChannelAgentSessionAgent;
  canResetWidth: boolean;
  channel: Channel;
  canInterruptTurn: boolean;
  isWorking: boolean;
  isSinglePanelView?: boolean;
  profiles?: UserProfileLookup;
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
  isSinglePanelView = false,
  profiles,
  onClose,
  onResetWidth,
  onResizeStart,
  widthPx,
}: AgentSessionThreadPanelProps) {
  const isLive = isManagedAgentActive(agent);
  const avatarUrl = profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const usesChannelSplitChrome = !isOverlay && !isSinglePanelView;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

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
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
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
        {!isOverlay && !isSinglePanelView && (
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
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 z-40 bg-background/75 backdrop-blur-md before:absolute before:left-0 before:right-0 before:top-10 before:h-px before:bg-border/35 after:absolute after:bottom-0 after:-left-px after:top-10 after:w-px after:bg-border/80 supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35",
              usesChannelSplitChrome ? "h-[92px]" : "h-[76px]",
            )}
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[76px] min-h-[76px] shrink-0 gap-[10px] bg-background/80 pb-[4px] pl-[16px] pr-[8px] pt-[42px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:pl-[24px] sm:pr-[12px] dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55`
              : isOverlay
                ? "relative z-50 min-h-[44px] shrink-0 gap-3 bg-background/80 px-3 py-[6px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : "absolute inset-x-0 top-[48px] z-50 h-[32px] gap-[10px] py-0 pl-[16px] pr-[8px] sm:pr-[12px]",
          )}
          data-tauri-drag-region
        >
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <div
              className={cn(
                "flex min-w-0 flex-1 items-center gap-2",
                isSinglePanelView && "translate-y-px",
              )}
            >
              <UserAvatar
                avatarUrl={avatarUrl}
                className="h-5 w-5 shrink-0 text-[8px]"
                displayName={agent.name}
                size="xs"
              />
              <h2
                className={cn(
                  "min-w-0 flex-1 translate-y-px truncate font-semibold tracking-tight",
                  usesChannelSplitChrome
                    ? "text-base leading-6"
                    : "text-sm leading-5",
                )}
              >
                {agent.name}
              </h2>
            </div>
            <div
              className={cn(
                "flex shrink-0 items-center gap-2",
                isSinglePanelView && "translate-y-px",
              )}
            >
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
                      className="h-6 px-2 text-[11px]"
                      data-testid="agent-session-stop-turn"
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
              <Button
                aria-label="Close activity panel"
                className={cn(
                  usesChannelSplitChrome
                    ? "h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
                    : "h-6 w-6 text-foreground hover:bg-muted/60 hover:text-foreground",
                )}
                data-testid="agent-session-close"
                onClick={onClose}
                size="icon"
                type="button"
                variant="ghost"
              >
                <X
                  className={cn(
                    usesChannelSplitChrome ? "size-5" : "h-3.5 w-3.5",
                  )}
                />
              </Button>
            </div>
          </div>
        </div>

        <div
          ref={scrollRef}
          onScroll={onScroll}
          className={cn(
            "min-h-0 flex-1 overflow-y-auto px-3 pb-4",
            // Single-panel mode keeps the 76px local header; split panes sit
            // under the channel screen's 92px top chrome.
            usesChannelSplitChrome
              ? "pt-[92px]"
              : isOverlay
                ? "pt-4"
                : "pt-[76px]",
          )}
        >
          <ManagedAgentSessionPanel
            agent={agent}
            channelId={channel.id}
            className="border-0 bg-transparent p-0 shadow-none"
            emptyDescription={`Mention ${agent.name} in the channel to see its work here.`}
            profiles={profiles}
            showHeader={false}
            showRaw={false}
          />
        </div>
      </aside>
    </>
  );
}
