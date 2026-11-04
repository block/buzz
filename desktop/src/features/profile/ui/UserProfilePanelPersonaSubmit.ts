// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

import { personaManagedAgentUpdate } from "@/features/profile/ui/UserProfilePanelUtils";
import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreateManagedAgentResponse,
  CreatePersonaInput,
  ManagedAgent,
  UpdateManagedAgentInput,
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
  updateManagedAgent: (
    input: UpdateManagedAgentInput,
  ) => Promise<{ agent: ManagedAgent; profileSyncError: string | null }>;
  updatePersona: (input: UpdatePersonaInput) => Promise<AgentPersona>;
};

type ValidateLinkedAgentRuntimeEditOptions = {
  input: UpdatePersonaInput;
  managedAgent: ManagedAgent | undefined;
  previousPersona?: AgentPersona;
  runtimes?: readonly AcpRuntimeCatalogEntry[];
};

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

  const runtimeLabel = runtime?.label ?? uiT("This provider");
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
  updateManagedAgent,
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

      const persona = await updatePersona(input);
      const agentUpdate = managedAgent
        ? personaManagedAgentUpdate(managedAgent, persona, {
            previousPersona,
            runtimes,
          })
        : null;
      const result = agentUpdate ? await updateManagedAgent(agentUpdate) : null;
      if (result?.profileSyncError) {
        toast.warning(
          uiT("{0} was updated, but profile sync failed: {1}", {
            p0: result.agent.name,
            p1: result.profileSyncError,
          }),
        );
      }
      toast.success(uiT("Updated {0}.", { p0: input.displayName }));
    } else {
      const persona = await createPersona(input);
      try {
        const created = await createManagedAgentForPersona(persona);
        if (created.spawnError) {
          toast.error(
            uiT("{0} was created, but it did not start: {1}", {
              p0: persona.displayName,
              p1: created.spawnError,
            }),
          );
        } else {
          toast.success(
            uiT("Created and started {0}.", { p0: created.agent.name }),
          );
        }
        if (created.profileSyncError) {
          toast.warning(
            uiT("{0} was created, but profile sync failed: {1}", {
              p0: created.agent.name,
              p1: created.profileSyncError,
            }),
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
      error instanceof Error ? error.message : uiT("Failed to save agent."),
    );
  }
}
