import * as React from "react";
import { Loader2 } from "lucide-react";

import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import { agentsForAllActivityPanel } from "@/features/channels/ui/botActivityViewAll";
import { useWorkingAgentHeadlines } from "@/features/channels/ui/useWorkingAgentHeadlines";
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
import { Shimmer } from "@/shared/ui/Shimmer";
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
};

function WorkingAgentActivityCard({
  agent,
  avatarUrl,
  channelId = null,
  onOpenAgentSession,
}: WorkingAgentActivityCardProps) {
  const headlines = useWorkingAgentHeadlines(true, agent.pubkey, channelId, 8);
  const latestHeadline = headlines[headlines.length - 1] ?? "Working";

  return (
    <article
      className="rounded-lg border border-border/70 bg-background/80 p-3 shadow-xs"
      data-testid={`all-agents-activity-card-${agent.pubkey}`}
    >
      <div className="flex items-start gap-3">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="shrink-0 ring-1 ring-primary/20"
          displayName={agent.name}
          size="sm"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-semibold text-foreground">
              {agent.name}
            </h3>
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-primary/70" />
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            <Shimmer className="inline">{latestHeadline}</Shimmer>
          </p>
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

      {headlines.length > 0 ? (
        <ul className="mt-3 space-y-1.5 border-t border-border/50 pt-3">
          {headlines.map((headline) => (
            <li
              className="rounded-md bg-muted/30 px-2.5 py-1.5 text-xs leading-relaxed text-foreground"
              key={headline}
            >
              {headline}
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-3 border-t border-border/50 pt-3 text-xs text-muted-foreground">
          Waiting for activity…
        </p>
      )}
    </article>
  );
}

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

  const workingSet = React.useMemo(
    () => new Set(workingBotPubkeys.map((pubkey) => pubkey.toLowerCase())),
    [workingBotPubkeys],
  );
  const panelAgents = React.useMemo(
    () => agentsForAllActivityPanel({ agents, workingBotPubkeys }),
    [agents, workingBotPubkeys],
  );
  const workingAgents = React.useMemo(
    () =>
      panelAgents.filter((agent) =>
        workingSet.has(agent.pubkey.toLowerCase()),
      ),
    [panelAgents, workingSet],
  );
  const agentAvatarUrl = (agent: BotActivityAgent) =>
    profiles?.[agent.pubkey.toLowerCase()]?.avatarUrl ?? null;

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
        >
          <AuxiliaryPanelHeaderGroup align="start">
            <AuxiliaryPanelHeaderTitleBlock
              subtitle={`${panelAgents.length} agent${
                panelAgents.length === 1 ? "" : "s"
              } in this channel${
                workingAgents.length > 0
                  ? ` · ${workingAgents.length} working now`
                  : ""
              }`}
              title="All agent activity"
            />
          </AuxiliaryPanelHeaderGroup>
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        className={cn(
          "flex flex-col gap-3 p-4",
          layout === "split" && "bg-transparent",
        )}
      >
        {panelAgents.map((agent) => (
          <WorkingAgentActivityCard
            agent={agent}
            avatarUrl={agentAvatarUrl(agent)}
            channelId={channelId}
            key={agent.pubkey}
            onOpenAgentSession={onOpenAgentSession}
          />
        ))}
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
