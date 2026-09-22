import { toast } from "sonner";

import { personaManagedAgentUpdate } from "@/features/profile/ui/UserProfilePanelUtils";
import { i18n } from "@/i18n";
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

  const runtimeLabel =
    runtime?.label ?? i18n.t("profile.persona.this-provider");
  return i18n.t("profile.persona.runtime-unavailable", { label: runtimeLabel });
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
          i18n.t("profile.persona.sync-failed-updated", {
            name: result.agent.name,
            error: result.profileSyncError,
          }),
        );
      }
      toast.success(
        i18n.t("profile.persona.updated", { name: input.displayName }),
      );
    } else {
      const persona = await createPersona(input);
      try {
        const created = await createManagedAgentForPersona(persona);
        if (created.spawnError) {
          toast.error(
            i18n.t("profile.persona.created-not-started", {
              name: persona.displayName,
              error: created.spawnError,
            }),
          );
        } else {
          toast.success(
            i18n.t("profile.persona.created-and-started", {
              name: created.agent.name,
            }),
          );
        }
        if (created.profileSyncError) {
          toast.warning(
            i18n.t("profile.persona.created-sync-failed", {
              name: created.agent.name,
              error: created.profileSyncError,
            }),
          );
        }
      } catch (error) {
        toast.error(
          error instanceof Error
            ? i18n.t("profile.persona.created-instance-failed", {
                name: persona.displayName,
                error: error.message,
              })
            : i18n.t("profile.persona.created-instance-failed-plain", {
                name: persona.displayName,
              }),
        );
      }
    }

    onDone();
  } catch (error) {
    toast.error(
      error instanceof Error
        ? error.message
        : i18n.t("profile.persona.save-failed"),
    );
  }
}
