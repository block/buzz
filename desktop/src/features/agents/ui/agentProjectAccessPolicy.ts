import type { Project } from "@/features/projects/hooks";
import type { ProjectConnection } from "@/shared/api/tauriProjectConnections";
import type { AgentToolRequirement } from "@/shared/api/types";
import type {
  AgentProjectAccessDraft,
  AgentProjectAccessReadiness,
} from "./AgentProjectAccessSection";

export function resolveAgentProjectAccessReadiness({
  connections,
  connectionsError,
  connectionsPending,
  draft,
  projectRequired = true,
  scopeAvailable,
  selectedProject,
  toolRequirements,
}: {
  connections: readonly ProjectConnection[];
  connectionsError: boolean;
  connectionsPending: boolean;
  draft: AgentProjectAccessDraft;
  projectRequired?: boolean;
  scopeAvailable: boolean;
  selectedProject: Project | null;
  toolRequirements: readonly AgentToolRequirement[];
}): AgentProjectAccessReadiness {
  if (!draft.projectId) {
    return projectRequired
      ? { ready: false, reason: "Choose a Project for this agent." }
      : { ready: true, reason: null };
  }
  if (!selectedProject) {
    return {
      ready: false,
      reason: "The selected Project is no longer available.",
    };
  }
  if (!selectedProject.projectChannelId) {
    return {
      ready: false,
      reason: "Add a discussion channel to this Project.",
    };
  }
  if (!scopeAvailable) {
    return {
      ready: false,
      reason: "Reconnect to the community before launching this agent.",
    };
  }

  const required = toolRequirements.filter(
    (requirement) => requirement.required,
  );
  if (connectionsPending && required.length > 0) {
    return { ready: false, reason: "Loading this Project's connections..." };
  }
  if (connectionsError && required.length > 0) {
    return {
      ready: false,
      reason: "Couldn't load this Project's connections. Try again.",
    };
  }

  const unresolved = required.find((requirement) => {
    const connection = connections.find(
      (candidate) => candidate.id === draft.connectionBindings[requirement.id],
    );
    return (
      connection?.health.status !== "ready" ||
      !connection.capabilityIds.includes(requirement.capability)
    );
  });

  return unresolved
    ? {
        ready: false,
        reason: `Choose a ready connection for ${unresolved.label || "each required tool"}.`,
      }
    : { ready: true, reason: null };
}
