// Per-thread activity pill: renders on a thread's summary row when agents
// did file work in that thread, and opens the activity map in a popover
// locked to the thread. The pill IS the thread selection — no picker in
// the popover; other threads are reached by scrolling to their pills. A
// long-lived channel's map entry points stay distributed across its
// threads instead of funneling through one dense channel-wide view.
//
// Reads ChannelThreadActivityProvider context; renders nothing on surfaces
// without the provider (or when the thread has no resolved activity), so
// mounting it inside MessageThreadSummaryRow is free for non-agent threads.

import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import type { ThreadGroup } from "./activityThreads";
import { ThreadActivityPanel } from "./ThreadActivityPanel";
import {
  type ThreadActivityAgent,
  useChannelThreadActivity,
} from "./threadActivityContext";

export function ThreadActivityPill({ threadRootId }: { threadRootId: string }) {
  const { groupsByRootId, agents, channelId, workingPubkeys } =
    useChannelThreadActivity();
  const group = groupsByRootId.get(threadRootId);
  if (!group || !channelId) return null;
  return (
    <ThreadActivityPillBody
      agents={agents}
      channelId={channelId}
      group={group}
      workingPubkeys={workingPubkeys}
    />
  );
}

function agentDisplayLabel(
  group: ThreadGroup,
  agents: readonly ThreadActivityAgent[],
): string {
  const memberPubkeys = [...group.turnIdsByAgent.keys()];
  if (memberPubkeys.length === 1) {
    const pubkey = memberPubkeys[0].toLowerCase();
    const agent = agents.find(
      (candidate) => candidate.pubkey.toLowerCase() === pubkey,
    );
    if (agent) return agent.name;
  }
  return `${memberPubkeys.length} agents`;
}

function ThreadActivityPillBody({
  agents,
  channelId,
  group,
  workingPubkeys,
}: {
  agents: readonly ThreadActivityAgent[];
  channelId: string;
  group: ThreadGroup;
  workingPubkeys: readonly string[];
}) {
  const [open, setOpen] = React.useState(false);

  // Live only when an agent has an OPEN turn in THIS thread and the channel
  // working signal agrees — an open turn alone can be a crashed agent, and
  // a working agent may be working in a different thread.
  const working = React.useMemo(() => {
    if (workingPubkeys.length === 0 || group.agentsWithOpenTurn.size === 0) {
      return false;
    }
    const workingSet = new Set(
      workingPubkeys.map((pubkey) => pubkey.toLowerCase()),
    );
    return [...group.agentsWithOpenTurn].some((pubkey) =>
      workingSet.has(pubkey.toLowerCase()),
    );
  }, [workingPubkeys, group]);

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-expanded={open}
          className={cn(
            "flex h-6 shrink-0 items-center gap-1.5 rounded-full border border-border/50 bg-background/85 px-2.5 text-2xs text-muted-foreground shadow-sm backdrop-blur-sm transition-colors hover:bg-muted/70 hover:text-foreground",
            open && "text-foreground",
          )}
          data-testid="thread-activity-pill"
          type="button"
        >
          <span className="font-medium uppercase tracking-[0.14em]">
            Activity
          </span>
          <span className="max-w-32 truncate">
            {agentDisplayLabel(group, agents)}
          </span>
          {working ? (
            <span
              aria-hidden
              className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500"
              title="Agents working"
            />
          ) : null}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-[min(90vw,44rem)] p-1.5"
        collisionPadding={12}
        data-testid="thread-activity-popover"
        side="bottom"
      >
        <ThreadActivityPanel
          agents={agents}
          channelId={channelId}
          className="border-0 bg-transparent shadow-none"
          threadGroup={group}
        />
      </PopoverContent>
    </Popover>
  );
}
