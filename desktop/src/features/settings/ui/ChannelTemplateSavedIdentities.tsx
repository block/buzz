import { usePersonasQuery, useTeamsQuery } from "@/features/agents/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { ChannelTemplate } from "@/shared/api/types";
import type { ReactNode } from "react";

import { CompactTeamAvatarStack } from "./ChannelTemplateAgentPicker";

export function ChannelTemplateSavedIdentities({
  template,
}: {
  template: ChannelTemplate;
}) {
  const personas = usePersonasQuery().data ?? [];
  const teams = useTeamsQuery().data ?? [];
  const personasById = new Map(
    personas.map((persona) => [persona.id, persona]),
  );
  const teamsById = new Map(teams.map((team) => [team.id, team]));

  return (
    <>
      {template.agents.personas.map(({ personaId }) => {
        const persona = personasById.get(personaId);
        const label = persona?.displayName ?? personaId;
        return (
          <SavedIdentity
            avatar={
              <ProfileAvatar
                avatarUrl={persona?.avatarUrl ?? null}
                className="h-8 w-8 text-xs shadow-none"
                label={label}
                testId={`channel-template-agent-avatar-${template.id}-${personaId}`}
              />
            }
            key={`agent-${personaId}`}
            kind="Agent"
            label={label}
            testId={`channel-template-detail-agent-${template.id}-${personaId}`}
          />
        );
      })}
      {template.agents.teams.map(({ teamId }) => {
        const team = teamsById.get(teamId);
        const label = team?.name ?? teamId;
        const teamPersonas =
          team?.personaIds.flatMap((personaId) => {
            const persona = personasById.get(personaId);
            return persona ? [persona] : [];
          }) ?? [];
        return (
          <SavedIdentity
            avatar={
              <CompactTeamAvatarStack
                memberCount={team?.personaIds.length ?? 0}
                personas={teamPersonas}
                teamName={label}
              />
            }
            key={`team-${teamId}`}
            kind="Agent team"
            label={label}
            testId={`channel-template-detail-team-${template.id}-${teamId}`}
          />
        );
      })}
    </>
  );
}

function SavedIdentity({
  avatar,
  kind,
  label,
  testId,
}: {
  avatar: ReactNode;
  kind: string;
  label: string;
  testId: string;
}) {
  return (
    <div
      className="flex min-w-0 items-center gap-3 rounded-lg border border-border/60 bg-background/60 px-3 py-2"
      data-testid={testId}
    >
      <dt className="sr-only">{kind}</dt>
      <dd className="flex min-w-0 items-center gap-3">
        {avatar}
        <span className="truncate text-sm font-medium" title={label}>
          {label}
        </span>
      </dd>
    </div>
  );
}
