/**
 * useAgentEditMergedSubmit — submit hook for AgentEditMergedDialog.
 *
 * Encapsulates isSaving/saveError state and the async submit function so
 * AgentEditMergedDialog stays under the desktop file-size gate.
 */

import * as React from "react";
import { toast } from "sonner";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";

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
import { setPersonaShared } from "@/shared/api/tauriPersonas";
import type { PersonaSharePublicationResult } from "@/shared/api/tauriPersonas";
import {
  seedAgentFormModel,
  emitAgentFormDiff,
  type AgentEditContext,
  type AgentFormModel,
} from "./agentFormModel";
import { runAgentSaveCoordinator } from "./agentSaveCoordinator";
import { parsePersonaNamePoolText } from "./personaDialogState";
import { resolveEffortSubmission } from "./personaRuntimeModel";

// ── Types ─────────────────────────────────────────────────────────────────────

export type AgentEditSubmitState = {
  ctx: AgentEditContext;
  displayName: string;
  avatarUrl: string;
  description: string;
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
  /**
   * Definition `updatedAt` captured when the form was seeded. Passed to the
   * coordinator's concurrent-edit guard: a definition write is aborted if the
   * latest ctx definition was revised since the form opened.
   */
  seededDefinitionUpdatedAt: string | null;
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
  /** Controlled pending effort selection from the dialog (null = adapter default). */
  effortLevel: string | null;
  /** Ref tracking whether the user has made an explicit effort selection. */
  effortTouched: React.RefObject<boolean>;
  /**
   * The persisted effort level from the config surface at seed time
   * (`configSurfaceQuery.data?.normalized.thinkingEffort?.value ?? null`).
   * Used by `resolveEffortSubmission` to suppress unchanged selections so a
   * name-only edit never rewrites the effort column.
   */
  originalEffortLevel: string | null;
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
    description: s.description,
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

// ── Store refetch adapter ──────────────────────────────────────────────────────

/**
 * Refetch both agent stores and return the current (persona, agent) pair for
 * the edited entities. This is the production `refetchStores` seam consumed by
 * the save coordinator's verification step.
 *
 * `refetchQueries` (not `invalidateQueries`) so the await resolves only after
 * the fresh data has been written to the cache — `invalidateQueries` only marks
 * a query stale, so `getQueryData` immediately after would still return the
 * pre-save value and the coordinator's observed-state check would see a phantom
 * mismatch and leave the dialog open.
 *
 * `{ throwOnError: true }` is REQUIRED: TanStack Query v5 swallows refetch
 * errors by default (`refetchQueries` resolves even when the underlying fetch
 * rejects), so without it a failed verification refetch would resolve, then
 * `getQueryData` would read the retained STALE cache and the coordinator would
 * misclassify a possibly-committed write as an observed non-persist. With it, a
 * failed refetch rejects and flows into the coordinator's verification-unknown
 * path — persistence reported as unknown, never as a failed write.
 */
export async function refetchAgentStores(
  queryClient: Pick<QueryClient, "refetchQueries" | "getQueryData">,
  def: { id: string } | null,
  inst: { pubkey: string } | null,
): Promise<{ persona: AgentPersona | null; agent: ManagedAgent | null }> {
  await Promise.all([
    queryClient.refetchQueries(
      { queryKey: personasQueryKey },
      { throwOnError: true },
    ),
    queryClient.refetchQueries(
      { queryKey: managedAgentsQueryKey },
      { throwOnError: true },
    ),
  ]);
  const personas =
    queryClient.getQueryData<AgentPersona[]>(personasQueryKey) ?? [];
  const agents =
    queryClient.getQueryData<ManagedAgent[]>(managedAgentsQueryKey) ?? [];
  return {
    persona: def ? (personas.find((p) => p.id === def.id) ?? null) : null,
    agent: inst ? (agents.find((a) => a.pubkey === inst.pubkey) ?? null) : null,
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

        const {
          personaInput,
          agentInput: rawAgentInput,
          policySets,
        } = emitAgentFormDiff(seed, next, s.ctx);

        // Effort write: when touched, resolve via resolveEffortSubmission
        // (mirrors the deleted AgentInstanceEditDialog's PR #4625 semantics):
        //   - suppresses when agentCommand="" (pin→inherit) so the inherit
        //     transition does not restore the effort column it just cleared
        //   - suppresses unchanged selections (no-op save never rewrites the column)
        // Only includes effortLevel in the locked update when persist=true.
        let agentInput = rawAgentInput;
        if (s.effortTouched.current && inst) {
          const effortSubmission = resolveEffortSubmission({
            effortLevel: s.effortLevel,
            originalEffortLevel: s.originalEffortLevel,
            inheritTransition: rawAgentInput?.agentCommand === "",
          });
          if (effortSubmission.persist) {
            agentInput = agentInput ?? { pubkey: inst.pubkey };
            agentInput = { ...agentInput, effortLevel: effortSubmission.level };
          }
        }

        const refetchStores = () => refetchAgentStores(queryClient, def, inst);

        const success = await runAgentSaveCoordinator({
          ctx: s.ctx,
          personaInput,
          agentInput,
          policySets,
          publishCatalogUpdates: !!(def?.shared && !s.defReadOnly),
          expectedDefinitionUpdatedAt: s.seededDefinitionUpdatedAt,
          runtimes: s.runtimes.length > 0 ? s.runtimes : undefined,
          updatePersona: s.updatePersona,
          updatePersonaAndPublish: s.updatePersonaAndPublish,
          // Publish-only retry: when save+publish succeeds on disk but the
          // publication step throws, re-publish the current on-disk head via
          // set_persona_shared without writing any field changes.  The persona
          // is already shared=true (it was shared when the save+publish path
          // was reached), so passing true re-publishes the current head.
          publishRetry: (id: string) => setPersonaShared(id, true),
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
          // The coordinator already reported the specific failure via toast.
          // This error banner is a secondary indicator for users who miss the
          // toast — use an accurate summary that covers all false-return paths.
          setSaveError(
            new Error(
              "Some changes may not have fully saved — see the notification above for details.",
            ),
          );
        }
        if (success && inst) {
          const agents =
            queryClient.getQueryData<ManagedAgent[]>(managedAgentsQueryKey) ??
            [];
          const updated = agents.find((a) => a.pubkey === inst.pubkey);
          if (updated) s.onUpdated?.(updated);
        }
      } catch (err) {
        // Belt-and-braces: the coordinator already contains every settlement
        // rejection, but nothing thrown inside the submit path may escape
        // silently — an uncaught rejection would leave the dialog stuck on
        // "Saving..." with no error. Surface it as an unverified-save state.
        setSaveError(
          new Error(
            err instanceof Error && err.message
              ? `Could not verify whether your changes saved: ${err.message}. Reopen to check before retrying.`
              : "Could not verify whether your changes saved. Reopen to check before retrying.",
          ),
        );
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
