import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
import type { CreatePersonaInput } from "@/shared/api/types";

export function buildBatchImportPersonaInput(
  persona: ParsedPersonaPreview,
): CreatePersonaInput {
  return {
    displayName: persona.displayName,
    avatarUrl: persona.avatarDataUrl ?? persona.avatarRef ?? undefined,
    systemPrompt: persona.systemPrompt,
    runtime: persona.runtime ?? undefined,
    model: persona.model ?? undefined,
    provider: persona.provider ?? undefined,
    namePool: persona.namePool.length > 0 ? persona.namePool : undefined,
  };
}
