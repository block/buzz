/**
 * useAgentEditMergedSubmit — submit hook for AgentEditMergedDialog.
 *
 * Encapsulates isSaving/saveError state and the async submit function so
 * AgentEditMergedDialog stays under the desktop file-size gate.
 */

import * as React from "react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  personasQueryKey,
} from "@/features/agents/hooks";
import {
  setManagedAgentAutoRestart,
  setManagedAgentStartOnAppLaunch,
} from "@/shared/api/tauriManagedAgents";
import type {
  AgentPersona,
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  UpdateManagedAgentInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import type { PersonaSharePublicationResult } from "@/shared/api/tauriPersonas";
import {
  seedAgentFormModel,
  emitAgentFormDiff,
  type AgentEditContext,
  type AgentFormModel,
} from "./agentFormModel";
import { runAgentSaveCoordinator } from "./agentSaveCoordinator";
import { parsePersonaNamePoolText } from "./personaDialogState";

// ── Types ─────────────────────────────────────────────────────────────────────

export type AgentEditSubmitState = {
  ctx: AgentEditContext;
  displayName: string;
  avatarUrl: string;
  systemPrompt: string;
  namePoolText: string;
  model: string;
  provider: string;
  respondTo: string | null;
  respondToAllowlist: string[];
  parallelism: string;
  parsedParallelism: number;
  envVars: Record<string, string>;
  instanceEnvVars: Record<string, string>;
  instanceName: string;
  autoRestartOnConfigChange: boolean;
  startOnAppLaunch: boolean | undefined;
  /** D-field: definition runtime id (independent of I-harness pin). */
  definitionRuntimeId: string;
  /**
   * Ref tracking the auto-seeded definition runtime ID. When non-null, the
   * auto-seed was not a user choice — skip persisting runtime as a D-change
   * if no other D-field was actually modified.
   */
  autoSeededDefinitionRuntimeRef: React.RefObject<string | null>;
  /** I-field: harness pin runtime id (instance only). */
  selectedRuntimeId: string;
  inheritHarness: boolean;
  agentCommand: string;
  agentArgs: string;
  acpCommand: string;
  showInst: boolean;
  defReadOnly: boolean;
  inheritedSubmissionProvider: string | null;
  runtimes: readonly AcpRuntimeCatalogEntry[];
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
  updatePersonaAndPublish: (
    input: UpdatePersonaInput,
  ) => Promise<PersonaSharePublicationResult>;
  updateManagedAgent: (
    input: UpdateManagedAgentInput,
  ) => Promise<{ agent: ManagedAgent; profileSyncError: string | null }>;
  startMutate: (
    pubkey: string,
    callbacks: { onSuccess: () => void; onError: (err: unknown) => void },
  ) => void;
  onValidate?: () => string | null;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
};

export type AgentEditSubmitHookReturn = {
  isSaving: boolean;
  saveError: Error | null;
  handleSubmit: (canSubmit: boolean) => Promise<void>;
  resetSaveError: () => void;
};

/**
 * Build the canonical "next" AgentFormModel from live dialog state.
 *
 * Single source of truth for what the user is submitting: consumed by the
 * submit hook to emit the diff AND by the dialog to derive the D-field dirty
 * signal (`definitionFieldsDirty`). Keeping one builder means the dirty
 * affordance and the actual write can never disagree.
 */
export function buildNextAgentFormModel(
  seed: AgentFormModel,
  s: AgentEditSubmitState,
): AgentFormModel {
  const namePool = parsePersonaNamePoolText(s.namePoolText);
  const normalizedModel = (s.model || null) as string | null;
  const normalizedProvider = s.showInst
    ? (s.inheritedSubmissionProvider ?? null)
    : s.provider.trim() || null;

  // Resolve the effective harness command from the selected runtime (or the
  // manual entry) so the model — not a post-emit merge — carries it.
  const effectiveRuntime = s.runtimes.find(
    (r) => r.id === (s.selectedRuntimeId || "custom"),
  );
  const resolvedHarnessCommand = (
    effectiveRuntime?.command ?? s.agentCommand
  ).trim();
  const resolvedHarnessArgs = s.agentArgs
    .split(",")
    .map((a) => a.trim())
    .filter(Boolean);

  return {
    ...seed,
    displayName: s.displayName.trim(),
    avatarUrl: s.avatarUrl.trim(),
    systemPrompt: s.systemPrompt.trim(),
    respondTo: s.respondTo as typeof seed.respondTo,
    respondToAllowlist: s.respondToAllowlist,
    // D-field: use definitionRuntimeId (independent of I-harness pin).
    // If the runtime was auto-seeded (not a user choice), use the seed's
    // original runtime so a no-op save doesn't persist the app default
    // as a new definition runtime.
    runtime:
      s.autoSeededDefinitionRuntimeRef.current !== null &&
      s.autoSeededDefinitionRuntimeRef.current === s.definitionRuntimeId
        ? (seed.runtime ?? undefined) // preserve original (undefined = no runtime)
        : s.definitionRuntimeId === "custom"
          ? undefined
          : s.definitionRuntimeId,
    model: normalizedModel,
    provider: normalizedProvider,
    // D-field env: use the definition env from the form state (not the
    // live linkedPersonaEnvVars, which would bypass user edits).
    envVars: s.envVars,
    namePool,
    instanceName: s.instanceName.trim() || undefined,
    instanceEnvVars: s.showInst ? s.instanceEnvVars : undefined,
    // Parallelism is D-owned only in definition-only context, where the
    // backend clears it via an omitted member in the full-replacement behavior
    // group. Represent a blank field as an explicit clear (null) there so
    // `emitAgentFormDiff` dirties the D-field and emits the clearing value.
    // A valid positive value is carried through (the backend rejects
    // out-of-range and settlement surfaces that error); any other non-blank
    // value falls back to the seed, a safe no-op that never clears. A blank
    // field in instance context also keeps the seed — the instance
    // `parallelism` setter is `Option<u32>` with no clear-to-null wire shape.
    parallelism:
      s.parsedParallelism > 0
        ? s.parsedParallelism
        : s.parallelism.trim() === "" && s.ctx.kind === "definition-only"
          ? null
          : (seed.parallelism ?? null),
    // Harness pin (I-fields) — now first-class model fields; emitAgentFormDiff
    // settles inherit-vs-pin and the "" clear sentinel against the instance.
    harnessInherit: s.showInst ? s.inheritHarness : undefined,
    harnessCommand: s.showInst ? resolvedHarnessCommand : undefined,
    harnessArgs: s.showInst ? resolvedHarnessArgs : undefined,
    acpCommand: s.showInst ? s.acpCommand.trim() : undefined,
    autoRestartOnConfigChange: s.showInst
      ? s.autoRestartOnConfigChange
      : undefined,
    startOnAppLaunch: s.showInst ? s.startOnAppLaunch : seed.startOnAppLaunch,
  };
}

// ── Hook ──────────────────────────────────────────────────────────────────────

export function useAgentEditMergedSubmit(
  state: AgentEditSubmitState,
): AgentEditSubmitHookReturn {
  const queryClient = useQueryClient();
  const [isSaving, setIsSaving] = React.useState(false);
  const [saveError, setSaveError] = React.useState<Error | null>(null);

  // Keep a ref to the current state so handleSubmit always uses the latest
  // values without needing to be recreated on every render.
  const stateRef = React.useRef(state);
  stateRef.current = state;

  const handleSubmit = React.useCallback(
    async (canSubmit: boolean) => {
      if (!canSubmit) return;
      const s = stateRef.current;

      if (s.onValidate) {
        const err = s.onValidate();
        if (err) {
          toast.error(err);
          return;
        }
      }

      setSaveError(null);
      setIsSaving(true);

      const def = s.ctx.kind !== "instance-only" ? s.ctx.definition : null;
      const inst = s.ctx.kind !== "definition-only" ? s.ctx.instance : null;

      try {
        const seed = seedAgentFormModel(s.ctx);
        const next = buildNextAgentFormModel(seed, s);

        const { personaInput, agentInput, policySets } = emitAgentFormDiff(
          seed,
          next,
          s.ctx,
        );

        const refetchStores = async () => {
          // Use refetchQueries (not invalidateQueries) so the await resolves only
          // after the fresh data has been written to the cache. invalidateQueries
          // only marks the query stale; getQueryData immediately after still returns
          // the pre-save value, causing the coordinator's observed-state check to
          // see a phantom mismatch and leave the dialog open.
          await Promise.all([
            queryClient.refetchQueries({ queryKey: personasQueryKey }),
            queryClient.refetchQueries({ queryKey: managedAgentsQueryKey }),
          ]);
          const personas =
            queryClient.getQueryData<AgentPersona[]>(personasQueryKey) ?? [];
          const agents =
            queryClient.getQueryData<ManagedAgent[]>(managedAgentsQueryKey) ??
            [];
          return {
            persona: def
              ? (personas.find((p) => p.id === def.id) ?? null)
              : null,
            agent: inst
              ? (agents.find((a) => a.pubkey === inst.pubkey) ?? null)
              : null,
          };
        };

        const success = await runAgentSaveCoordinator({
          ctx: s.ctx,
          personaInput,
          agentInput,
          policySets,
          publishCatalogUpdates: !!(def?.shared && !s.defReadOnly),
          runtimes: s.runtimes.length > 0 ? s.runtimes : undefined,
          updatePersona: s.updatePersona,
          updatePersonaAndPublish: s.updatePersonaAndPublish,
          updateManagedAgent: async (upd) => {
            if (!inst)
              throw new Error("No instance in definition-only context");
            return s.updateManagedAgent(upd);
          },
          setAutoRestart: (pk, v) => setManagedAgentAutoRestart(pk, v),
          setStartOnAppLaunch: (pk, v) =>
            setManagedAgentStartOnAppLaunch(pk, v),
          refetchStores,
          onDone: () => s.onOpenChange(false),
          onSavedWhileStopped: (agent) => {
            const name = agent.name;
            toast(`${name} saved while stopped.`, {
              action: {
                label: "Start now",
                onClick: () =>
                  s.startMutate(agent.pubkey, {
                    onSuccess: () => toast.success(`${name} started.`),
                    onError: (err) =>
                      toast.error(
                        err instanceof Error
                          ? `${name} failed to start: ${err.message}`
                          : `${name} failed to start.`,
                      ),
                  }),
              },
            });
          },
        });

        if (!success) {
          setSaveError(
            new Error("Some changes may not have persisted. Reopen to retry."),
          );
        }
        if (success && inst) {
          const agents =
            queryClient.getQueryData<ManagedAgent[]>(managedAgentsQueryKey) ??
            [];
          const updated = agents.find((a) => a.pubkey === inst.pubkey);
          if (updated) s.onUpdated?.(updated);
        }
      } finally {
        setIsSaving(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [queryClient],
  );

  return {
    isSaving,
    saveError,
    handleSubmit,
    resetSaveError: () => setSaveError(null),
  };
}
