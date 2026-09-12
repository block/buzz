// Thread-scoped activity map: every agent's file activity in ONE thread,
// rendered as a combined scene with per-agent colored threads and riders;
// riders and agent chips click through to that agent's session panel.
//
// Locked to the thread whose pill opened it — the pill IS the thread
// selection, so there is no picker here. Other threads' maps are reached by
// scrolling to their pills. (An unscoped channel-wide view died with the
// channel strip; resurrect deliberately if ever missed, not by default.)

import * as React from "react";

import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useChannelWorkingAgentPubkeys } from "../agentWorkingSignal";
import { ActivityPanelBody } from "./AgentActivityPanel";
import { colorForAgent } from "./activityModel";
import type { ThreadGroup } from "./activityThreads";
import type { deriveActivityTurns } from "./activityTurns";
import type { ActivityScope } from "./useActivityScene";
import {
  type MultiAgentActivityInput,
  useMultiAgentActivityScene,
} from "./useMultiAgentActivity";

export interface ThreadActivityPanelProps {
  channelId: string;
  /** Managed agents visible on this surface (pubkey + display name). */
  agents: readonly MultiAgentActivityInput[];
  /** The resolved thread group this map is locked to. */
  threadGroup: ThreadGroup;
  className?: string;
}

const EMPTY_TURNS: ReturnType<typeof deriveActivityTurns> = [];
const EMPTY_AGENTS: readonly MultiAgentActivityInput[] = [];

export function ThreadActivityPanel({
  channelId,
  agents,
  threadGroup,
  className,
}: ThreadActivityPanelProps) {
  const { onOpenAgentSession } = useAgentSession();
  const [agentFilter, setAgentFilter] = React.useState<string | null>(null);

  const threadAgents = React.useMemo(
    () => agentsWithThreadActivity(agents, threadGroup.turnIdsByAgent),
    [agents, threadGroup],
  );
  const threadTurnFilter = React.useMemo(
    () => turnFilterForThreadAgents(threadAgents, threadGroup.turnIdsByAgent),
    [threadAgents, threadGroup],
  );

  const filteredAgents = React.useMemo(
    () =>
      agentFilter
        ? threadAgents.filter(
            (agent) => normalizePubkey(agent.pubkey) === agentFilter,
          )
        : threadAgents,
    [agentFilter, threadAgents],
  );

  const sceneState = useMultiAgentActivityScene(
    filteredAgents,
    channelId,
    threadTurnFilter,
  );
  const allThreadSceneState = useMultiAgentActivityScene(
    agentFilter ? threadAgents : EMPTY_AGENTS,
    channelId,
    threadTurnFilter,
  );
  const scene = sceneState.scene ?? allThreadSceneState.scene;
  const sceneAgents = sceneState.scene
    ? sceneState.agents
    : allThreadSceneState.agents;

  // Reset a filter pointing at an agent that no longer belongs to this thread.
  // If the agent still belongs but currently yields no file events, the panel
  // falls back to the all-thread scene so the All-agents recovery chip remains
  // reachable.
  React.useEffect(() => {
    if (!agentFilter) return;
    if (
      !threadAgents.some(
        (agent) => normalizePubkey(agent.pubkey) === agentFilter,
      )
    ) {
      setAgentFilter(null);
    }
  }, [agentFilter, threadAgents]);

  const workingPubkeys = useChannelWorkingAgentPubkeys(channelId);
  const threadWorking = threadScopeIsLive(
    threadGroup.agentsWithOpenTurn,
    workingPubkeys,
  );
  const workingSet = React.useMemo(
    () => new Set(workingPubkeys.map((pubkey) => normalizePubkey(pubkey))),
    [workingPubkeys],
  );

  const openAgent = React.useCallback(
    (agentId: string) => {
      onOpenAgentSession?.(agentId, channelId);
    },
    [channelId, onOpenAgentSession],
  );

  // Scope state exists to satisfy the shared body; the thread view swaps
  // the turn picker for agent chips, so scope stays "session".
  const [scope, setScope] = React.useState<ActivityScope>("session");

  if (!scene || scene.events.length === 0) {
    return (
      <div
        className={cn(
          "rounded-lg border border-border/70 bg-muted/30 px-3 py-4 text-sm text-muted-foreground",
          className,
        )}
      >
        No agent file activity in this thread yet.
      </div>
    );
  }

  const chips = (
    <div className="flex min-w-0 flex-wrap items-center gap-1">
      <AgentChip
        active={agentFilter === null}
        color={null}
        label="All agents"
        onClick={() => setAgentFilter(null)}
      />
      {sceneAgentsOrInputs(threadAgents, sceneAgents).map((agent) => (
        <AgentChip
          key={agent.pubkey}
          active={agentFilter === agent.normalizedPubkey}
          color={colorForAgent(agent.pubkey, agent.index)}
          label={agent.name}
          working={workingSet.has(agent.normalizedPubkey)}
          onClick={() =>
            setAgentFilter((current) =>
              current === agent.normalizedPubkey
                ? null
                : agent.normalizedPubkey,
            )
          }
          onDoubleClick={() => openAgent(agent.pubkey)}
        />
      ))}
    </div>
  );

  return (
    <ActivityPanelBody
      key={`thread:${channelId}:${threadGroup.rootId ?? "unthreaded"}:${agentFilter ?? "all"}`}
      className={className}
      scene={scene}
      scopeIsLive={threadWorking}
      scope={scope}
      setScope={setScope}
      turns={EMPTY_TURNS}
      pickerSlot={chips}
      onRiderClick={openAgent}
    />
  );
}

interface ChipAgent {
  pubkey: string;
  normalizedPubkey: string;
  name: string;
  index: number;
}

export function threadScopeIsLive(
  agentsWithOpenTurn: ReadonlySet<string>,
  workingPubkeys: readonly string[],
): boolean {
  if (workingPubkeys.length === 0 || agentsWithOpenTurn.size === 0) {
    return false;
  }
  const workingSet = new Set(
    workingPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return [...agentsWithOpenTurn].some((pubkey) =>
    workingSet.has(normalizePubkey(pubkey)),
  );
}

export function agentsWithThreadActivity(
  agents: readonly MultiAgentActivityInput[],
  turnIdsByAgent: ReadonlyMap<string, ReadonlySet<string>>,
): MultiAgentActivityInput[] {
  const activePubkeys = new Set(
    [...turnIdsByAgent.entries()]
      .filter(([, turnIds]) => turnIds.size > 0)
      .map(([pubkey]) => normalizePubkey(pubkey)),
  );
  return agents.filter((agent) =>
    activePubkeys.has(normalizePubkey(agent.pubkey)),
  );
}

export function turnFilterForThreadAgents(
  agents: readonly MultiAgentActivityInput[],
  turnIdsByAgent: ReadonlyMap<string, ReadonlySet<string>>,
): ReadonlyMap<string, ReadonlySet<string>> {
  if (agents.length === 0) return turnIdsByAgent;
  const turnIdsByNormalizedPubkey = new Map(
    [...turnIdsByAgent.entries()].map(([pubkey, turnIds]) => [
      normalizePubkey(pubkey),
      turnIds,
    ]),
  );
  const filter = new Map<string, ReadonlySet<string>>();
  for (const agent of agents) {
    const turnIds = turnIdsByNormalizedPubkey.get(
      normalizePubkey(agent.pubkey),
    );
    if (turnIds?.size) filter.set(agent.pubkey, turnIds);
  }
  return filter;
}

/**
 * Chip list source: the thread-active input agent set, labeled with scene names
 * when derivation produced them (scene agents carry resolved display names).
 */
function sceneAgentsOrInputs(
  inputs: readonly MultiAgentActivityInput[],
  sceneAgents: readonly { id: string; name: string }[],
): ChipAgent[] {
  const nameById = new Map(
    sceneAgents.map((agent) => [normalizePubkey(agent.id), agent.name]),
  );
  return inputs.map((input, index) => {
    const normalizedPubkey = normalizePubkey(input.pubkey);
    return {
      pubkey: input.pubkey,
      normalizedPubkey,
      name:
        nameById.get(normalizedPubkey) ??
        input.name ??
        truncatePubkey(input.pubkey),
      index,
    };
  });
}

function AgentChip({
  active,
  color,
  label,
  working,
  onClick,
  onDoubleClick,
}: {
  active: boolean;
  color: string | null;
  label: string;
  working?: boolean;
  onClick: () => void;
  onDoubleClick?: () => void;
}) {
  return (
    <button
      className={cn(
        "flex h-6 items-center gap-1.5 rounded-full border px-2 text-xs transition-colors",
        active
          ? "border-ring/60 bg-accent text-accent-foreground"
          : "border-border/70 bg-background text-muted-foreground hover:bg-muted/70",
      )}
      title={onDoubleClick ? `${label} — double-click to open session` : label}
      type="button"
      onClick={onClick}
      onDoubleClick={onDoubleClick}
    >
      {color ? (
        <span
          aria-hidden
          className={cn("h-2 w-2 rounded-full", working && "animate-pulse")}
          style={{ backgroundColor: color }}
        />
      ) : null}
      <span className="max-w-32 truncate">{label}</span>
    </button>
  );
}
