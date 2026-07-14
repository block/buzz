import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import { listManagedAgents } from "@/shared/api/tauri";
import type { ManagedAgent } from "@/shared/api/types";
import { getAgentObserverSnapshot } from "../observerRelayStore";
import { getAgentWorkingState } from "../agentWorkingSignal";
import {
  decideAutoRestart,
  nextEdgeState,
  type AutoRestartEdgeState,
} from "./autoRestartPolicy";

/** How often the policy re-evaluates between summary refetches. Keeps the
 * continuity clock honest without waiting for the next 5s poll. */
const POLICY_TICK_MS = 15_000;

/**
 * Chunk F policy loop: watches managed-agent summaries and auto-restarts
 * drifted, idle, connected, local agents (per-agent opt-out, default ON).
 *
 * All decision logic lives in `decideAutoRestart` (pure, exhaustively
 * tested). This hook only wires inputs, owns per-pubkey edge state, and
 * calls the existing stop/start commands — both idempotent and serialized
 * on the backend store lock, so a cross-window double-fire is benign (and
 * further shrunk by the pre-fire summary re-fetch).
 */
export function useAutoRestartPolicy() {
  const queryClient = useQueryClient();
  const agents: ManagedAgent[] | undefined = useManagedAgentsQuery().data;
  const edgesRef = React.useRef(new Map<string, AutoRestartEdgeState>());
  const inFlightRef = React.useRef(new Set<string>());
  const [, setTick] = React.useState(0);

  // Re-evaluate on an interval so the quiescence clock advances even when
  // summaries and observer stores are quiet.
  React.useEffect(() => {
    const timer = setInterval(() => setTick((t) => t + 1), POLICY_TICK_MS);
    return () => clearInterval(timer);
  }, []);

  // No dependency array by design: the tick pattern re-runs this effect
  // every render so it reads live store state; all mutation is ref-local.
  React.useEffect(() => {
    if (!agents) return;
    const now = Date.now();
    const edges = edgesRef.current;

    for (const agent of agents) {
      const isRunning = agent.status === "running";
      const edge = nextEdgeState(edges.get(agent.pubkey), {
        needsRestart: agent.needsRestart,
        isRunning,
      });

      const working = getAgentWorkingState(agent.pubkey);
      const observer = getAgentObserverSnapshot(agent.pubkey, true);

      const decision = decideAutoRestart({
        autoRestartEnabled: agent.autoRestartOnConfigChange,
        needsRestart: agent.needsRestart,
        working: working.working,
        workingSource: working.source,
        connected: observer.connectionState === "open",
        isLocalBackend: agent.backend.type === "local",
        isRunning,
        edgeConsumed: edge.consumed,
        quiescentForMs: edge.armedAt === null ? 0 : now - edge.armedAt,
      });

      if (decision === "hold") {
        edges.set(agent.pubkey, { ...edge, armedAt: null });
        continue;
      }
      if (decision === "arm") {
        edges.set(agent.pubkey, {
          ...edge,
          armedAt: edge.armedAt ?? now,
        });
        continue;
      }

      // decision === "fire"
      if (inFlightRef.current.has(agent.pubkey)) continue;
      inFlightRef.current.add(agent.pubkey);
      // Consume the edge BEFORE the attempt: a failed restart badges only
      // until needsRestart cycles (edge-triggered debounce, no retry loops).
      edges.set(agent.pubkey, { consumed: true, armedAt: null });

      void (async () => {
        try {
          // Pre-fire re-fetch: shrink the stale-decision window to ~0.
          const fresh = await listManagedAgents();
          const current = fresh.find((a) => a.pubkey === agent.pubkey);
          if (
            !current?.needsRestart ||
            !current.autoRestartOnConfigChange ||
            current.status !== "running" ||
            getAgentWorkingState(agent.pubkey).source !== "none"
          ) {
            return;
          }
          await stopManagedAgent(agent.pubkey);
          await startManagedAgent(agent.pubkey);
        } catch {
          // Failed attempt: edge stays consumed — badge-only until the
          // needsRestart edge cycles. No retry loops by design.
        } finally {
          inFlightRef.current.delete(agent.pubkey);
          void queryClient.invalidateQueries({
            queryKey: managedAgentsQueryKey,
          });
        }
      })();
    }

    // Drop edge state for agents that no longer exist.
    const known = new Set(agents.map((a) => a.pubkey));
    for (const pubkey of edges.keys()) {
      if (!known.has(pubkey)) edges.delete(pubkey);
    }
  });
}
