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
  respondTo: string;
  respondToAllowlist: string[];
  parallelism: string;
  parsedParallelism: number;
  envVars: Record<string, string>;
  instanceEnvVars: Record<string, string>;
  instanceName: string;
  autoRestartOnConfigChange: boolean;
  selectedRuntimeId: string;
  inheritHarness: boolean;
  agentCommand: string;
  agentArgs: string;
  acpCommand: string;
  showInst: boolean;
  defReadOnly: boolean;
  inheritedSubmissionProvider: string | null;
  inheritedSubmissionEnvVars: Record<string, string>;
  linkedPersonaEnvVars: Record<string, string> | undefined;
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
        const namePool = parsePersonaNamePoolText(s.namePoolText);
        const normalizedModel = (s.model || null) as string | null;
        const normalizedProvider = s.showInst
          ? (s.inheritedSubmissionProvider ?? null)
          : s.provider.trim() || null;

        const next = {
          ...seed,
          displayName: s.displayName.trim(),
          avatarUrl: s.avatarUrl.trim(),
          systemPrompt: s.systemPrompt.trim(),
          respondTo: s.respondTo as typeof seed.respondTo,
          respondToAllowlist: s.respondToAllowlist,
          runtime:
            s.selectedRuntimeId === "custom" ? undefined : s.selectedRuntimeId,
          model: normalizedModel,
          provider: normalizedProvider,
          envVars: s.showInst
            ? (s.linkedPersonaEnvVars ?? def?.envVars ?? {})
            : s.envVars,
          namePool,
          instanceName: s.instanceName.trim() || undefined,
          instanceEnvVars: s.showInst
            ? (s.inheritedSubmissionEnvVars as Record<string, string>)
            : undefined,
          parallelism:
            s.parsedParallelism > 0
              ? s.parsedParallelism
              : (seed.parallelism ?? null),
          autoRestartOnConfigChange: s.showInst
            ? s.autoRestartOnConfigChange
            : undefined,
          startOnAppLaunch: seed.startOnAppLaunch,
        };

        const {
          personaInput,
          agentInput: base,
          policySets,
        } = emitAgentFormDiff(seed, next, s.ctx);

        // Merge harness-pin fields (I-fields not tracked in AgentFormModel).
        let agentInput = base;
        if (s.showInst && inst) {
          const resolvedArgs = s.agentArgs
            .split(",")
            .map((a) => a.trim())
            .filter(Boolean);
          const effectiveRuntime = s.runtimes.find(
            (r) => r.id === (s.selectedRuntimeId || "custom"),
          );
          let commandUpdate: string | undefined;
          if (s.inheritHarness) {
            commandUpdate = inst.agentCommandOverride != null ? "" : undefined;
          } else {
            const pinned = (effectiveRuntime?.command ?? s.agentCommand).trim();
            if (
              pinned !== inst.agentCommand ||
              (inst.agentCommandOverride == null && pinned.length > 0)
            ) {
              commandUpdate = pinned;
            }
          }
          const argsChanged =
            !s.inheritHarness &&
            resolvedArgs.join(",") !== inst.agentArgs.join(",");
          if (commandUpdate !== undefined || argsChanged) {
            agentInput = agentInput ?? { pubkey: inst.pubkey };
            if (commandUpdate !== undefined) {
              agentInput.agentCommand = commandUpdate;
              agentInput.harnessOverride =
                commandUpdate.length > 0 ? !s.inheritHarness : undefined;
            }
            if (argsChanged) agentInput.agentArgs = resolvedArgs;
          }
          if (s.acpCommand.trim() !== inst.acpCommand) {
            agentInput = agentInput ?? { pubkey: inst.pubkey };
            agentInput.acpCommand = s.acpCommand.trim();
          }
        }

        const refetchStores = async () => {
          await Promise.all([
            queryClient.invalidateQueries({ queryKey: personasQueryKey }),
            queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
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
