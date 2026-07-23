import * as React from "react";

import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import {
  useAgentTranscript,
  useArchivedChannelEvents,
} from "@/features/agents/ui/useObserverEvents";
import { formatLastLiveLabel } from "@/features/profile/lib/lastLiveLabel";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import type { BotActivityAgent } from "./BotActivityBar";
import { deriveLastLiveAt } from "./composerLiveActivity";

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
 * full activity view. The panel subtree is wrapped in a native `inert`
 * container so its rows (which can include keyboard-focusable message links)
 * are removed from pointer, keyboard-tab, AND assistive-tech interaction —
 * CSS pointer-events alone would leave them tabbable behind the overlay.
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
  const archivedEvents = useArchivedChannelEvents(agent.pubkey, channelId);
  const lastLiveAt = React.useMemo(
    () =>
      deriveLastLiveAt({ activeTurns, archivedEvents, channelId, transcript }),
    [activeTurns, archivedEvents, channelId, transcript],
  );

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
      <div
        className="h-full min-h-0"
        data-testid="composer-live-activity-inert"
        inert={true}
      >
        <ManagedAgentSessionPanel
          agent={{ ...agent, avatarUrl }}
          autoTail={true}
          channelId={channelId}
          className="relative z-0 h-full min-h-0 border-0 bg-transparent px-3 text-xs shadow-none"
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
    </div>
  );
}
