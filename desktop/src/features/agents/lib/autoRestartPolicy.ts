import type { AgentWorkingSource } from "../agentWorkingSignal";

/**
 * Chunk F auto-restart policy — the pure decision core.
 *
 * SAFETY-CRITICAL: the stop command is SIGTERM → ≤1s → SIGKILL with no
 * in-process drain, so this predicate is the ONLY thing standing between
 * the policy loop and killing a mid-turn agent. Every never-fire condition
 * below is load-bearing; the test matrix enumerates them exhaustively.
 *
 * Scope: the stop/start commands the loop fires are pair-scoped to the
 * active community, and `needsRestart` reports drift for that same pair —
 * a fire bounces only the community being viewed, never an agent's pairs
 * in other communities.
 *
 * Decisions:
 * - "fire": restart now (all gates green, continuity window satisfied).
 * - "arm": conditions are green but the quiescence window is still
 *   accumulating — keep the timer running.
 * - "hold": some gate is red — reset any accumulated quiescence and show
 *   the badge only.
 */
export type AutoRestartDecision = "fire" | "arm" | "hold";

export type AutoRestartInputs = {
  /** Per-agent opt-out toggle (record field, default ON). */
  autoRestartEnabled: boolean;
  /** Config drift detected by the summary poll (`needsRestart`). */
  needsRestart: boolean;
  /** Unified working signal for this agent (any channel). */
  working: boolean;
  /** Strongest working-signal source; "none" is ambiguous (idle OR absent
   * observer stream) and therefore never sufficient to fire on its own —
   * the connected gate plus the continuity window carry that risk. */
  workingSource: AgentWorkingSource;
  /** Observer relay connection state; anything but "connected" inhibits. */
  connected: boolean;
  /** Only local agents can be restarted by this loop. */
  isLocalBackend: boolean;
  /** Agent process status from the summary ("running" required). */
  isRunning: boolean;
  /** Edge-trigger state: true when this needsRestart rising edge has
   * already consumed its one attempt (failed or in flight). */
  edgeConsumed: boolean;
  /** Milliseconds the fire-conditions have held continuously. */
  quiescentForMs: number;
};

/** Continuity window: fire-conditions must hold this long uninterrupted.
 * 3 minutes = 18× the 10s turn-liveness cadence — comfortably beyond any
 * relay hiccup that could make a mid-turn agent look idle (the turn store
 * prunes after only 25s, which is why this window is minutes-scale). */
export const AUTO_RESTART_QUIESCENCE_MS = 3 * 60 * 1000;

export function decideAutoRestart(
  inputs: AutoRestartInputs,
): AutoRestartDecision {
  const {
    autoRestartEnabled,
    needsRestart,
    working,
    workingSource,
    connected,
    isLocalBackend,
    isRunning,
    edgeConsumed,
    quiescentForMs,
  } = inputs;

  // Never-fire gates. Each resets the continuity window ("hold").
  if (!autoRestartEnabled) return "hold";
  if (!needsRestart) return "hold";
  if (!isLocalBackend) return "hold";
  if (!isRunning) return "hold";
  if (!connected) return "hold";
  // Any working signal — observer OR typing — defers. `working` and
  // `workingSource` travel together, but check both so a partial reader
  // can never slip through.
  if (working || workingSource !== "none") return "hold";
  // One attempt per rising edge: a consumed edge badges until it cycles.
  if (edgeConsumed) return "hold";

  return quiescentForMs >= AUTO_RESTART_QUIESCENCE_MS ? "fire" : "arm";
}

/**
 * Per-agent edge-trigger state, keyed by pubkey in the policy hook.
 *
 * Rearm rule (Pinky's review row): the edge resets when `needsRestart`
 * falls OR when the agent stops — a manual stop/start cycle re-arms the
 * edge so a subsequently drifting agent auto-fires again.
 */
export type AutoRestartEdgeState = {
  consumed: boolean;
  /** Wall-clock ms when fire-conditions began holding; null = not armed. */
  armedAt: number | null;
};

export function nextEdgeState(
  previous: AutoRestartEdgeState | undefined,
  inputs: { needsRestart: boolean; isRunning: boolean },
): AutoRestartEdgeState {
  const prior = previous ?? { consumed: false, armedAt: null };
  // Falling edge or a stopped agent re-arms.
  if (!inputs.needsRestart || !inputs.isRunning) {
    return { consumed: false, armedAt: null };
  }
  return prior;
}
