import type { AgentPersona, UpdatePersonaInput } from "@/shared/api/types";

/** True when the linked definition can accept description/instructions edits. */
export function isPersonaIdentityEditable(
  persona: AgentPersona | null | undefined,
): boolean {
  return persona != null && !persona.isBuiltIn && !persona.sourceTeam;
}

/**
 * Build an UpdatePersonaInput that preserves non-identity fields while
 * applying description / instructions / model / provider drafts. Returns null
 * when nothing changed (caller should skip the persona write).
 *
 * Model and provider are definition-owned for linked instances: spawn reads
 * them from the persona (`resolve_effective_config` → `resolve_linked`), and
 * `update_managed_agent` refuses instance model/provider writes when
 * `persona_id` is set. Persist those edits here so Save + restart keep them.
 */
export function buildPersonaIdentityUpdate(options: {
  descriptionDraft: string;
  persona: AgentPersona;
  systemPromptDraft: string;
  /** When provided, persist a model change onto the definition. */
  modelDraft?: string | null;
  /** When provided, persist a provider change onto the definition. */
  providerDraft?: string | null;
}): UpdatePersonaInput | null {
  const { descriptionDraft, persona, systemPromptDraft } = options;
  const descriptionChanged = (persona.description ?? "") !== descriptionDraft;
  const instructionsChanged = persona.systemPrompt !== systemPromptDraft;

  const nextModel =
    options.modelDraft !== undefined
      ? options.modelDraft?.trim() || null
      : (persona.model ?? null);
  const nextProvider =
    options.providerDraft !== undefined
      ? options.providerDraft?.trim() || null
      : (persona.provider ?? null);
  const modelChanged =
    options.modelDraft !== undefined &&
    nextModel !== (persona.model?.trim() || null);
  const providerChanged =
    options.providerDraft !== undefined &&
    nextProvider !== (persona.provider?.trim() || null);

  if (
    !descriptionChanged &&
    !instructionsChanged &&
    !modelChanged &&
    !providerChanged
  ) {
    return null;
  }

  return {
    id: persona.id,
    displayName: persona.displayName,
    avatarUrl: persona.avatarUrl ?? undefined,
    description: descriptionDraft,
    systemPrompt: systemPromptDraft,
    runtime: persona.runtime ?? undefined,
    model: nextModel ?? undefined,
    provider: nextProvider ?? undefined,
    namePool: persona.namePool,
    envVars: persona.envVars,
    behavior:
      persona.respondTo != null
        ? {
            respondTo: persona.respondTo,
            respondToAllowlist: persona.respondToAllowlist,
            parallelism: persona.parallelism ?? undefined,
          }
        : undefined,
  };
}

/**
 * Resolve the managed-agent `systemPrompt` field for an instance Save.
 * - After a definition identity write, mirror the new instructions onto the
 *   instance so profile/Info resolve immediately.
 * - Linked instances otherwise omit the field (definition is authoritative).
 * - Unlinked agents edit the instance field directly.
 */
export function resolveInstanceSystemPromptUpdate(options: {
  agentSystemPrompt: string | null;
  linkedPersona: AgentPersona | null;
  syncedSystemPrompt: string | null | undefined;
  systemPromptDraft: string;
}): string | null | undefined {
  const {
    agentSystemPrompt,
    linkedPersona,
    syncedSystemPrompt,
    systemPromptDraft,
  } = options;
  if (syncedSystemPrompt !== undefined) {
    return syncedSystemPrompt !== agentSystemPrompt
      ? syncedSystemPrompt
      : undefined;
  }
  if (linkedPersona != null) {
    return undefined;
  }
  const next = systemPromptDraft.trim() || null;
  return next !== agentSystemPrompt ? next : undefined;
}

/** Persist identity / model / provider drafts when editable and dirty; returns mirrored systemPrompt. */
export async function persistPersonaIdentityUpdate(options: {
  descriptionDraft: string;
  persona: AgentPersona;
  systemPromptDraft: string;
  modelDraft?: string | null;
  providerDraft?: string | null;
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
}): Promise<string | null | undefined> {
  const input = buildPersonaIdentityUpdate(options);
  if (!input) return undefined;
  await options.updatePersona(input);
  return options.systemPromptDraft.trim() || null;
}

/**
 * Persist only model (and optional provider) onto an editable linked persona.
 * Used by ModelPicker when `update_managed_agent` cannot write instance model
 * for persona-linked agents. Returns whether a write ran.
 */
export async function persistPersonaModelUpdate(options: {
  persona: AgentPersona;
  model: string | null;
  provider?: string | null;
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
}): Promise<boolean> {
  if (!isPersonaIdentityEditable(options.persona)) return false;
  const input = buildPersonaIdentityUpdate({
    descriptionDraft: options.persona.description ?? "",
    persona: options.persona,
    systemPromptDraft: options.persona.systemPrompt,
    modelDraft: options.model,
    providerDraft:
      options.provider !== undefined
        ? options.provider
        : options.persona.provider,
  });
  if (!input) return false;
  await options.updatePersona(input);
  return true;
}
