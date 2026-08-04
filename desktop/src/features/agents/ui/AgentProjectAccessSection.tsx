import {
  AlertCircle,
  FolderGit2,
  Link2,
  LoaderCircle,
  ShieldCheck,
} from "lucide-react";
import * as React from "react";

import type { Project } from "@/features/projects/hooks";
import { useProjectConnectionsQuery } from "@/features/projects/projectConnectionHooks";
import {
  durableProjectAddress,
  toProjectConnectionScope,
} from "@/shared/api/agentProjectTypes";
import type { AgentToolRequirement } from "@/shared/api/types";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { resolveAgentProjectAccessReadiness } from "./agentProjectAccessPolicy";

const NO_CONNECTION = "__no_connection__";
const NO_PROJECT = "__no_project__";

export type AgentProjectAccessDraft = {
  /** Local UI identity only. Never sent to Tauri or persisted. */
  projectId: string;
  connectionBindings: Record<string, string>;
};

export type AgentProjectAccessReadiness = {
  ready: boolean;
  reason: string | null;
};

export const emptyAgentProjectAccessDraft: AgentProjectAccessDraft = {
  projectId: "",
  connectionBindings: {},
};

export function AgentProjectAccessSection({
  allowUnassigned = false,
  description = "Its conversations and connected tools stay with that Project.",
  disabled,
  draft,
  idPrefix = "agent",
  onDraftChange,
  onReadinessChange,
  operatorPubkey,
  projects,
  projectsLoading,
  relayUrl,
  toolRequirements,
}: {
  allowUnassigned?: boolean;
  description?: React.ReactNode;
  disabled: boolean;
  draft: AgentProjectAccessDraft;
  idPrefix?: string;
  onDraftChange: (draft: AgentProjectAccessDraft) => void;
  onReadinessChange: (readiness: AgentProjectAccessReadiness) => void;
  operatorPubkey: string | null;
  projects: readonly Project[];
  projectsLoading: boolean;
  relayUrl: string | null;
  toolRequirements: readonly AgentToolRequirement[];
}) {
  const selectedProject =
    projects.find((project) => project.id === draft.projectId) ?? null;
  const agentProjectScope = React.useMemo(
    () =>
      selectedProject?.projectChannelId && relayUrl && operatorPubkey
        ? {
            relayUrl,
            operatorPubkey,
            projectAddress: durableProjectAddress(selectedProject),
            channelId: selectedProject.projectChannelId,
          }
        : null,
    [operatorPubkey, relayUrl, selectedProject],
  );
  const projectConnectionScope = React.useMemo(
    () =>
      agentProjectScope ? toProjectConnectionScope(agentProjectScope) : null,
    [agentProjectScope],
  );
  const connectionsQuery = useProjectConnectionsQuery(projectConnectionScope, {
    enabled: toolRequirements.length > 0,
  });
  const connections = React.useMemo(
    () => connectionsQuery.data ?? [],
    [connectionsQuery.data],
  );

  React.useEffect(() => {
    onReadinessChange(
      resolveAgentProjectAccessReadiness({
        projectRequired: !allowUnassigned,
        connections,
        connectionsError: connectionsQuery.isError,
        connectionsPending: connectionsQuery.isPending,
        draft,
        scopeAvailable: Boolean(agentProjectScope),
        selectedProject,
        toolRequirements,
      }),
    );
  }, [
    agentProjectScope,
    allowUnassigned,
    connections,
    connectionsQuery.isError,
    connectionsQuery.isPending,
    draft,
    onReadinessChange,
    selectedProject,
    toolRequirements,
  ]);

  const projectOptions = [
    ...(allowUnassigned ? [{ label: "No Project", value: NO_PROJECT }] : []),
    ...projects.map((project) => ({
      disabled: !project.projectChannelId,
      label: project.projectChannelId
        ? project.name
        : `${project.name} (add a discussion channel first)`,
      value: project.id,
    })),
  ];

  function setBinding(requirementId: string, connectionId: string) {
    const nextBindings = { ...draft.connectionBindings };
    if (connectionId === NO_CONNECTION) {
      delete nextBindings[requirementId];
    } else {
      nextBindings[requirementId] = connectionId;
    }
    onDraftChange({ ...draft, connectionBindings: nextBindings });
  }

  return (
    <section className="space-y-3" data-testid="agent-project-access-section">
      <div className="flex items-center gap-2">
        <FolderGit2 className="h-4 w-4 text-muted-foreground" />
        <h3 className="text-base font-semibold text-foreground">
          Project access
        </h3>
      </div>
      <p className="text-xs text-muted-foreground">{description}</p>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor={`${idPrefix}-project`}
        >
          Project
        </label>
        <PersonaDropdownField
          disabled={disabled || projectsLoading}
          id={`${idPrefix}-project`}
          onValueChange={(projectId) =>
            onDraftChange({
              projectId: projectId === NO_PROJECT ? "" : projectId,
              connectionBindings: {},
            })
          }
          options={projectOptions}
          placeholder={
            projectsLoading
              ? "Loading Projects..."
              : projectOptions.length === 0
                ? "No Projects available"
                : "Choose a Project"
          }
          value={draft.projectId || (allowUnassigned ? NO_PROJECT : "")}
        />
      </div>

      {selectedProject && toolRequirements.length > 0 ? (
        <div className="space-y-3 border-border/60 border-t pt-3">
          <div className="flex items-center gap-2">
            <Link2 className="h-4 w-4 text-muted-foreground" />
            <h4 className="text-sm font-medium text-foreground">
              Tool connections
            </h4>
          </div>

          <div className="flex items-start gap-2 rounded-xl bg-muted/40 p-3 text-xs text-muted-foreground">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" />
            <p>
              A connection gives this agent access to every tool exposed by that
              MCP server. Buzz also checks that it provides the capability
              requested below.
            </p>
          </div>

          {connectionsQuery.isPending ? (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <LoaderCircle className="h-4 w-4 animate-spin" />
              Loading connections...
            </div>
          ) : connectionsQuery.isError ? (
            <div className="flex items-start gap-2 text-xs text-destructive">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>Couldn't load connections. Try again.</span>
            </div>
          ) : (
            toolRequirements.map((requirement) => {
              const compatible = connections.filter((connection) =>
                connection.capabilityIds.includes(requirement.capability),
              );
              const options = [
                ...(!requirement.required
                  ? [{ label: "No connection", value: NO_CONNECTION }]
                  : []),
                ...compatible.map((connection) => ({
                  disabled: connection.health.status !== "ready",
                  label:
                    connection.health.status === "ready"
                      ? connection.name
                      : `${connection.name} (${connection.health.status.replaceAll("_", " ")})`,
                  value: connection.id,
                })),
              ];
              return (
                <div className="space-y-1.5" key={requirement.id}>
                  <label
                    className="text-xs font-medium text-foreground"
                    htmlFor={`${idPrefix}-tool-binding-${requirement.id}`}
                  >
                    {requirement.label || "Unnamed tool"}
                    {!requirement.required ? (
                      <span className="ml-1 font-normal text-muted-foreground">
                        Optional
                      </span>
                    ) : null}
                  </label>
                  <PersonaDropdownField
                    disabled={disabled}
                    id={`${idPrefix}-tool-binding-${requirement.id}`}
                    onValueChange={(connectionId) =>
                      setBinding(requirement.id, connectionId)
                    }
                    options={options}
                    placeholder={
                      compatible.length > 0
                        ? "Choose a connection"
                        : "No compatible connection"
                    }
                    value={
                      draft.connectionBindings[requirement.id] ??
                      (requirement.required ? "" : NO_CONNECTION)
                    }
                  />
                  {compatible.length === 0 ? (
                    <p className="text-xs text-warning">
                      Open {selectedProject.name} Connections to add and test
                      one.
                    </p>
                  ) : null}
                </div>
              );
            })
          )}
        </div>
      ) : null}
    </section>
  );
}
