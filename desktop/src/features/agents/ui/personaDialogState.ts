import type { ParsePersonaFilesResult } from "@/shared/api/tauriPersonas";
import type {
  AgentPersona,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type PersonaDialogState = {
  description: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput;
  submitLabel: string;
  title: string;
};

type ParsedPersonaDraft = ParsePersonaFilesResult["personas"][number];

/**
 * Whether the persona dialog's save action should be enabled.
 *
 * A display name is the only required field. The system prompt is optional:
 * core memory is auto-injected at runtime, so a persona need not carry its
 * own prompt. `isPending` blocks double-submits while a save is in flight.
 */
export function canSubmitPersonaDialog(args: {
  displayName: string;
  isPending: boolean;
}): boolean {
  return args.displayName.trim().length > 0 && !args.isPending;
}

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
      avatarUrl: persona.avatarDataUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
    },
  };
}
