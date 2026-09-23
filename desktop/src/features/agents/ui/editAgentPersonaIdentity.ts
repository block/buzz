import type { AgentPersona, UpdatePersonaInput } from "@/shared/api/types";

/** True when the linked definition can accept description/instructions edits. */
export function isPersonaIdentityEditable(
  persona: AgentPersona | null | undefined,
): boolean {
  return persona != null && !persona.isBuiltIn && !persona.sourceTeam;
}

/**
 * Build an UpdatePersonaInput that preserves non-identity fields while
 * applying description + instructions drafts. Returns null when nothing
 * changed (caller should skip the persona write).
 */
export function buildPersonaIdentityUpdate(options: {
  descriptionDraft: string;
  persona: AgentPersona;
  systemPromptDraft: string;
}): UpdatePersonaInput | null {
  const { descriptionDraft, persona, systemPromptDraft } = options;
  const descriptionChanged = (persona.description ?? "") !== descriptionDraft;
  const instructionsChanged = persona.systemPrompt !== systemPromptDraft;
  if (!descriptionChanged && !instructionsChanged) {
    return null;
  }

  return {
    id: persona.id,
    displayName: persona.displayName,
    avatarUrl: persona.avatarUrl ?? undefined,
    description: descriptionDraft,
    systemPrompt: systemPromptDraft,
    runtime: persona.runtime ?? undefined,
    model: persona.model ?? undefined,
    provider: persona.provider ?? undefined,
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

/** Persist identity drafts when editable and dirty; returns mirrored systemPrompt. */
export async function persistPersonaIdentityUpdate(options: {
  descriptionDraft: string;
  persona: AgentPersona;
  systemPromptDraft: string;
  updatePersona: (input: UpdatePersonaInput) => Promise<unknown>;
}): Promise<string | null | undefined> {
  const input = buildPersonaIdentityUpdate(options);
  if (!input) return undefined;
  await options.updatePersona(input);
  return options.systemPromptDraft.trim() || null;
}
