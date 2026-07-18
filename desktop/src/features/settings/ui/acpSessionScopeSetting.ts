export type SessionScopeAgent = {
  pubkey: string;
  status: string;
  backend: { type: string };
};

export type SessionScopeDependencies = {
  setBackend: (scope: "thread" | "channel") => Promise<void>;
  listAgents: () => Promise<SessionScopeAgent[]>;
  stopAgent: (pubkey: string) => Promise<unknown>;
  startAgent: (pubkey: string) => Promise<unknown>;
  setUi: (threadScoped: boolean) => void;
};

async function restartRunningLocalAgents(
  agents: SessionScopeAgent[],
  deps: SessionScopeDependencies,
): Promise<void> {
  for (const agent of agents) {
    if (agent.status !== "running" || agent.backend.type !== "local") continue;
    await deps.stopAgent(agent.pubkey);
    await deps.startAgent(agent.pubkey);
  }
}

/**
 * Apply the Rust-owned session-scope setting and restart affected processes. The UI is
 * committed only after every restart succeeds. On failure, both persisted
 * backend state and already-restarted agents are restored best-effort.
 */
export async function applyAcpSessionScopeSetting(
  previous: boolean,
  next: boolean,
  deps: SessionScopeDependencies,
): Promise<void> {
  const agents = await deps.listAgents();
  try {
    await deps.setBackend(next ? "thread" : "channel");
    await restartRunningLocalAgents(agents, deps);
    deps.setUi(next);
  } catch (error) {
    try {
      await deps.setBackend(previous ? "thread" : "channel");
    } catch (rollbackError) {
      console.error(
        "Failed to roll back ACP session-scope backend state",
        rollbackError,
      );
    }
    for (const agent of agents) {
      if (agent.status !== "running" || agent.backend.type !== "local")
        continue;
      try {
        await deps.stopAgent(agent.pubkey);
        await deps.startAgent(agent.pubkey);
      } catch (rollbackError) {
        console.error(
          `Failed to roll back ACP session-scope process ${agent.pubkey}`,
          rollbackError,
        );
      }
    }
    deps.setUi(previous);
    throw error;
  }
}
