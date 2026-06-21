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
          `${result.agent.name} was updated, but profile sync failed: ${result.profileSyncError}`,
        );
      }
      toast.success(`Updated ${input.displayName}.`);
    } else {
      const persona = await createPersona(input);
      try {
        const created = await createManagedAgentForPersona(persona);
        if (created.spawnError) {
          toast.error(
            `${persona.displayName} was created, but it did not start: ${created.spawnError}`,
          );
        } else {
          toast.success(`Created and started ${created.agent.name}.`);
        }
        if (created.profileSyncError) {
          toast.warning(
            `${created.agent.name} was created, but profile sync failed: ${created.profileSyncError}`,
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
