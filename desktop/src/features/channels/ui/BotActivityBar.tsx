import * as React from "react";
import { Loader2 } from "lucide-react";

import {
  useActiveAgentPubkeysForChannel,
  useActiveAgentTurns,
} from "@/features/agents/activeAgentTurnsStore";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import {
  getActivityHeadline,
  isMeaningfulItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import { TranscriptActivityItem } from "@/features/agents/ui/activityRenderClasses/TranscriptActivityItem";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Shimmer } from "@/shared/ui/Shimmer";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type BotActivityAgent = Pick<ManagedAgent, "pubkey" | "name">;

type BotActivityBarProps = {
  agents: BotActivityAgent[];
  channelId?: string | null;
  onOpenAgentSession: (pubkey: string) => void;
  openAgentSessionPubkey: string | null;
  profiles?: UserProfileLookup;
  typingBotPubkeys: string[];
  variant?: "toolbar" | "inline";
};

const HOVER_OPEN_DELAY_MS = 150;
const HOVER_CLOSE_DELAY_MS = 180;
const HEADLINE_ROTATION_MS = 2200;
const PREVIEW_ITEM_LIMIT = 8;

type ComposerPreviewItem = {
  agent: BotActivityAgent;
  item: TranscriptItem;
  sortKey: string;
};

function useAgentActivityPreviewItems({
  agent,
  channelId,
  enabled,
}: {
  agent: BotActivityAgent;
  channelId?: string | null;
  enabled: boolean;
}): ComposerPreviewItem[] {
  const transcript = useAgentTranscript(enabled, agent.pubkey);

  return React.useMemo(() => {
    if (!enabled) {
      return [];
    }

    const scopedTranscript = channelId
      ? transcript.filter((item) => item.channelId === channelId)
      : transcript;

    return scopedTranscript.filter(isMeaningfulItem).map((item) => ({
      agent,
      item,
      sortKey: getPreviewItemSortKey(item),
    }));
  }, [agent, channelId, enabled, transcript]);
}

function getPreviewItemSortKey(item: TranscriptItem) {
  return `${item.timestamp}:${item.id}`;
}

function comparePreviewItems(
  left: ComposerPreviewItem,
  right: ComposerPreviewItem,
) {
  const leftTime = Date.parse(left.item.timestamp);
  const rightTime = Date.parse(right.item.timestamp);
  if (Number.isFinite(leftTime) && Number.isFinite(rightTime)) {
    const timeDiff = leftTime - rightTime;
    if (timeDiff !== 0) {
      return timeDiff;
    }
  }

  return left.sortKey.localeCompare(right.sortKey);
}

function previewItemsEqual(
  left: ComposerPreviewItem[],
  right: ComposerPreviewItem[],
) {
  if (left.length !== right.length) {
    return false;
  }

  return left.every(
    (item, index) =>
      item.item === right[index]?.item &&
      item.agent.pubkey === right[index]?.agent.pubkey &&
      item.sortKey === right[index]?.sortKey,
  );
}

export function BotActivityComposerAction({
  agents,
  channelId = null,
  onOpenAgentSession,
  openAgentSessionPubkey,
  profiles,
  typingBotPubkeys,
  variant = "toolbar",
}: BotActivityBarProps) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const typingAgents = React.useMemo(() => {
    const typingSet = new Set(
      typingBotPubkeys.map((pubkey) => pubkey.toLowerCase()),
    );

    return agents.filter((agent) => typingSet.has(agent.pubkey.toLowerCase()));
  }, [agents, typingBotPubkeys]);
  const activeAgentPubkeys = useActiveAgentPubkeysForChannel(channelId);
  const activeAgents = React.useMemo(() => {
    if (activeAgentPubkeys.length === 0) {
      return [];
    }

    const activeSet = new Set(activeAgentPubkeys);
    return agents.filter((agent) =>
      activeSet.has(normalizePubkey(agent.pubkey)),
    );
  }, [activeAgentPubkeys, agents]);
  const workingAgents = React.useMemo(() => {
    const byPubkey = new Map<string, BotActivityAgent>();
    for (const agent of typingAgents) {
      byPubkey.set(normalizePubkey(agent.pubkey), agent);
    }
    for (const agent of activeAgents) {
      byPubkey.set(normalizePubkey(agent.pubkey), agent);
    }
    return [...byPubkey.values()];
  }, [activeAgents, typingAgents]);
  const singleTypingAgent =
    workingAgents.length === 1 ? (workingAgents[0] ?? null) : null;
  const transcript = useAgentTranscript(
    Boolean(singleTypingAgent),
    singleTypingAgent?.pubkey,
  );
  const [previewItemsByAgent, setPreviewItemsByAgent] = React.useState<
    Record<string, ComposerPreviewItem[]>
  >({});
  const typingAgentKeys = React.useMemo(
    () => new Set(workingAgents.map((agent) => normalizePubkey(agent.pubkey))),
    [workingAgents],
  );
  const previewItems = React.useMemo(
    () =>
      workingAgents
        .flatMap(
          (agent) => previewItemsByAgent[normalizePubkey(agent.pubkey)] ?? [],
        )
        .sort(comparePreviewItems)
        .slice(-PREVIEW_ITEM_LIMIT),
    [previewItemsByAgent, workingAgents],
  );
  const handlePreviewItemsChange = React.useCallback(
    (agentPubkey: string, items: ComposerPreviewItem[]) => {
      const key = normalizePubkey(agentPubkey);
      setPreviewItemsByAgent((current) => {
        if (previewItemsEqual(current[key] ?? [], items)) {
          return current;
        }

        return { ...current, [key]: items };
      });
    },
    [],
  );

  React.useEffect(() => {
    setPreviewItemsByAgent((current) => {
      let changed = false;
      const next: Record<string, ComposerPreviewItem[]> = {};
      for (const [key, items] of Object.entries(current)) {
        if (typingAgentKeys.has(key)) {
          next[key] = items;
        } else {
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [typingAgentKeys]);

  const activityHeadlines = React.useMemo(() => {
    if (!singleTypingAgent) {
      return [];
    }

    const seen = new Set<string>();
    const headlines: string[] = [];
    const scopedTranscript = channelId
      ? transcript.filter((item) => item.channelId === channelId)
      : transcript;

    for (let i = scopedTranscript.length - 1; i >= 0; i--) {
      const item = scopedTranscript[i];
      if (!isMeaningfulItem(item)) {
        continue;
      }
      const headline = getActivityHeadline(item);
      if (!headline || seen.has(headline)) {
        continue;
      }

      seen.add(headline);
      headlines.unshift(headline);
      if (headlines.length >= 5) {
        break;
      }
    }

    return headlines;
  }, [channelId, singleTypingAgent, transcript]);
  const [headlineIndex, setHeadlineIndex] = React.useState(0);

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const openWithDelay = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer]);

  const closeWithDelay = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);

  const keepOpen = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  React.useEffect(() => {
    return () => clearHoverTimer();
  }, [clearHoverTimer]);

  React.useEffect(() => {
    if (activityHeadlines.length <= 1) {
      return;
    }

    const interval = window.setInterval(() => {
      setHeadlineIndex((current) => (current + 1) % activityHeadlines.length);
    }, HEADLINE_ROTATION_MS);

    return () => window.clearInterval(interval);
  }, [activityHeadlines.length]);

  if (workingAgents.length === 0) {
    return null;
  }

  const agentAvatarUrl = (agent: BotActivityAgent) =>
    profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const triggerLabel =
    workingAgents.length === 1
      ? `${workingAgents[0]?.name ?? "Agent"} is working`
      : `${workingAgents.length} agents working`;
  const isInline = variant === "inline";
  const visibleStatusLabel =
    workingAgents.length === 1
      ? `${workingAgents[0]?.name ?? "Agent"}: ${
          activityHeadlines[headlineIndex % activityHeadlines.length] ??
          "Working"
        }`
      : `${workingAgents[0]?.name ?? "Agent"} +${workingAgents.length - 1}`;

  return (
    <>
      {workingAgents.map((agent) => (
        <AgentActivityPreviewCollector
          agent={agent}
          channelId={channelId}
          key={agent.pubkey}
          onItemsChange={handlePreviewItemsChange}
        />
      ))}
      <Popover onOpenChange={setOpen} open={open}>
        <PopoverTrigger asChild>
          <button
            aria-label={`${triggerLabel}. View activity.`}
            className={cn(
              "inline-flex items-center justify-center rounded-full border border-border/60 bg-background font-medium text-muted-foreground transition-colors hover:border-primary/30 hover:bg-primary/5 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:border-primary/40 data-[state=open]:bg-primary/10 data-[state=open]:text-primary",
              isInline
                ? "h-7 min-w-0 gap-2 overflow-visible border-transparent bg-transparent px-0 text-xs font-semibold leading-none shadow-none hover:border-transparent hover:bg-transparent data-[state=open]:border-transparent data-[state=open]:bg-transparent"
                : "h-9 min-w-9 gap-1.5 px-2 text-xs",
            )}
            data-testid="bot-activity-composer-trigger"
            onBlur={closeWithDelay}
            onClick={() => {
              clearHoverTimer();
              setOpen((current) => !current);
            }}
            onFocus={() => setOpen(true)}
            onMouseEnter={openWithDelay}
            onMouseLeave={closeWithDelay}
            type="button"
          >
            <span className="flex items-center overflow-visible py-px -space-x-1">
              {workingAgents.slice(0, 2).map((agent) => (
                <UserAvatar
                  avatarUrl={agentAvatarUrl(agent)}
                  className={cn(
                    "border border-background",
                    isInline
                      ? "!h-[18px] !w-[18px] shadow-xs ring-1 ring-primary/25 text-3xs"
                      : "shrink-0",
                  )}
                  displayName={agent.name}
                  key={agent.pubkey}
                  size="xs"
                />
              ))}
            </span>
            {workingAgents.length > 2 ? (
              <span className="text-2xs leading-none">
                +{workingAgents.length - 2}
              </span>
            ) : null}
            <span
              className={cn(
                isInline ? "min-w-0 flex-1 overflow-hidden" : "sr-only",
              )}
            >
              {isInline ? (
                <Shimmer className="block truncate">
                  {visibleStatusLabel}
                </Shimmer>
              ) : (
                "working"
              )}
            </span>
            {isInline ? null : (
              <Loader2 className="h-4 w-4 shrink-0 animate-spin opacity-70" />
            )}
          </button>
        </PopoverTrigger>
        <PopoverContent
          align={isInline ? "start" : "end"}
          className="w-[24rem] max-w-[calc(100vw-2rem)] p-0"
          onMouseEnter={keepOpen}
          onMouseLeave={closeWithDelay}
          onOpenAutoFocus={(event) => event.preventDefault()}
          side="top"
          sideOffset={8}
        >
          <div className="flex items-center justify-between gap-3 border-border/60 border-b px-3 py-2">
            <div className="min-w-0 text-xs font-medium text-muted-foreground">
              Agents working
            </div>
            <button
              className="shrink-0 rounded-full px-2 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              onClick={() => {
                const firstAgent = workingAgents[0];
                if (!firstAgent) {
                  return;
                }
                clearHoverTimer();
                setOpen(false);
                onOpenAgentSession(firstAgent.pubkey);
              }}
              type="button"
            >
              Open feed
            </button>
          </div>
          <ComposerActivityPreview
            agentAvatarUrl={agentAvatarUrl}
            onOpenAgentSession={(pubkey) => {
              clearHoverTimer();
              setOpen(false);
              onOpenAgentSession(pubkey);
            }}
            openAgentSessionPubkey={openAgentSessionPubkey}
            previewItems={previewItems}
            profiles={profiles}
            typingAgents={workingAgents}
          />
        </PopoverContent>
      </Popover>
    </>
  );
}

function AgentActivityPreviewCollector({
  agent,
  channelId,
  onItemsChange,
}: {
  agent: BotActivityAgent;
  channelId?: string | null;
  onItemsChange: (agentPubkey: string, items: ComposerPreviewItem[]) => void;
}) {
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  const hasActiveChannelTurn = channelId
    ? activeTurns.some((turn) => turn.channelId === channelId)
    : activeTurns.length > 0;
  const items = useAgentActivityPreviewItems({
    agent,
    channelId,
    enabled: hasActiveChannelTurn,
  });

  React.useEffect(() => {
    onItemsChange(agent.pubkey, items);
  }, [agent.pubkey, items, onItemsChange]);

  return null;
}

function ComposerActivityPreview({
  agentAvatarUrl,
  onOpenAgentSession,
  openAgentSessionPubkey,
  previewItems,
  profiles,
  typingAgents,
}: {
  agentAvatarUrl: (agent: BotActivityAgent) => string | null;
  onOpenAgentSession: (pubkey: string) => void;
  openAgentSessionPubkey: string | null;
  previewItems: ComposerPreviewItem[];
  profiles?: UserProfileLookup;
  typingAgents: BotActivityAgent[];
}) {
  const tailRef = React.useRef<HTMLDivElement>(null);
  const latestTailKey =
    previewItems.length > 0
      ? (previewItems[previewItems.length - 1]?.sortKey ?? null)
      : null;
  const selectedPubkey = openAgentSessionPubkey
    ? normalizePubkey(openAgentSessionPubkey)
    : null;

  React.useEffect(() => {
    if (!latestTailKey) {
      return;
    }

    tailRef.current?.scrollIntoView({ block: "end" });
  }, [latestTailKey]);

  if (previewItems.length === 0) {
    return (
      <div className="flex h-48 flex-col gap-1 overflow-y-auto px-2 py-2">
        {typingAgents.map((agent) => {
          const isSelected = selectedPubkey === normalizePubkey(agent.pubkey);

          return (
            <button
              className={cn(
                "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors",
                isSelected
                  ? "bg-primary/10 text-primary"
                  : "text-foreground hover:bg-accent hover:text-accent-foreground",
              )}
              data-testid={`bot-activity-composer-item-${agent.pubkey}`}
              key={agent.pubkey}
              onClick={() => onOpenAgentSession(agent.pubkey)}
              type="button"
            >
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className="shrink-0"
                displayName={agent.name}
                size="sm"
              />
              <span className="min-w-0 flex-1 truncate">{agent.name}</span>
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground/70" />
            </button>
          );
        })}
      </div>
    );
  }

  return (
    <div
      aria-label="Live agent activity preview"
      className="h-48 overflow-y-auto px-2 py-2"
      data-testid="bot-activity-composer-preview"
      role="log"
    >
      <div className="flex flex-col gap-2">
        {previewItems.map(({ agent, item, sortKey }) => (
          <div
            className="group flex w-full items-start gap-2 rounded-xl px-2 py-1.5 transition-colors hover:bg-accent/70"
            key={`${normalizePubkey(agent.pubkey)}:${sortKey}`}
          >
            <button
              aria-label={`Open ${agent.name} activity`}
              className="mt-0.5 shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              data-testid={`bot-activity-composer-item-${agent.pubkey}`}
              onClick={() => onOpenAgentSession(agent.pubkey)}
              type="button"
            >
              <UserAvatar
                avatarUrl={agentAvatarUrl(agent)}
                className="ring-2 ring-background"
                displayName={agent.name}
                size="xs"
              />
            </button>
            <div className="min-w-0 flex-1">
              <div className="mb-0.5 flex items-center gap-1.5">
                <button
                  className="min-w-0 truncate rounded-sm text-left text-2xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                  onClick={() => onOpenAgentSession(agent.pubkey)}
                  type="button"
                >
                  {agent.name}
                </button>
                <span className="h-1 w-1 shrink-0 rounded-full bg-primary/60" />
                <span className="truncate text-2xs text-muted-foreground/80">
                  {getActivityHeadline(item) ?? item.title}
                </span>
              </div>
              <div className="overflow-hidden rounded-lg [&_*]:max-w-full">
                <TranscriptActivityItem
                  agentAvatarUrl={
                    profiles?.[normalizePubkey(agent.pubkey)]?.avatarUrl ?? null
                  }
                  agentName={agent.name}
                  agentPubkey={agent.pubkey}
                  item={item}
                  profiles={profiles}
                />
              </div>
            </div>
          </div>
        ))}
        <div aria-hidden="true" ref={tailRef} />
      </div>
    </div>
  );
}
