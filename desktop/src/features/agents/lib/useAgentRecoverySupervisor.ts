import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";
import { getAgentWorkingState } from "@/features/agents/agentWorkingSignal";
import { sendDesktopNotification } from "@/features/notifications/lib/desktop";
import {
  listManagedAgentRuntimes,
  restartManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import {
  beginAgentRecovery,
  recordFailedRecoveryAttempt,
  recoveryAttemptDue,
  recoveryExhausted,
  recoveryLifecycleHealthy,
  type AgentRecoveryState,
} from "./agentRecoveryPolicy";
import { createCompletionPollScheduler } from "./completionPollScheduler";

const HEALTH_POLL_MS = 5_000;

type PairRecovery = AgentRecoveryState & {
  pubkey: string;
  relayUrl: string;
  agentName: string;
  inFlight: boolean;
  awaitingHealth: boolean;
  exhaustedNotified: boolean;
};

function pairKey(pubkey: string, relayUrl: string): string {
  return JSON.stringify([pubkey, relayUrl]);
}

function notifyDesktop(title: string, body: string): void {
  void sendDesktopNotification({ title, body });
}

/**
 * Keeps opted-in local agent listeners reachable after an unexpected exit.
 * Recovery is bounded (5s, 30s, 2m), never interrupts an active turn, and
 * reports both exhaustion and successful recovery in-app and on the desktop.
 */
export function useAgentRecoverySupervisor(): void {
  const queryClient = useQueryClient();
  const agents = useManagedAgentsQuery().data;
  const agentsRef = React.useRef(agents);
  const recoveriesRef = React.useRef(new Map<string, PairRecovery>());
  agentsRef.current = agents;

  React.useEffect(() => {
    let cancelled = false;
    let pollInFlight = false;

    async function poll(): Promise<void> {
      if (pollInFlight || cancelled) return;
      pollInFlight = true;
      try {
        const currentAgents = agentsRef.current;
        if (!currentAgents) return;
        const eligible = new Map(
          currentAgents
            .filter(
              (agent) =>
                agent.backend.type === "local" && agent.startOnAppLaunch,
            )
            .map((agent) => [agent.pubkey.toLowerCase(), agent]),
        );

        let runtimes: ManagedAgentRuntimeStatus[];
        try {
          runtimes = await listManagedAgentRuntimes();
        } catch {
          return;
        }
        if (cancelled) return;
        queryClient.setQueryData(managedAgentRuntimesQueryKey, runtimes);

        const now = Date.now();
        const healthyPairs = new Set<string>();
        for (const runtime of runtimes) {
          const agent = eligible.get(runtime.pubkey.toLowerCase());
          if (!agent) continue;
          const key = pairKey(runtime.pubkey, runtime.relayUrl);
          if (recoveryLifecycleHealthy(runtime.lifecycle)) {
            healthyPairs.add(key);
            const prior = recoveriesRef.current.get(key);
            if (prior) {
              recoveriesRef.current.delete(key);
              toast.success(`${agent.name} is reachable again`);
              notifyDesktop(
                "Buzz agent recovered",
                `${agent.name} is listening again.`,
              );
            }
            continue;
          }
          if (runtime.lifecycle === "starting") continue;
          const existingRecovery = recoveriesRef.current.get(key);
          if (!existingRecovery) {
            recoveriesRef.current.set(key, {
              ...beginAgentRecovery(now, runtime.error),
              pubkey: runtime.pubkey,
              relayUrl: runtime.relayUrl,
              agentName: agent.name,
              inFlight: false,
              awaitingHealth: false,
              exhaustedNotified: false,
            });
            toast.warning(`${agent.name} listener failed; recovery scheduled`);
          } else if (existingRecovery.awaitingHealth) {
            const updated = recordFailedRecoveryAttempt(
              existingRecovery,
              now,
              runtime.error,
            );
            recoveriesRef.current.set(key, {
              ...existingRecovery,
              ...updated,
              inFlight: false,
              awaitingHealth: false,
            });
          }
        }

        for (const [key, recovery] of recoveriesRef.current) {
          if (
            healthyPairs.has(key) ||
            recovery.inFlight ||
            recovery.awaitingHealth
          ) {
            continue;
          }
          if (recoveryExhausted(recovery)) {
            if (!recovery.exhaustedNotified) {
              recovery.exhaustedNotified = true;
              toast.error(`${recovery.agentName} could not be recovered`);
              notifyDesktop(
                "Buzz agent needs attention",
                `${recovery.agentName} failed after 3 recovery attempts.`,
              );
            }
            continue;
          }
          if (
            !recoveryAttemptDue(
              recovery,
              now,
              getAgentWorkingState(recovery.pubkey).working,
            )
          ) {
            continue;
          }

          recovery.inFlight = true;
          void restartManagedAgentRuntime(recovery.pubkey, recovery.relayUrl)
            .then((runtime) => {
              if (cancelled) return;
              recoveriesRef.current.set(key, {
                ...recovery,
                inFlight: false,
                awaitingHealth: true,
              });
              queryClient.setQueryData<ManagedAgentRuntimeStatus[]>(
                managedAgentRuntimesQueryKey,
                (current = []) => [
                  ...current.filter(
                    (candidate) =>
                      pairKey(candidate.pubkey, candidate.relayUrl) !== key,
                  ),
                  runtime,
                ],
              );
            })
            .catch((error: unknown) => {
              if (cancelled) return;
              const updated = recordFailedRecoveryAttempt(
                recovery,
                Date.now(),
                error instanceof Error ? error.message : String(error),
              );
              recoveriesRef.current.set(key, {
                ...recovery,
                ...updated,
                inFlight: false,
                awaitingHealth: false,
              });
            });
        }
      } finally {
        pollInFlight = false;
      }
    }

    const scheduler = createCompletionPollScheduler({
      poll,
      delayMs: HEALTH_POLL_MS,
    });
    return () => {
      cancelled = true;
      scheduler.stop();
    };
  }, [queryClient]);
}
