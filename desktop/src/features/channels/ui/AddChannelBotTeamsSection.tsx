import { Check, Users } from "lucide-react";
import type * as React from "react";

import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

type SelectionChipButtonProps = {
  disabled: boolean;
  label: string;
  onClick: () => void;
  selected: boolean;
  children: React.ReactNode;
};

function SelectionChipButton({
  disabled,
  label: _label,
  onClick,
  selected,
  children,
}: SelectionChipButtonProps) {
  return (
    <button
      aria-pressed={selected}
      className={cn(
        "inline-flex min-h-9 items-center gap-2 rounded-full border px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        selected
          ? "border-primary bg-primary/10 text-foreground"
          : "border-border/80 bg-background/60 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
        disabled && "cursor-not-allowed opacity-50",
      )}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      {children}
    </button>
  );
}

type AddChannelBotTeamsSectionProps = {
  canToggleSelections: boolean;
  inChannelPersonaIds?: ReadonlySet<string>;
  isLoading: boolean;
  onToggleTeam: (personaIds: string[]) => void;
  personas: AgentPersona[];
  selectedPersonaIds: readonly string[];
  teams: AgentTeam[];
};

export function AddChannelBotTeamsSection({
  canToggleSelections,
  inChannelPersonaIds,
  isLoading,
  onToggleTeam,
  personas,
  selectedPersonaIds,
  teams,
}: AddChannelBotTeamsSectionProps) {
  if (isLoading || teams.length === 0) {
    return null;
  }

  return (
    <div className="space-y-3">
      <div>
        <div className="text-sm font-medium">Teams</div>
        <p className="text-xs text-muted-foreground">
          Select a team to toggle all its personas at once.
        </p>
      </div>

      <TooltipProvider delayDuration={150}>
        <div className="flex flex-wrap gap-2">
          {teams.map((team) => {
            const resolution = resolveTeamPersonas(team, personas);
            const validIds = resolution.resolvedPersonaIds;
            const allSelected =
              validIds.length > 0 &&
              validIds.every((id) => selectedPersonaIds.includes(id));
            const inChannelCount = inChannelPersonaIds
              ? validIds.filter((id) => inChannelPersonaIds.has(id)).length
              : 0;
            const allInChannel =
              inChannelCount > 0 && inChannelCount === validIds.length;

            return (
              <Tooltip key={team.id}>
                <TooltipTrigger asChild>
                  <div>
                    <SelectionChipButton
                      disabled={
                        !canToggleSelections ||
                        !resolution.isUsable ||
                        allInChannel
                      }
                      label={team.name}
                      onClick={() => onToggleTeam(validIds)}
                      selected={allSelected}
                    >
                      <Users
                        className={cn(
                          "h-4 w-4",
                          allSelected ? "text-primary" : "text-current",
                        )}
                      />
                      {team.name}
                      <span
                        className={cn(
                          "text-xs",
                          allSelected ? "text-primary/70" : "text-current/70",
                        )}
                      >
                        ({validIds.length})
                      </span>
                      {inChannelCount > 0 ? (
                        <span
                          className={cn(
                            "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-2xs font-medium leading-none",
                            allSelected
                              ? "bg-primary/15 text-primary"
                              : "bg-muted/60 text-muted-foreground",
                          )}
                        >
                          <Check className="h-4 w-4" />
                          {allInChannel
                            ? "All in channel"
                            : `${inChannelCount} in channel`}
                        </span>
                      ) : null}
                    </SelectionChipButton>
                  </div>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs text-left">
                  <div className="space-y-1.5">
                    <p className="font-medium">{team.name}</p>
                    {team.description ? (
                      <p className="text-2xs text-primary-foreground/80">
                        {team.description}
                      </p>
                    ) : null}
                    <div className="flex flex-wrap gap-1">
                      {resolution.resolvedPersonas.map((persona) => {
                        const personaInChannel =
                          inChannelPersonaIds?.has(persona.id) ?? false;
                        return (
                          <div
                            className="flex items-center gap-1 rounded-full bg-primary-foreground/10 px-1.5 py-0.5"
                            key={persona.id}
                          >
                            <ProfileAvatar
                              avatarUrl={persona.avatarUrl}
                              className="h-4 w-4 text-3xs bg-primary-foreground/20 text-primary-foreground"
                              label={persona.displayName}
                            />
                            <span className="text-2xs text-primary-foreground">
                              {persona.displayName}
                            </span>
                            {personaInChannel ? (
                              <Check className="h-4 w-4 text-emerald-300" />
                            ) : null}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </TooltipContent>
              </Tooltip>
            );
          })}
        </div>
      </TooltipProvider>
    </div>
  );
}
