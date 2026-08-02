import * as React from "react";

import { useChangeManagedAgentBackendMutation } from "@/features/agents/backendChangeHooks";
import type { ManagedAgent } from "@/shared/api/types";
import {
  canSubmitWhereToRun,
  resolveEffectiveBackendChange,
  whereToRunDraftForBackend,
  type WhereToRunDraft,
} from "./whereToRunIntent";

export type BackendSwapOutcome =
  | { status: "unchanged" }
  | { status: "cancelled" }
  | { status: "failed" }
  | { status: "swapped"; agent: ManagedAgent };

/**
 * The instance-edit dialog's backend-swap concern: owns the "Run on" draft,
 * the swap mutation, and the swap-specific error channel. The field patch and
 * the swap are two commands and the patch can succeed while the swap fails,
 * so this error is deliberately separate from `updateMutation.error` — the
 * dialog can say which half needs attention.
 */
export function useEditAgentBackendSwap(agent: ManagedAgent) {
  const mutation = useChangeManagedAgentBackendMutation();
  const [runDraft, setRunDraft] = React.useState<WhereToRunDraft>(() =>
    whereToRunDraftForBackend(agent.backend),
  );
  const [error, setError] = React.useState<Error | null>(null);

  const resetMutation = mutation.reset;
  const reset = React.useCallback(() => {
    setRunDraft(whereToRunDraftForBackend(agent.backend));
    setError(null);
    resetMutation();
  }, [agent.backend, resetMutation]);

  /**
   * Run the swap for the just-saved agent — AFTER the field patch, so an
   * execution-node deploy snapshots the saved record. "failed" is returned
   * after capturing the error; the caller keeps the dialog open so the
   * failure is visible and retryable (re-saving re-runs only the swap once
   * the fields match).
   */
  async function perform(
    savedAgent: ManagedAgent,
    runtimeId: string | undefined,
  ): Promise<BackendSwapOutcome> {
    const intent = resolveEffectiveBackendChange(runDraft, agent);
    if (!intent) return { status: "unchanged" };
    setError(null);
    try {
      const result = await mutation.mutateAsync({
        agent: savedAgent,
        intent,
        runtimeId,
      });
      if (result.cancelled) return { status: "cancelled" };
      return { status: "swapped", agent: result.agent };
    } catch (cause) {
      setError(cause instanceof Error ? cause : new Error(String(cause)));
      return { status: "failed" };
    }
  }

  return {
    canSubmit: canSubmitWhereToRun(runDraft),
    error,
    isPending: mutation.isPending,
    perform,
    reset,
    runDraft,
    setRunDraft,
  };
}
