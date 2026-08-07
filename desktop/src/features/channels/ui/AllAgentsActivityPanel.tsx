import * as React from "react";
import { Loader2 } from "lucide-react";

import { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import { agentsForAllActivityPanel } from "@/features/channels/ui/botActivityViewAll";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AllAgentsActivityPanelProps = {
  agents: BotActivityAgent[];
  channelId?: string | null;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
  transparentChrome?: boolean;
  widthPx: number;
  workingBotPubkeys: string[];
};

type WorkingAgentActivityCardProps = {
  agent: BotActivityAgent;
  avatarUrl: string | null;
  channelId?: string | null;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
};

/** Mount heavy transcript panels only when the card approaches the viewport. */
function useNearViewport(rootMargin = "160px") {
  const ref = React.useRef<HTMLDivElement | null>(null);
  const [near, setNear] = React.useState(false);

  React.useEffect(() => {
    if (near) {
      return;
    }
    const node = ref.current;
    if (!node || typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setNear(true);
          observer.disconnect();
        }
      },
      { rootMargin },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [near, rootMargin]);

  return { ref, near };
}

const WorkingAgentActivityCard = React.memo(function WorkingAgentActivityCard({
  agent,
  avatarUrl,
  channelId = null,
  onOpenAgentSession,
  profiles,
}: WorkingAgentActivityCardProps) {
  const { ref, near } = useNearViewport();

  return (
    <div
      className="overflow-hidden rounded-lg border border-border/70 bg-background/80 shadow-xs"
      data-testid={`all-agents-activity-card-${agent.pubkey}`}
      ref={ref}
      style={{ contentVisibility: "auto", containIntrinsicSize: "0 220px" }}
    >
      <div className="flex items-center gap-3 border-b border-border/50 px-3 py-2.5">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="shrink-0 ring-1 ring-primary/20"
          displayName={agent.name}
          size="sm"
        />
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <h3 className="truncate text-sm font-semibold text-foreground">
            {agent.name}
          </h3>
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-primary/70" />
        </div>
        <button
          className="shrink-0 rounded-md px-2 py-1 text-xs font-medium text-primary transition-colors hover:bg-primary/10"
          data-testid={`all-agents-activity-open-${agent.pubkey}`}
          onClick={() => onOpenAgentSession(agent.pubkey, channelId)}
          type="button"
        >
          View
        </button>
      </div>

      <div className="relative flex h-44 flex-col overflow-hidden">
        {near ? (
          <ManagedAgentSessionPanel
            agent={{
              pubkey: agent.pubkey,
              name: agent.name,
              status: agent.status ?? "running",
              avatarUrl,
            }}
            autoTail={true}
            channelId={channelId}
            className="min-h-0 flex-1 border-0 bg-transparent px-3 text-xs shadow-none **:data-message-id:pointer-events-none"
            emptyDescription="Waiting for activity…"
            emptyState="loading"
            includeArchivedEvents={false}
            panelPadding={false}
            profiles={profiles}
            rawLayout="responsive"
            showHeader={false}
            showRaw={false}
            transcriptContentClassName="py-2"
            transcriptVariant="compactPreview"
          />
        ) : (
          <div className="flex min-h-0 flex-1 items-center justify-center text-xs text-muted-foreground">
            Loading activity…
          </div>
        )}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 top-0 h-6 bg-linear-to-b from-background/80 to-transparent"
        />
      </div>
    </div>
  );
});

export function AllAgentsActivityPanel({
  agents,
  channelId = null,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onOpenAgentSession,
  profiles,
  transparentChrome = false,
  widthPx,
  workingBotPubkeys,
}: AllAgentsActivityPanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const panelAgents = React.useMemo(
    () => agentsForAllActivityPanel({ agents, workingBotPubkeys }),
    [agents, workingBotPubkeys],
  );
  const agentAvatarUrl = React.useCallback(
    (agent: BotActivityAgent) =>
      profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null,
    [profiles],
  );

  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="all-agents-activity-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={layout !== "split" && !isOverlay}
          backdropSurface="soft"
          inset={layout !== "split" ? "wide" : "default"}
        >
          <AuxiliaryPanelHeaderGroup align="start">
            <AuxiliaryPanelHeaderTitleBlock
              subtitle={`${panelAgents.length} agent${
                panelAgents.length === 1 ? "" : "s"
              } working now`}
              title="All agent activity"
            />
          </AuxiliaryPanelHeaderGroup>
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        className={cn(
          // Use px/pb only — a full `p-*` would override the single-panel
          // `pt-13` inset that clears the overlapping header chrome.
          "flex flex-col gap-2 overflow-y-auto px-4 pb-4",
          layout === "split" && "bg-transparent",
        )}
        panelPadding
      >
        {panelAgents.map((agent) => (
          <WorkingAgentActivityCard
            agent={agent}
            avatarUrl={agentAvatarUrl(agent)}
            channelId={channelId}
            key={agent.pubkey}
            onOpenAgentSession={onOpenAgentSession}
            profiles={profiles}
          />
        ))}
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
