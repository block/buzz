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
import { truncatePubkey } from "@/shared/lib/pubkey";
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

export function ThreadActivityPanel({
  channelId,
  agents,
  threadGroup,
  className,
}: ThreadActivityPanelProps) {
  const { onOpenAgentSession } = useAgentSession();
  const [agentFilter, setAgentFilter] = React.useState<string | null>(null);

  const { scene, agents: sceneAgents } = useMultiAgentActivityScene(
    agentFilter
      ? agents.filter((agent) => agent.pubkey === agentFilter)
      : agents,
    channelId,
    threadGroup.turnIdsByAgent,
  );

  // Reset a filter pointing at an agent that no longer has activity.
  React.useEffect(() => {
    if (!agentFilter) return;
    if (!agents.some((agent) => agent.pubkey === agentFilter)) {
      setAgentFilter(null);
    }
  }, [agentFilter, agents]);

  const workingPubkeys = useChannelWorkingAgentPubkeys(channelId);
  const anyWorking = workingPubkeys.length > 0;

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
      {sceneAgentsOrInputs(agents, sceneAgents).map((agent) => (
        <AgentChip
          key={agent.pubkey}
          active={agentFilter === agent.pubkey}
          color={colorForAgent(agent.pubkey, agent.index)}
          label={agent.name}
          working={workingPubkeys.includes(agent.pubkey)}
          onClick={() =>
            setAgentFilter((current) =>
              current === agent.pubkey ? null : agent.pubkey,
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
      scopeIsLive={anyWorking}
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
  name: string;
  index: number;
}

/**
 * Chip list source: the input agent set, labeled with scene names when the
 * derivation produced them (scene agents carry resolved display names).
 */
function sceneAgentsOrInputs(
  inputs: readonly MultiAgentActivityInput[],
  sceneAgents: readonly { id: string; name: string }[],
): ChipAgent[] {
  const nameById = new Map(sceneAgents.map((agent) => [agent.id, agent.name]));
  return inputs.map((input, index) => ({
    pubkey: input.pubkey,
    name:
      nameById.get(input.pubkey) ?? input.name ?? truncatePubkey(input.pubkey),
    index,
  }));
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
