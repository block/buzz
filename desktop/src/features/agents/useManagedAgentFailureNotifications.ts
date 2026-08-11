import * as React from "react";
import { toast } from "sonner";

import {
  useManagedAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import type { ManagedAgent } from "@/shared/api/types";

type FailureCursor = ReadonlyMap<string, string | null>;

export function findNewManagedAgentFailures(
  previous: FailureCursor,
  agents: readonly ManagedAgent[],
): ManagedAgent[] {
  return agents.filter(
    (agent) =>
      agent.status === "stopped" &&
      agent.lastError !== null &&
      agent.lastStoppedAt !== null &&
      previous.has(agent.pubkey) &&
      previous.get(agent.pubkey) !== agent.lastStoppedAt,
  );
}

function failureCursor(
  agents: readonly ManagedAgent[],
): Map<string, string | null> {
  return new Map(agents.map((agent) => [agent.pubkey, agent.lastStoppedAt]));
}

export function useManagedAgentFailureNotifications(): void {
  const agents = useManagedAgentsQuery().data;
  const startAgent = useStartManagedAgentMutation();
  const previousRef = React.useRef<Map<string, string | null> | null>(null);

  React.useEffect(() => {
    if (!agents) return;

    const previous = previousRef.current;
    previousRef.current = failureCursor(agents);
    if (!previous) return;

    for (const agent of findNewManagedAgentFailures(previous, agents)) {
      const description =
        friendlyAgentLastError(agent.lastError, agent.lastErrorCode)?.copy ??
        agent.lastError ??
        "The agent stopped unexpectedly.";
      const toastId = `managed-agent-failure-${agent.pubkey}-${agent.lastStoppedAt}`;

      toast.error(`${agent.name} stopped`, {
        action: {
          label: "Retry",
          onClick: (event) => {
            event.preventDefault();
            startAgent.mutate(agent.pubkey, {
              onError: (error) => {
                window.setTimeout(() => {
                  toast.error(`Couldn't restart ${agent.name}`, {
                    description:
                      error instanceof Error
                        ? error.message
                        : "Agent startup failed.",
                  });
                }, 0);
              },
            });
          },
        },
        description,
        duration: 15_000,
        id: toastId,
      });
    }
  }, [agents, startAgent]);
}
