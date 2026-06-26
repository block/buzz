import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
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

type ImportedPersonaAvatarPreview = Pick<
  ParsedPersonaPreview,
  "avatarDataUrl" | "avatarRef"
>;

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

function isSafeImportedAvatarRef(
  ref: string | null | undefined,
): ref is string {
  const trimmed = ref?.trim();
  if (!trimmed) return false;

  try {
    const parsed = new URL(trimmed);
    return (
      parsed.protocol === "http:" ||
      parsed.protocol === "https:" ||
      parsed.protocol === "blob:" ||
      parsed.protocol === "ipfs:" ||
      (parsed.protocol === "data:" && trimmed.startsWith("data:image/"))
    );
  } catch {
    return false;
  }
}

export function importedAvatarUrl(persona: ImportedPersonaAvatarPreview) {
  if (persona.avatarDataUrl) return persona.avatarDataUrl;
  return isSafeImportedAvatarRef(persona.avatarRef) ? persona.avatarRef : "";
}

export function formatPersonaNamePoolText(namePool: string[] | undefined) {
  return namePool?.join(", ") ?? "";
}

export function parsePersonaNamePoolText(text: string): string[] {
  return text
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value.length > 0);
}

export function createPersonaDialogState(): PersonaDialogState {
  return {
    title: "Create agent",
    description:
      "Create an agent profile and start its managed agent instance.",
    submitLabel: "Create agent",
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
      "Create a new agent by copying this profile and adjusting it as needed.",
    submitLabel: "Create agent",
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
    title: "Edit agent",
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
  persona: ParsedPersonaPreview,
): PersonaDialogState {
  return {
    title: `Import ${persona.displayName}`,
    description: "Review and create this imported agent.",
    submitLabel: "Create agent",
    initialValues: {
      displayName: persona.displayName,
      avatarUrl: importedAvatarUrl(persona),
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      ...(persona.namePool.length > 0 ? { namePool: persona.namePool } : {}),
    },
  };
}
