import { Users } from "lucide-react";
import * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

const TEMPLATE_PICKER_ROW_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";
const MAX_COMPACT_TEAM_AVATARS = 3;

type TemplateAgentSelectionProps = {
  isLoading: boolean;
  isPending: boolean;
  personas: readonly AgentPersona[];
  selectedPersonaIds: readonly string[];
  selectedTeamIds: readonly string[];
  teams: readonly AgentTeam[];
};

export function TemplateAgentSummary({
  isLoading,
  isPending,
  onEdit,
  personas,
  selectedPersonaIds,
  selectedTeamIds,
  teams,
}: TemplateAgentSelectionProps & { onEdit: () => void }) {
  const personasById = React.useMemo(
    () => new Map(personas.map((persona) => [persona.id, persona])),
    [personas],
  );
  const selectedTeams = selectedTeamIds.flatMap((teamId) => {
    const team = teams.find((candidate) => candidate.id === teamId);
    return team ? [team] : [];
  });
  const selectedPersonas = selectedPersonaIds.flatMap((personaId) => {
    const persona = personasById.get(personaId);
    return persona ? [persona] : [];
  });
  const hasSelections = selectedTeams.length > 0 || selectedPersonas.length > 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex items-center justify-between gap-3">
        <span className="text-sm font-medium text-foreground">Agents</span>
        <Button
          data-testid="channel-template-edit-agents"
          disabled={isPending}
          onClick={onEdit}
          size="sm"
          type="button"
          variant="outline"
        >
          Edit
        </Button>
      </div>
      <div
        className="min-h-14 max-h-36 overflow-y-auto rounded-xl border border-border/70 bg-background/70"
        data-testid="channel-template-agent-summary"
      >
        {isLoading ? (
          <p className="px-4 py-5 text-sm text-muted-foreground">
            Loading included agents...
          </p>
        ) : null}
        {!isLoading && !hasSelections ? (
          <p className="px-4 py-5 text-center text-sm text-muted-foreground">
            No agent teams or agents selected.
          </p>
        ) : null}
        {hasSelections ? (
          <div>
            {selectedTeams.map((team) => {
              const teamPersonas = team.personaIds.flatMap((personaId) => {
                const persona = personasById.get(personaId);
                return persona ? [persona] : [];
              });
              return (
                <TemplateSummaryRow
                  avatar={
                    <CompactTeamAvatarStack
                      memberCount={team.personaIds.length}
                      personas={teamPersonas}
                      teamName={team.name}
                    />
                  }
                  key={team.id}
                  label={team.name}
                  reserveThreeAvatarSlots
                  testId={`channel-template-summary-team-${team.id}`}
                />
              );
            })}
            {selectedPersonas.map((persona) => (
              <TemplateSummaryRow
                avatar={
                  <ProfileAvatar
                    avatarUrl={persona.avatarUrl}
                    className="h-8 w-8 text-xs shadow-none"
                    label={persona.displayName}
                  />
                }
                key={persona.id}
                label={persona.displayName}
                reserveThreeAvatarSlots
                testId={`channel-template-summary-member-${persona.id}`}
              />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function TemplateAgentPicker({
  isPending,
  isLoading,
  onToggleMember,
  onToggleTeam,
  personas,
  selectedPersonaIds,
  selectedTeamIds,
  showTitle = true,
  teams,
}: TemplateAgentSelectionProps & {
  onToggleMember: (personaId: string) => void;
  onToggleTeam: (teamId: string) => void;
  showTitle?: boolean;
}) {
  const personasById = React.useMemo(
    () => new Map(personas.map((persona) => [persona.id, persona])),
    [personas],
  );
  const isEmpty = !isLoading && teams.length === 0 && personas.length === 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      {showTitle ? (
        <span
          className="text-sm font-medium text-foreground"
          data-testid="channel-template-agent-picker-title"
        >
          Agents
        </span>
      ) : null}
      <div
        className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-border/70 bg-background/70"
        data-testid="channel-template-agent-picker"
      >
        {isLoading ? (
          <p className="px-4 py-5 text-sm text-muted-foreground">
            Loading agent teams and agents...
          </p>
        ) : null}
        {isEmpty ? (
          <p className="px-4 py-5 text-center text-sm text-muted-foreground">
            No agent teams or agents are available.
          </p>
        ) : null}
        {teams.length > 0 ? (
          <section data-testid="channel-template-picker-teams">
            <TemplatePickerSectionHeader testId="channel-template-teams-header">
              Agent teams
            </TemplatePickerSectionHeader>
            {teams.map((team) => {
              const teamPersonas = team.personaIds.flatMap((personaId) => {
                const persona = personasById.get(personaId);
                return persona ? [persona] : [];
              });

              return (
                <TemplatePickerRow
                  avatar={
                    <CompactTeamAvatarStack
                      memberCount={team.personaIds.length}
                      personas={teamPersonas}
                      teamName={team.name}
                    />
                  }
                  disabled={isPending}
                  isSelected={selectedTeamIds.includes(team.id)}
                  key={team.id}
                  label={team.name}
                  onToggle={() => onToggleTeam(team.id)}
                  reserveThreeAvatarSlots
                  testId={`channel-template-team-row-${team.id}`}
                />
              );
            })}
          </section>
        ) : null}
        {personas.length > 0 ? (
          <section data-testid="channel-template-picker-members">
            <TemplatePickerSectionHeader testId="channel-template-members-header">
              Agents
            </TemplatePickerSectionHeader>
            {personas.map((persona) => (
              <TemplatePickerRow
                avatar={
                  <ProfileAvatar
                    avatarUrl={persona.avatarUrl}
                    className="h-8 w-8 text-xs shadow-none"
                    label={persona.displayName}
                  />
                }
                disabled={isPending}
                isSelected={selectedPersonaIds.includes(persona.id)}
                key={persona.id}
                label={persona.displayName}
                onToggle={() => onToggleMember(persona.id)}
                testId={`channel-template-member-row-${persona.id}`}
              />
            ))}
          </section>
        ) : null}
      </div>
    </div>
  );
}

function TemplatePickerSectionHeader({
  children,
  testId,
}: {
  children: React.ReactNode;
  testId: string;
}) {
  return (
    <div
      className="sticky top-0 z-10 mr-3 flex min-h-9 items-center bg-background/95 px-4 pb-1.5 pt-3 text-xs font-medium text-muted-foreground/75 backdrop-blur supports-[backdrop-filter]:bg-background/80"
      data-testid={testId}
    >
      {children}
    </div>
  );
}

function TemplateSummaryRow({
  avatar,
  label,
  reserveThreeAvatarSlots = false,
  testId,
}: {
  avatar: React.ReactNode;
  label: string;
  reserveThreeAvatarSlots?: boolean;
  testId: string;
}) {
  return (
    <div
      className={cn(
        "relative flex min-h-12 items-center gap-3 px-4 py-2.5",
        TEMPLATE_PICKER_ROW_DIVIDER_CLASS,
      )}
      data-testid={testId}
    >
      <div
        className={cn(
          "flex h-8 min-w-8 shrink-0 items-center",
          reserveThreeAvatarSlots && "w-[4.5rem]",
        )}
      >
        {avatar}
      </div>
      <span className="min-w-0 flex-1 truncate text-sm font-medium tracking-tight">
        {label}
      </span>
    </div>
  );
}

function TemplatePickerRow({
  avatar,
  disabled,
  isSelected,
  label,
  onToggle,
  reserveThreeAvatarSlots = false,
  testId,
}: {
  avatar: React.ReactNode;
  disabled: boolean;
  isSelected: boolean;
  label: string;
  onToggle: () => void;
  reserveThreeAvatarSlots?: boolean;
  testId: string;
}) {
  return (
    <div
      className={cn(
        "group/add-result relative isolate flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
        TEMPLATE_PICKER_ROW_DIVIDER_CLASS,
        isSelected && "bg-muted/40",
      )}
      data-testid={testId}
    >
      <button
        aria-label={`${isSelected ? "Remove" : "Add"} ${label}`}
        aria-pressed={isSelected}
        className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        disabled={disabled}
        onClick={onToggle}
        type="button"
      />
      <div
        className={cn(
          "pointer-events-none relative z-10 flex h-8 min-w-8 shrink-0 items-center",
          reserveThreeAvatarSlots && "w-[4.5rem]",
        )}
      >
        {avatar}
      </div>
      <div className="pointer-events-none relative z-10 min-w-0 flex-1">
        <span
          className="block truncate text-sm font-medium tracking-tight"
          data-template-picker-row-title
        >
          {label}
        </span>
      </div>
      <Button
        aria-label={isSelected ? "Remove" : "Add"}
        className="relative z-20 shrink-0"
        disabled={disabled}
        onClick={(event) => {
          event.stopPropagation();
          onToggle();
        }}
        size="sm"
        type="button"
        variant={isSelected ? "outline" : "default"}
      >
        {isSelected ? "Remove" : "Add"}
      </Button>
    </div>
  );
}

export function CompactTeamAvatarStack({
  memberCount,
  personas,
  teamName,
}: {
  memberCount: number;
  personas: readonly AgentPersona[];
  teamName: string;
}) {
  const visibleLimit =
    memberCount > MAX_COMPACT_TEAM_AVATARS
      ? MAX_COMPACT_TEAM_AVATARS - 1
      : MAX_COMPACT_TEAM_AVATARS;
  const visiblePersonas = personas.slice(0, visibleLimit);
  const overflowCount = Math.max(0, memberCount - visiblePersonas.length);
  const stackItemCount = visiblePersonas.length + (overflowCount > 0 ? 1 : 0);

  if (stackItemCount === 0) {
    return (
      <span className="flex h-8 w-8 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Users className="h-4 w-4" />
      </span>
    );
  }

  return (
    <span
      aria-label={`${teamName} member avatars`}
      className="flex h-8 items-center"
      data-testid="compact-team-avatar-stack"
      role="img"
    >
      {visiblePersonas.map((persona, index) => (
        <span
          className={cn("h-8 w-8", index > 0 && "-ml-3")}
          data-compact-team-avatar-slot="member"
          key={persona.id}
          style={{
            zIndex: index + 1,
            ...(index < stackItemCount - 1 && {
              mask: "radial-gradient(circle 18px at calc(100% + 5px) 50%, transparent 99%, #fff 100%)",
              WebkitMask:
                "radial-gradient(circle 18px at calc(100% + 5px) 50%, transparent 99%, #fff 100%)",
            }),
          }}
        >
          <ProfileAvatar
            avatarUrl={persona.avatarUrl}
            className="h-full w-full bg-muted text-xs shadow-none"
            label={persona.displayName}
          />
        </span>
      ))}
      {overflowCount > 0 ? (
        <span
          className={cn(
            "flex h-8 w-8 items-center justify-center rounded-full bg-muted text-2xs font-semibold text-muted-foreground",
            visiblePersonas.length > 0 && "-ml-3",
          )}
          data-compact-team-avatar-slot="overflow"
          style={{ zIndex: stackItemCount }}
        >
          +{overflowCount}
        </span>
      ) : null}
    </span>
  );
}
