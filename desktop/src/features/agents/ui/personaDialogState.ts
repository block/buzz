import type { ParsePersonaFilesResult } from "@/shared/api/tauriPersonas";
import type {
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

function isPersistableAvatarUrl(value: string): boolean {
  return /^(?:https?:|data:image\/|blob:)/i.test(value);
}

/**
 * Picks an avatar URL to seed the import dialog with. Only persistable
 * http(s)/data/blob URLs survive; anything else (e.g. a bare filename or an
 * unrenderable symbolic ref) is dropped so the dialog starts blank rather than
 * carrying a value the app can't display.
 */
function resolveImportedPersonaAvatarUrl({
  avatarDataUrl,
  avatarRef,
}: {
  avatarDataUrl?: string | null;
  avatarRef?: string | null;
}): string | null {
  for (const candidate of [avatarRef, avatarDataUrl]) {
    const trimmed = candidate?.trim();
    if (trimmed && isPersistableAvatarUrl(trimmed)) {
      return trimmed;
    }
  }
  return null;
}

export type PersonaDialogState = {
  description: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput;
  submitLabel: string;
  title: string;
};

type ParsedPersonaDraft = ParsePersonaFilesResult["personas"][number];

export function createPersonaDialogState(): PersonaDialogState {
  return {
    title: "Create persona",
    description:
      "Save a reusable role, prompt, and optional avatar for future agent deployments.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: "",
      avatarUrl: "",
      systemPrompt: "",
      runtime: undefined,
      model: undefined,
    },
  };
}

export function duplicatePersonaDialogState(
  persona: AgentPersona,
): PersonaDialogState {
  return {
    title: `Duplicate ${persona.displayName}`,
    description:
      "Create a new persona by copying this template and adjusting it as needed.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: `${persona.displayName} copy`,
      avatarUrl: persona.avatarUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Carry envVars and namePool into the duplicate. Without this, a
      // duplicated persona that relies on an API key in env_vars would
      // silently fail at spawn until the user re-entered every credential.
      // The user sees the inherited values in the dialog and can clear
      // them if they want a blank template.
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
    },
  };
}

export function editPersonaDialogState(
  persona: AgentPersona,
): PersonaDialogState {
  return {
    title: "Edit persona",
    description: "",
    submitLabel: "Save changes",
    initialValues: {
      id: persona.id,
      displayName: persona.displayName,
      avatarUrl: persona.avatarUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Seed both namePool and envVars from the loaded persona so editing
      // unrelated fields doesn't submit an empty value that wipes them.
      // (Persona update treats Some(empty) as "clear all" intentionally;
      // the dialog must therefore round-trip the existing values.)
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
    },
  };
}

export function importPersonaDialogState(
  persona: ParsedPersonaDraft,
): PersonaDialogState {
  return {
    title: `Import ${persona.displayName}`,
    description: "Review and save this imported persona.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: persona.displayName,
      avatarUrl: resolveImportedPersonaAvatarUrl(persona) ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
    },
  };
}
