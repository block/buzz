import {
  Activity,
  ChevronRight,
  ListTree,
  type LucideIcon,
  Workflow,
} from "lucide-react";
import * as React from "react";

import { AgentSessionTranscriptList } from "@/features/agents/ui/AgentSessionTranscriptList";
import { ModelWorkStream } from "@/features/agents/ui/ModelWorkStreamView";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import {
  buildInlineAgentActivityPlacement,
  type InlineAgentActivityPlacement,
} from "@/features/channels/lib/inlineAgentActivity";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type InlineAgentActivity = InlineAgentActivityPlacement & {
  content: React.ReactNode;
};

type AgentActivityView = "adaptive" | "trace";

const ACTIVITY_VIEWS: {
  icon: LucideIcon;
  id: AgentActivityView;
  label: string;
  title: string;
}[] = [
  {
    icon: Workflow,
    id: "adaptive",
    label: "Adaptive",
    title: "Model-selected operating modes",
  },
  { icon: ListTree, id: "trace", label: "Trace", title: "Full trace" },
];

export function useInlineAgentActivity({
  agents,
  channelId,
  onOpenAgentSession,
  profiles,
  renderedMessageIds,
  workingBotPubkeys,
}: {
  agents: BotActivityAgent[];
  channelId: string | null;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  profiles?: UserProfileLookup;
  renderedMessageIds: ReadonlySet<string>;
  workingBotPubkeys: string[];
}): InlineAgentActivity | null {
  const [view, setView] = React.useState<AgentActivityView>("adaptive");
  const workingSet = React.useMemo(
    () => new Set(workingBotPubkeys.map((pubkey) => pubkey.toLowerCase())),
    [workingBotPubkeys],
  );
  const workingAgent = React.useMemo(
    () => agents.find((agent) => workingSet.has(agent.pubkey.toLowerCase())),
    [agents, workingSet],
  );
  const workingAgentPubkey = workingAgent?.pubkey ?? null;
  const [recentAgent, setRecentAgent] = React.useState<{
    channelId: string;
    pubkey: string;
  } | null>(null);
  React.useEffect(() => {
    if (channelId && workingAgentPubkey) {
      setRecentAgent((current) =>
        current?.channelId === channelId &&
        current.pubkey.toLowerCase() === workingAgentPubkey.toLowerCase()
          ? current
          : { channelId, pubkey: workingAgentPubkey },
      );
    }
  }, [channelId, workingAgentPubkey]);
  const selectedAgent = React.useMemo(
    () =>
      workingAgent ??
      agents.find(
        (agent) =>
          recentAgent?.channelId === channelId &&
          agent.pubkey.toLowerCase() === recentAgent.pubkey.toLowerCase(),
      ) ??
      (agents.length === 1 ? agents[0] : null),
    [agents, channelId, recentAgent, workingAgent],
  );
  const transcript = useAgentTranscript(
    Boolean(selectedAgent),
    selectedAgent?.pubkey,
  );
  const isWorking = selectedAgent
    ? workingSet.has(selectedAgent.pubkey.toLowerCase())
    : false;
  const placement = React.useMemo(
    () =>
      channelId
        ? buildInlineAgentActivityPlacement({
            channelId,
            isWorking,
            renderedMessageIds,
            transcript,
          })
        : null,
    [channelId, isWorking, renderedMessageIds, transcript],
  );

  if (!channelId || !selectedAgent || !placement) {
    return null;
  }

  const avatarUrl =
    profiles?.[selectedAgent.pubkey.toLowerCase()]?.avatarUrl ?? null;
  const traceItems = placement.items.filter(
    (item) => item.type !== "message" && item.type !== "metadata",
  );
  const content = (
    <section
      aria-label={`${selectedAgent.name} activity`}
      className="px-3 py-2"
      data-testid="inline-agent-activity"
    >
      <div className="flex min-w-0 items-start gap-3">
        <UserAvatar
          avatarUrl={avatarUrl}
          className="mt-0.5 shrink-0"
          displayName={selectedAgent.name}
          size="sm"
        />
        <div className="min-w-0 flex-1 border-l-2 border-primary/25 pl-3">
          <div className="mb-2 flex min-w-0 flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
            <button
              className="group inline-flex min-w-0 items-center gap-1 text-xs font-semibold text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              onClick={() =>
                onOpenAgentSession(selectedAgent.pubkey, channelId)
              }
              type="button"
            >
              <Activity aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{selectedAgent.name} activity</span>
              <ChevronRight
                aria-hidden="true"
                className="h-3.5 w-3.5 shrink-0 transition-transform group-hover:translate-x-0.5"
              />
            </button>
            <div
              aria-label="Agent activity view"
              className="grid h-7 w-full shrink-0 grid-cols-2 items-center rounded-md border border-border/80 bg-muted/30 p-0.5 sm:w-auto"
              role="tablist"
            >
              {ACTIVITY_VIEWS.map((activityView) => {
                const Icon = activityView.icon;
                const selected = view === activityView.id;
                return (
                  <button
                    aria-selected={selected}
                    className={viewButtonClassName(selected)}
                    key={activityView.id}
                    onClick={() => setView(activityView.id)}
                    role="tab"
                    title={activityView.title}
                    type="button"
                  >
                    <Icon aria-hidden="true" className="h-3 w-3" />
                    {activityView.label}
                  </button>
                );
              })}
            </div>
          </div>
          {view === "adaptive" ? (
            <ModelWorkStream
              agentAvatarUrl={avatarUrl}
              agentName={selectedAgent.name}
              agentPubkey={selectedAgent.pubkey}
              isWorking={isWorking}
              items={placement.items}
              profiles={profiles}
            />
          ) : (
            <AgentSessionTranscriptList
              agentAvatarUrl={avatarUrl}
              agentName={selectedAgent.name}
              agentPubkey={selectedAgent.pubkey}
              channelId={channelId}
              contentContainerClassName="gap-2"
              emptyDescription="Waiting for activity."
              items={traceItems}
              profiles={profiles}
              variant="inlineTimeline"
            />
          )}
        </div>
      </div>
    </section>
  );

  return { ...placement, content };
}

function viewButtonClassName(selected: boolean) {
  return selected
    ? "inline-flex h-6 min-w-0 items-center justify-center gap-1 rounded-[4px] bg-background px-1.5 text-3xs font-semibold text-foreground shadow-xs"
    : "inline-flex h-6 min-w-0 items-center justify-center gap-1 rounded-[4px] px-1.5 text-3xs font-medium text-muted-foreground transition-colors hover:text-foreground";
}
