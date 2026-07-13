import type {
  AgentPersona,
  CreatePersonaInput,
  PersonaBehaviorInput,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type PersonaDialogState = {
  description: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput;
  submitLabel: string;
  title: string;
};

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
    description: "Create an agent and start it immediately.",
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
      ...behaviorEntry(persona),
    },
  };
}

/**
 * Seed a dialog behavior group from a stored persona. A quad-less persona
 * yields no `behavior` key at all, keeping initialValues byte-identical to
 * the pre-quad shape (spread-in entry, matching the namePool import pattern).
 */
function behaviorEntry(
  persona: AgentPersona,
): { behavior: PersonaBehaviorInput } | Record<string, never> {
  if (persona.respondTo == null && persona.parallelism == null) {
    return {};
  }
  return {
    behavior: {
      respondTo: persona.respondTo ?? undefined,
      respondToAllowlist:
        persona.respondTo === "allowlist"
          ? persona.respondToAllowlist
          : undefined,
      parallelism: persona.parallelism ?? undefined,
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
      ...behaviorEntry(persona),
    },
  };
}
