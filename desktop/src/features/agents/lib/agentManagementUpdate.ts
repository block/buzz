import type { BackendIntent } from "./instanceInputForDefinition";
import { personaManagedAgentUpdate } from "@/features/profile/ui/UserProfilePanelUtils";
import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  ManagedAgent,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { isManagedAgentActive } from "./managedAgentControlActions";

export function validateAgentManagementBackendEdit({
  backendIntent,
  managedAgent,
  nextName,
}: {
  backendIntent: BackendIntent | null;
  managedAgent: ManagedAgent | undefined;
  nextName: string;
}): string | null {
  if (!backendIntent) return null;
  if (!managedAgent) {
    return "This agent does not have one unique instance to migrate.";
  }
  if (managedAgent.backend.type !== "local") {
    return "Provider-backed agents cannot change run location yet.";
  }
  if (isManagedAgentActive(managedAgent)) {
    return "Stop this agent before changing where it runs.";
  }
  if (nextName.trim() !== managedAgent.name) {
    return "Keep the current agent name during migration; rename it in a separate review.";
  }
  return null;
}

/**
 * Build the one instance update that follows an owner-reviewed definition edit.
 * Identity/runtime synchronization and a stopped local→provider migration must
 * land together so one review cannot silently discard either half.
 */
export function agentManagementInstanceUpdate({
  backendIntent,
  managedAgent,
  persona,
  previousPersona,
  runtimes,
}: {
  backendIntent: BackendIntent | null;
  managedAgent: ManagedAgent;
  persona: AgentPersona;
  previousPersona?: AgentPersona;
  runtimes: readonly AcpRuntimeCatalogEntry[];
}): UpdateManagedAgentInput | null {
  const synced = personaManagedAgentUpdate(managedAgent, persona, {
    previousPersona,
    runtimes,
  });
  if (!synced && !backendIntent) return null;

  return {
    ...(synced ?? { pubkey: managedAgent.pubkey }),
    ...(backendIntent
      ? {
          backend: {
            type: "provider" as const,
            id: backendIntent.id,
            config: backendIntent.config,
          },
        }
      : {}),
  };
}
