import * as React from "react";

import { AgentSessionTranscriptVariantProvider } from "@/features/agents/ui/agentSessionTranscriptContext";
import { TranscriptActivityItem } from "@/features/agents/ui/activityRenderClasses/TranscriptActivityItem";
import { useAgentTranscripts } from "@/features/agents/ui/useObserverEvents";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  groupLiveActivity,
  mergeLiveActivity,
  type LiveActivityAgent,
} from "./composerLiveActivity";

/**
 * Merged multi-agent live activity preview for the composer popover.
 *
 * Interleaves the working agents' transcripts (scoped to the current channel)
 * into one chronological stream, badges each same-agent run with the agent's
 * identity, and auto-tails streaming updates. Rendering reuses the compact
 * transcript presenters from the full activity view.
 */
export function ComposerLiveActivityFeed({
  agents,
  channelId,
  className,
  onOpenAgentSession,
  profiles,
}: {
  agents: LiveActivityAgent[];
  channelId: string | null;
  className?: string;
  onOpenAgentSession: (pubkey: string) => void;
  profiles?: UserProfileLookup;
}) {
  const agentPubkeys = React.useMemo(
    () => agents.map((agent) => agent.pubkey),
    [agents],
  );
  const transcripts = useAgentTranscripts(agents.length > 0, agentPubkeys);
  const entries = React.useMemo(
    () => mergeLiveActivity(agents, transcripts, channelId),
    [agents, channelId, transcripts],
  );
  const groups = React.useMemo(() => groupLiveActivity(entries), [entries]);

  const scrollContainerRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  // Stable identity per entries snapshot — useAnchoredScroll keys effects on
  // `messages`, so a fresh array every render would re-run them needlessly.
  const anchoredMessages = React.useMemo(
    () => entries.map((entry) => ({ id: entry.key })),
    [entries],
  );
  const anchoredScroll = useAnchoredScroll({
    channelId: `composer-live-activity:${channelId ?? "all"}`,
    contentRef,
    isLoading: false,
    messages: anchoredMessages,
    scrollContainerRef,
  });

  const agentAvatarUrl = (agent: LiveActivityAgent) =>
    profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;

  if (entries.length === 0) {
    return (
      <div
        className={cn(
          "flex flex-col items-center justify-center px-6 text-center",
          className,
        )}
        data-testid="composer-live-activity-empty"
      >
        <FuzzyLogo
          ariaLabel="Waiting for agent activity"
          className="mx-auto text-muted-foreground"
          fuzz={false}
          loop
        />
      </div>
    );
  }

  return (
    <div
      className={cn("overflow-y-auto overscroll-contain", className)}
      data-testid="composer-live-activity-feed"
      onScroll={anchoredScroll.onScroll}
      ref={scrollContainerRef}
    >
      <div
        aria-label="Live agent activity"
        aria-live="polite"
        className="flex w-full flex-col gap-2 px-3 py-2"
        ref={contentRef}
        role="log"
      >
        <AgentSessionTranscriptVariantProvider value="compactPreview">
          {groups.map((group) => (
            <div
              className="flex flex-col gap-1"
              data-message-id={group.key}
              data-testid={`composer-live-activity-group-${group.agent.pubkey}`}
              key={group.key}
            >
              <button
                aria-label={`Open ${group.agent.name} activity`}
                className="flex w-fit max-w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left transition-colors hover:bg-accent"
                onClick={() => onOpenAgentSession(group.agent.pubkey)}
                type="button"
              >
                <UserAvatar
                  avatarUrl={agentAvatarUrl(group.agent)}
                  className="!h-[18px] !w-[18px] shrink-0 text-3xs"
                  displayName={group.agent.name}
                  size="xs"
                />
                <span className="truncate text-xs font-semibold text-muted-foreground">
                  {group.agent.name}
                </span>
              </button>
              <div className="flex min-w-0 flex-col gap-1 pl-2">
                {group.entries.map((entry) => (
                  <div data-message-id={entry.key} key={entry.key}>
                    <TranscriptActivityItem
                      agentAvatarUrl={agentAvatarUrl(group.agent)}
                      agentName={group.agent.name}
                      agentPubkey={group.agent.pubkey}
                      item={entry.item}
                      profiles={profiles}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </AgentSessionTranscriptVariantProvider>
      </div>
    </div>
  );
}
