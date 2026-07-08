/**
 * Restart plumbing for preview experiments whose effect is pinned at agent
 * spawn time (env vars set in `spawn_agent_child`). Toggling such an
 * experiment repaints the UI immediately, but running agents keep their
 * spawn-time env — so the settings card confirms with the user and restarts
 * running agents after the toggle is applied.
 *
 * Pure logic lives here so node tests can cover cancel/confirm/partial-
 * failure without a DOM.
 */
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Preview experiments that gate agent behavior via spawn-time env.
 * Toggling these prompts for an agent restart. Keep in sync with the
 * spawn gates in `desktop/src-tauri/src/managed_agents/runtime.rs`.
 */
const EXPERIMENTS_REQUIRING_AGENT_RESTART: ReadonlySet<string> = new Set([
  "acpToolSummaries",
]);

export function experimentRequiresAgentRestart(featureId: string): boolean {
  return EXPERIMENTS_REQUIRING_AGENT_RESTART.has(featureId);
}

type RestartCandidate = Pick<ManagedAgent, "pubkey" | "name" | "status"> & {
  backend: { type: string };
};

/**
 * Only locally spawned, currently running agents carry the spawn-time env
 * this restart exists to refresh. Stopped agents stay stopped; provider
 * deployments are not spawned through the local env path.
 */
export function selectAgentsToRestart<T extends RestartCandidate>(
  agents: readonly T[],
): T[] {
  return agents.filter(
    (agent) => agent.backend.type === "local" && agent.status === "running",
  );
}

export type AgentRestartOutcome = {
  restarted: number;
  failures: { name: string; error: string }[];
};

/**
 * Confirm-time orchestration, in the order that matters:
 *
 *   1. `applyToggle()` — flip the localStorage override (UI updates now).
 *   2. `await mirrorExperiments()` — push the override to the Rust side
 *      BEFORE any agent respawns, otherwise a restarted agent could read
 *      the stale mirror and spawn with the old env (the exact confusion
 *      this modal exists to fix). The passive `useDesktopExperimentsMirror`
 *      effect also fires, but it's async and unordered — hence the
 *      explicit await here.
 *   3. Restart the agents snapshotted at confirmation time.
 *
 * A failed mirror write aborts the restart (agents would respawn with the
 * old env anyway) but does NOT roll back the toggle — matching the
 * best-effort mirror semantics elsewhere; the mirror retries on next boot.
 */
export async function applyExperimentAndRestartAgents({
  applyToggle,
  mirrorExperiments,
  agents,
  startAgent,
  stopAgent,
}: {
  applyToggle: () => void;
  mirrorExperiments: () => Promise<void>;
  agents: readonly RestartCandidate[];
  startAgent: (pubkey: string) => Promise<unknown>;
  stopAgent: (pubkey: string) => Promise<unknown>;
}): Promise<AgentRestartOutcome> {
  applyToggle();
  await mirrorExperiments();
  return restartAgentsForExperiment({ agents, startAgent, stopAgent });
}

/**
 * Stop→start each agent (mirrors `respawnManagedAgentWithRules` semantics).
 * One agent's failure never blocks the others; failures are collected for
 * the caller's messaging. The experiment toggle is NOT rolled back on
 * failure — the setting applied, only the process refresh lagged.
 */
export async function restartAgentsForExperiment({
  agents,
  startAgent,
  stopAgent,
}: {
  agents: readonly RestartCandidate[];
  startAgent: (pubkey: string) => Promise<unknown>;
  stopAgent: (pubkey: string) => Promise<unknown>;
}): Promise<AgentRestartOutcome> {
  const results = await Promise.allSettled(
    agents.map(async (agent) => {
      await stopAgent(agent.pubkey);
      await startAgent(agent.pubkey);
    }),
  );

  const outcome: AgentRestartOutcome = { restarted: 0, failures: [] };
  results.forEach((result, index) => {
    if (result.status === "fulfilled") {
      outcome.restarted += 1;
      return;
    }
    const reason = result.reason;
    outcome.failures.push({
      name: agents[index]?.name ?? "unknown agent",
      error: reason instanceof Error ? reason.message : String(reason),
    });
  });
  return outcome;
}

/** Human-readable toast copy for a restart outcome. */
export function describeRestartOutcome(outcome: AgentRestartOutcome): {
  kind: "success" | "error";
  message: string;
} {
  const total = outcome.restarted + outcome.failures.length;
  if (outcome.failures.length === 0) {
    return {
      kind: "success",
      message:
        outcome.restarted === 1
          ? "Restarted 1 agent."
          : `Restarted ${outcome.restarted} agents.`,
    };
  }
  const names = outcome.failures.map((failure) => failure.name).join(", ");
  return {
    kind: "error",
    message: `Restarted ${outcome.restarted} of ${total} agents. Failed to restart: ${names}. The experiment setting was still applied — restart these agents manually to pick it up.`,
  };
}
