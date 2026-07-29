import * as React from "react";

import { useUpdatePersonaMutation } from "@/features/agents/hooks";
import type { AgentPersona, UpdatePersonaInput } from "@/shared/api/types";
import { editPersonaDialogState } from "./personaDialogState";

/**
 * What saving a model change in the INSTANCE dialog must do when the record is
 * provider-backed and linked to a definition.
 *
 * The instance dialog defers model/provider/systemPrompt to the definition,
 * because for a linked instance the record's own bytes are never read: both
 * spawn and provider deploy resolve those three through
 * `resolve_effective_config`, which for a linked record reads the DEFINITION
 * and falls back to global — the record's `model` column is dead weight. So an
 * instance-dialog model edit written to the record is a guaranteed silent
 * no-op, which is why the submit path omits it.
 *
 * That deferral assumes the definition dialog is the surface that owns the
 * write — true for a LOCAL record, whose `!model` draft routes to
 * `definition-edit` and whose definition the owner can open from the agent
 * card. It is not true for a provider-backed record: the definition carries no
 * backend, harness pin or host command, so `agentManagementUpdateTarget` routes
 * it here instead, and this dialog becomes the only surface the owner has. A
 * model change reviewed here therefore has to reach the definition from here,
 * or it reaches nothing.
 *
 * The write itself goes through the ordinary `update_persona` path — the same
 * mutation `AgentDefinitionDialog` uses. Nothing new is invented for this lane;
 * only the model travels, and provider/systemPrompt deferral is unchanged.
 *
 * The record's own `model` column is deliberately NOT refreshed, and never
 * will be: every live `apply_persona_snapshot` caller is gated to
 * `BackendKind::Local`. It does not need to be. `resolve_effective_config`
 * answers from the definition for a linked instance, and it is the single
 * resolver behind both the deploy payload and the summary this dialog reads
 * back — so the edit is visible everywhere it is consulted, while the stale
 * column stays inert.
 */
export type InstanceModelDefinitionWrite =
  | { kind: "none" }
  | { kind: "write"; input: UpdatePersonaInput }
  /**
   * The model changed but no honest write exists. Save is blocked rather than
   * accepted, because the alternative is the silent no-op this module exists
   * to remove.
   */
  | { kind: "blocked"; reason: InstanceModelDefinitionBlockReason };

export type InstanceModelDefinitionBlockReason =
  /** `personaId` names a definition this app has not resolved: still loading, or deleted (an orphaned instance). */
  | "unresolved-definition"
  /** The definition belongs to a team and is re-imported from its directory, so an in-app edit would not survive. */
  | "team-managed";

/** The message shown beside a blocked Save. One owner, so gate and copy agree. */
export function instanceModelDefinitionBlockMessage(
  reason: InstanceModelDefinitionBlockReason,
): string {
  return reason === "team-managed"
    ? "This agent's model is managed by its team and can't be changed here. Edit the team's agent directory instead."
    : "This agent's configuration isn't available yet, so the model can't be saved. Close and try again once it loads.";
}

/**
 * Resolve the definition write a provider-backed instance save implies.
 *
 * `originalModel` is the record's CURRENT effective model (the summary's
 * definition-or-global resolution), not the value the field was seeded with: a
 * `!model` draft seeds the field with the requested model precisely so the
 * owner reviews it, and comparing against the seed would read that review as
 * "unchanged" and drop it. Comparing against the effective model also keeps a
 * globally-inherited value from being silently pinned onto the definition when
 * the owner opens the dialog and saves without touching the field.
 *
 * A blank model is a real value — "let the harness pick its default" — so it
 * writes `undefined`, which `update_persona` normalizes to a cleared column.
 */
export function resolveInstanceModelDefinitionWrite(input: {
  /** True when the record's harness is pinned to a host binary (`providerRecordHarness` non-null). */
  isProviderRecord: boolean;
  /** The record's `personaId`, or null when the instance owns its own config. */
  personaId: string | null;
  /** The linked definition, or null when `personaId` did not resolve to one. */
  linkedPersona: AgentPersona | null;
  /** The dialog's current model field. */
  model: string;
  /** The record's effective model before this edit. */
  originalModel: string | null;
}): InstanceModelDefinitionWrite {
  // A local record's definition dialog is reachable and correct; leaving its
  // semantics untouched is deliberate, not an oversight.
  if (!input.isProviderRecord || input.personaId == null) {
    return { kind: "none" };
  }

  const next = input.model.trim();
  if (next === (input.originalModel ?? "").trim()) {
    return { kind: "none" };
  }

  if (input.linkedPersona == null) {
    return { kind: "blocked", reason: "unresolved-definition" };
  }
  if (input.linkedPersona.sourceTeam) {
    return { kind: "blocked", reason: "team-managed" };
  }

  // `update_persona` replaces displayName/avatarUrl/systemPrompt/runtime/
  // model/provider/namePool wholesale, so the input must round-trip the stored
  // definition. `editPersonaDialogState` is the existing owner of that
  // projection — the definition dialog is seeded from it for the same reason.
  return {
    kind: "write",
    input: {
      ...(editPersonaDialogState(input.linkedPersona)
        .initialValues as UpdatePersonaInput),
      model: next || undefined,
    },
  };
}

/**
 * The instance dialog's entire model-write seam: the decision, its own
 * mutation, the blocked-Save gate and message, and the in-flight/error state
 * the dialog renders.
 *
 * One owner rather than five call sites in `AgentInstanceEditDialog`, because
 * they must agree — a Save the gate allows must have a write to perform, the
 * message must describe the block the gate applied, and an error from this leg
 * must reach the same place the record update's does. Owning the mutation here
 * also keeps the dialog from having to know that a model edit is a persona
 * write at all; it awaits `perform` and reads `error`.
 */
export function useInstanceModelDefinitionWrite(input: {
  isProviderRecord: boolean;
  personaId: string | null;
  linkedPersona: AgentPersona | null;
  model: string;
  originalModel: string | null;
  /** Clears a stale error when the dialog reopens or switches agent. */
  resetKey: unknown;
}): {
  /** True when Save must be blocked: a model change with no honest destination. */
  blocked: boolean;
  /** Why Save is blocked, for display beside the Model field. `undefined` when it isn't. */
  blockedMessage: string | undefined;
  error: Error | null;
  isPending: boolean;
  /** Awaited unconditionally by the submit path; a no-op decision performs nothing. */
  perform: () => Promise<void>;
} {
  const { isProviderRecord, personaId, linkedPersona, model, originalModel } =
    input;
  const mutation = useUpdatePersonaMutation();
  const decision = React.useMemo(
    () =>
      resolveInstanceModelDefinitionWrite({
        isProviderRecord,
        personaId,
        linkedPersona,
        model,
        originalModel,
      }),
    [isProviderRecord, personaId, linkedPersona, model, originalModel],
  );

  const { reset } = mutation;
  // biome-ignore lint/correctness/useExhaustiveDependencies: resetKey is the caller's explicit "forget the last attempt" signal; reset is stable
  React.useEffect(() => {
    reset();
  }, [input.resetKey, reset]);

  return {
    blocked: decision.kind === "blocked",
    blockedMessage:
      decision.kind === "blocked"
        ? instanceModelDefinitionBlockMessage(decision.reason)
        : undefined,
    error: mutation.error instanceof Error ? mutation.error : null,
    isPending: mutation.isPending,
    perform: async () => {
      if (decision.kind !== "write") return;
      await mutation.mutateAsync(decision.input);
    },
  };
}
