import * as React from "react";

import type { CreateChannelManagedAgentInput } from "@/features/agents/channelAgents";
import {
  useAvailableAcpRuntimes,
  usePersonasQuery,
  useTeamsQuery,
} from "@/features/agents/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import {
  getUsableTeams,
  resolveTeamPersonas,
} from "@/features/agents/lib/teamPersonas";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import {
  PROJECT_HOME_CHANNEL_TEMPLATE,
  PROJECT_HOME_TEMPLATE_ID,
} from "@/features/projects/lib/projectHomeTemplate";
import type { ProjectListingVisibility } from "@/features/projects/projectCreation";
import type {
  AcpRuntime,
  AgentPersona,
  AgentTeam,
  ChannelTemplate,
  ChannelVisibility,
} from "@/shared/api/types";

/** Expand the selected team and persona into deduplicated channel agents. */
export function buildCreateProjectAgents(input: {
  agentPersonaId: string;
  personas: AgentPersona[];
  runtimes: AcpRuntime[];
  teamId: string;
  teams: AgentTeam[];
}): CreateChannelManagedAgentInput[] {
  const defaultRuntime = input.runtimes[0] ?? null;
  const agents: CreateChannelManagedAgentInput[] = [];
  const seenPersonaIds = new Set<string>();
  const addPersona = (persona: AgentPersona, selectedTeamId?: string) => {
    if (seenPersonaIds.has(persona.id)) return;
    const resolved = resolvePersonaRuntime(
      persona.runtime,
      input.runtimes,
      defaultRuntime,
      false,
    );
    if (!resolved.runtime) {
      throw new Error(
        resolved.warnings[0] ??
          "No agent runtimes are available. Install a runtime to add agents.",
      );
    }
    seenPersonaIds.add(persona.id);
    agents.push({
      runtime: resolved.runtime,
      name: persona.displayName,
      personaId: persona.id,
      teamId: selectedTeamId,
      harnessOverride: false,
      systemPrompt: persona.systemPrompt,
      avatarUrl: persona.avatarUrl ?? undefined,
      model: persona.model ?? undefined,
      role: "bot",
      backend: { type: "local" },
    });
  };
  if (input.teamId) {
    const team = input.teams.find((entry) => entry.id === input.teamId);
    if (!team) throw new Error("Choose a team that still exists.");
    const resolution = resolveTeamPersonas(team, input.personas);
    for (const persona of resolution.resolvedPersonas) {
      addPersona(persona, team.id);
    }
  }
  if (input.agentPersonaId) {
    const persona = input.personas.find(
      (entry) => entry.id === input.agentPersonaId,
    );
    if (!persona) throw new Error("Choose an agent that still exists.");
    addPersona(persona);
  }
  return agents;
}

export function useCreateProjectFormSettings(
  active: boolean,
  onTemplateDescriptionChange?: (description: string) => void,
) {
  const personasQuery = usePersonasQuery({ enabled: active });
  const runtimesQuery = useAvailableAcpRuntimes({ enabled: active });
  const teamsQuery = useTeamsQuery();
  const templatesQuery = useChannelTemplatesQuery();
  const [channelVisibility, setChannelVisibility] =
    React.useState<ChannelVisibility>("open");
  const [projectVisibility, setProjectVisibility] =
    React.useState<ProjectListingVisibility>("listed");
  const [agentPersonaId, setAgentPersonaId] = React.useState("");
  const [teamId, setTeamId] = React.useState("");
  const [templateId, setTemplateId] = React.useState(PROJECT_HOME_TEMPLATE_ID);

  const personas = React.useMemo(
    () => getActivePersonas(personasQuery.data ?? []),
    [personasQuery.data],
  );
  const teams = React.useMemo(
    () => getUsableTeams(teamsQuery.data ?? [], personas),
    [personas, teamsQuery.data],
  );
  const templates = React.useMemo(
    () => [
      PROJECT_HOME_CHANNEL_TEMPLATE,
      ...(templatesQuery.data ?? []).filter(
        (template) => template.id !== PROJECT_HOME_TEMPLATE_ID,
      ),
    ],
    [templatesQuery.data],
  );

  React.useEffect(() => {
    if (!active) return;
    setChannelVisibility("open");
    setProjectVisibility("listed");
    setAgentPersonaId("");
    setTeamId("");
    setTemplateId(PROJECT_HOME_TEMPLATE_ID);
  }, [active]);

  React.useEffect(() => {
    if (
      agentPersonaId &&
      !personas.some((persona) => persona.id === agentPersonaId)
    ) {
      setAgentPersonaId("");
    }
  }, [agentPersonaId, personas]);
  React.useEffect(() => {
    if (teamId && !teams.some((team) => team.id === teamId)) {
      setTeamId("");
    }
  }, [teamId, teams]);
  React.useEffect(() => {
    if (
      templateId &&
      !templates.some((template) => template.id === templateId)
    ) {
      setTemplateId("");
    }
  }, [templateId, templates]);

  const buildAgents = React.useCallback(
    () =>
      buildCreateProjectAgents({
        agentPersonaId,
        personas,
        runtimes: runtimesQuery.data,
        teamId,
        teams,
      }),
    [agentPersonaId, personas, runtimesQuery.data, teamId, teams],
  );

  const applyTemplate = React.useCallback(
    (template: ChannelTemplate) => {
      setTemplateId(template.id);
      setChannelVisibility(template.visibility);
      if (template.id !== PROJECT_HOME_TEMPLATE_ID) {
        onTemplateDescriptionChange?.(template.description ?? "");
      }
    },
    [onTemplateDescriptionChange],
  );
  const handleTemplateChange = React.useCallback(
    (nextTemplateId: string) => {
      if (!nextTemplateId) {
        setTemplateId("");
        setChannelVisibility("open");
        onTemplateDescriptionChange?.("");
        return;
      }
      const template = templates.find((entry) => entry.id === nextTemplateId);
      if (template) applyTemplate(template);
    },
    [applyTemplate, onTemplateDescriptionChange, templates],
  );

  return {
    agentPersonaId,
    buildAgents,
    channelVisibility,
    handleTemplateCreated: applyTemplate,
    handleTemplateChange,
    personas,
    projectVisibility,
    runtimesAvailable: runtimesQuery.data.length > 0,
    setAgentPersonaId,
    setChannelVisibility,
    setProjectVisibility,
    setTeamId,
    teamId,
    teams,
    templateId,
    templates,
  };
}

export type CreateProjectFormSettingsState = ReturnType<
  typeof useCreateProjectFormSettings
>;
