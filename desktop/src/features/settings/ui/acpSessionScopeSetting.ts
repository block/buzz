export type SessionScopeAgent = {
  pubkey: string;
  status: string;
  backend: { type: string };
};

export type SessionScopeDependencies = {
  setBackend: (scope: "thread" | "channel") => Promise<void>;
  getBackend: () => Promise<"thread" | "channel">;
  listAgents: () => Promise<SessionScopeAgent[]>;
  stopAgent: (pubkey: string) => Promise<unknown>;
  startAgent: (pubkey: string) => Promise<unknown>;
  setUi: (threadScoped: boolean) => void;
  /** The authoritative backend scope could not be restored or read: the UI
   *  must surface a hard recovery state instead of claiming any scope. */
  onUnrecoverable: () => void;
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
 * committed only after every restart succeeds.
 *
 * Failure invariant: persisted setting, Rust in-memory value, UI, and affected
 * processes must converge on one authoritative scope. Rollback is only claimed
 * after the rollback write is confirmed; if that write fails, the authoritative
 * value is re-read and UI/processes reconcile to it. If the authority cannot be
 * read, or any process fails to reconcile under it, `onUnrecoverable` fires and
 * nothing pretends to know the scope.
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
    // Establish the authoritative backend scope before touching UI or
    // processes: preferably by restoring `previous`, otherwise by reading
    // what actually persisted.
    let authoritative: boolean;
    try {
      await deps.setBackend(previous ? "thread" : "channel");
      authoritative = previous;
    } catch (rollbackError) {
      console.error(
        "Failed to roll back ACP session-scope backend state",
        rollbackError,
      );
      try {
        authoritative = (await deps.getBackend()) === "thread";
      } catch (readError) {
        console.error(
          "Failed to read authoritative ACP session scope after rollback failure",
          readError,
        );
        deps.onUnrecoverable();
        throw error;
      }
    }
    // Reconcile every affected process under the confirmed authoritative
    // scope — never under an assumed one. Any failure here means a process
    // may still be running under the wrong scope (or not running at all),
    // so convergence must not be claimed: finish every attempt, then
    // surface hard recovery instead of committing a normal UI scope.
    let reconciliationFailed = false;
    for (const agent of agents) {
      if (agent.status !== "running" || agent.backend.type !== "local")
        continue;
      try {
        await deps.stopAgent(agent.pubkey);
        await deps.startAgent(agent.pubkey);
      } catch (rollbackError) {
        reconciliationFailed = true;
        console.error(
          `Failed to roll back ACP session-scope process ${agent.pubkey}`,
          rollbackError,
        );
      }
    }
    if (reconciliationFailed) {
      deps.onUnrecoverable();
    } else {
      deps.setUi(authoritative);
    }
    throw error;
  }
}
