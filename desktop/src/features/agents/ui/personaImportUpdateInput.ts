import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
import type { AgentPersona, UpdatePersonaInput } from "@/shared/api/types";

type BuildPersonaImportUpdateInputArgs = {
  existing: AgentPersona;
  preview: ParsedPersonaPreview;
  selectedFields: Iterable<string>;
};

export function buildPersonaImportUpdateInput({
  existing,
  preview,
  selectedFields,
}: BuildPersonaImportUpdateInputArgs): UpdatePersonaInput {
  const selectedFieldSet = new Set(selectedFields);

  return {
    id: existing.id,
    displayName: selectedFieldSet.has("displayName")
      ? preview.displayName
      : existing.displayName,
    systemPrompt: selectedFieldSet.has("systemPrompt")
      ? preview.systemPrompt
      : existing.systemPrompt,
    avatarUrl: selectedFieldSet.has("avatarUrl")
      ? (preview.avatarDataUrl ?? preview.avatarRef ?? undefined)
      : (existing.avatarUrl ?? undefined),
    runtime: selectedFieldSet.has("runtime")
      ? (preview.runtime ?? undefined)
      : (existing.runtime ?? undefined),
    model: selectedFieldSet.has("model")
      ? (preview.model ?? undefined)
      : (existing.model ?? undefined),
    provider: selectedFieldSet.has("provider")
      ? (preview.provider ?? undefined)
      : (existing.provider ?? undefined),
    namePool: selectedFieldSet.has("namePool")
      ? preview.namePool.length > 0
        ? preview.namePool
        : undefined
      : existing.namePool.length > 0
        ? [...existing.namePool]
        : undefined,
  };
}
