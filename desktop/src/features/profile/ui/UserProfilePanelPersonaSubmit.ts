import { toast } from "sonner";

import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreateManagedAgentResponse,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";

type SubmitProfilePersonaDialogOptions = {
  createManagedAgentForPersona: (
    persona: AgentPersona,
  ) => Promise<CreateManagedAgentResponse>;
  createPersona: (input: CreatePersonaInput) => Promise<AgentPersona>;
  input: CreatePersonaInput | UpdatePersonaInput;
  managedAgent: ManagedAgent | undefined;
  onDone: () => void;
  previousPersona?: AgentPersona;
  runtimes?: readonly AcpRuntimeCatalogEntry[];
  updatePersona: (input: UpdatePersonaInput) => Promise<AgentPersona>;
};

type ValidateLinkedAgentRuntimeEditOptions = {
  input: UpdatePersonaInput;
  managedAgent: ManagedAgent | undefined;
  previousPersona?: AgentPersona;
  runtimes?: readonly AcpRuntimeCatalogEntry[];
};

export function profileCreateResultKind(
  created: CreateManagedAgentResponse,
): "error" | "pending" | "success" {
  if (created.spawnError) return "error";
  if (created.profileSyncError) return "pending";
  return "success";
}

function normalizeRuntimePreference(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

export function validateLinkedAgentRuntimeEdit({
  input,
  managedAgent,
  previousPersona,
  runtimes,
}: ValidateLinkedAgentRuntimeEditOptions): string | null {
  if (!managedAgent || !previousPersona) {
    return null;
  }

  const previousRuntime = normalizeRuntimePreference(previousPersona.runtime);
  const nextRuntime = normalizeRuntimePreference(input.runtime);
  if (previousRuntime === nextRuntime) {
    return null;
  }

  const runtime = runtimes?.find((candidate) => candidate.id === nextRuntime);
  if (runtime?.availability === "available" && runtime.command) {
    return null;
  }

  const runtimeLabel = runtime?.label ?? "This provider";
  return `${runtimeLabel} is not available. Install it before saving this linked agent.`;
}

export async function submitProfilePersonaDialog({
  createManagedAgentForPersona,
  createPersona,
  input,
  managedAgent,
  onDone,
  previousPersona,
  runtimes,
  updatePersona,
}: SubmitProfilePersonaDialogOptions) {
  try {
    if ("id" in input) {
      const runtimeEditError = validateLinkedAgentRuntimeEdit({
        input,
        managedAgent,
        previousPersona,
        runtimes,
      });
      if (runtimeEditError) {
        toast.error(runtimeEditError);
        return;
      }

      await updatePersona(input);
      toast.success(`Updated ${input.displayName}.`);
    } else {
      const persona = await createPersona(input);
      try {
        const created = await createManagedAgentForPersona(persona);
        const resultKind = profileCreateResultKind(created);
        if (resultKind === "error") {
          toast.error(
            `${persona.displayName} was created, but it did not start: ${created.spawnError}`,
          );
        } else if (resultKind === "pending") {
          toast.warning(
            `${created.agent.name} reused its existing identity, but policy sync is still pending: ${created.profileSyncError}`,
          );
        } else {
          toast.success(`Created and started ${created.agent.name}.`);
        }
        if (created.spawnError && created.profileSyncError) {
          toast.warning(
            `${created.agent.name} policy sync is still pending: ${created.profileSyncError}`,
          );
        }
      } catch (error) {
        toast.error(
          error instanceof Error
            ? `${persona.displayName} was created, but the agent instance could not be created: ${error.message}`
            : `${persona.displayName} was created, but the agent instance could not be created.`,
        );
      }
    }

    onDone();
  } catch (error) {
    toast.error(
      error instanceof Error ? error.message : "Failed to save agent.",
    );
  }
}
