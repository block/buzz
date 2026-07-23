import * as React from "react";

import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { scopeByChannel } from "@/features/agents/ui/agentSessionPanelLayout";
import { isMeaningfulItem } from "@/features/agents/ui/agentSessionTranscriptPresentation";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import { formatLastLiveLabel } from "@/features/profile/lib/lastLiveLabel";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import type { BotActivityAgent } from "./BotActivityBar";

/**
 * Single-agent live activity preview for the composer "agents working"
 * popover. Preview-gated behind the `composerLiveActivity` feature.
 *
 * Renders the selected working agent's channel-scoped transcript with the
 * same compact primitive as the profile activity embed
 * (`ManagedAgentSessionPanel` + `compactPreview`), so projection, live/archive
 * merging, scrolling, and idle handling stay owned by that surface.
 *
 * The whole preview is ONE click target: an overlay button opens the agent's
 * full activity view, and transcript rows underneath are made inert. This
 * avoids nesting interactive transcript controls inside a clickable shell.
 */
export function ComposerLiveActivityFeed({
  agent,
  channelId,
  className,
  onOpenAgentSession,
  profiles,
}: {
  agent: BotActivityAgent;
  channelId: string | null;
  className?: string;
  onOpenAgentSession: (pubkey: string) => void;
  profiles?: UserProfileLookup;
}) {
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  const transcript = useAgentTranscript(true, agent.pubkey);
  const lastLiveAt = React.useMemo(() => {
    const scoped = scopeByChannel(transcript, channelId).filter(
      isMeaningfulItem,
    );
    for (let index = scoped.length - 1; index >= 0; index -= 1) {
      const item = scoped[index];
      if (!item) {
        continue;
      }
      const millis = Date.parse(item.timestamp);
      if (!Number.isNaN(millis)) {
        return millis;
      }
    }

    const channelTurn = channelId
      ? activeTurns.find((turn) => turn.channelId === channelId)
      : activeTurns[0];
    return channelTurn?.anchorAt ?? null;
  }, [activeTurns, channelId, transcript]);

  const now = useNow(15_000);
  const lastLiveLabel = formatLastLiveLabel(lastLiveAt, now);
  const openLabel = `Open ${agent.name}'s full activity. Last live ${lastLiveLabel}.`;
  const avatarUrl = profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;

  return (
    <div
      className={cn("relative overflow-hidden", className)}
      data-testid="composer-live-activity-feed"
    >
      <button
        aria-label={openLabel}
        className="absolute inset-0 z-10 cursor-pointer rounded-lg transition-colors hover:bg-accent/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        data-testid="composer-live-activity-open"
        onClick={() => onOpenAgentSession(agent.pubkey)}
        type="button"
      />
      <Button
        aria-label={openLabel}
        className="absolute right-2 top-2 z-20 rounded-full bg-primary px-2.5 text-xs font-semibold text-primary-foreground hover:bg-primary/90"
        data-testid="composer-live-activity-last-live"
        onClick={(event) => {
          event.stopPropagation();
          onOpenAgentSession(agent.pubkey);
        }}
        size="xs"
        title={`Last live ${lastLiveLabel}`}
        type="button"
      >
        {lastLiveLabel}
      </Button>
      <ManagedAgentSessionPanel
        agent={{ ...agent, avatarUrl }}
        autoTail={true}
        channelId={channelId}
        className="relative z-0 h-full min-h-0 border-0 bg-transparent px-3 text-xs shadow-none **:data-message-id:pointer-events-none"
        emptyDescription="Live activity will appear here."
        emptyState="loading"
        panelPadding={false}
        profiles={profiles}
        rawLayout="responsive"
        showHeader={false}
        showRaw={false}
        transcriptContentClassName="py-2"
        transcriptVariant="compactPreview"
      />
    </div>
  );
}
