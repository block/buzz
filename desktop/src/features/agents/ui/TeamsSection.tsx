import {
  CopyPlus,
  EllipsisVertical,
  Pencil,
  Rocket,
  Share2,
  Trash2,
  Upload,
} from "lucide-react";

import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { CreateIdentityCard } from "./CreateIdentityCard";
import { TeamIdentityCard } from "./TeamIdentityCard";

const TEAM_CARD_COLUMN_CLASS = "w-full";
const TEAM_CARD_GRID_CLASS = `${TEAM_CARD_COLUMN_CLASS} grid grid-cols-[repeat(auto-fill,minmax(220px,240px))] justify-start gap-3`;

type TeamsSectionProps = {
  teams: AgentTeam[];
  personas: AgentPersona[];
  error: Error | null;
  isLoading: boolean;
  isPending: boolean;
  onCreate: () => void;
  onDuplicate: (team: AgentTeam) => void;
  onEdit: (team: AgentTeam) => void;
  onDelete: (team: AgentTeam) => void;
  onAddToChannel: (team: AgentTeam) => void;
  onShare: (team: AgentTeam) => void;
  onImport: () => void;
};

export function TeamsSection({
  teams,
  personas,
  error,
  isLoading,
  isPending,
  onCreate,
  onDuplicate,
  onEdit,
  onDelete,
  onAddToChannel,
  onShare,
  onImport,
}: TeamsSectionProps) {
  return (
    <section className="relative space-y-4" data-testid="agents-library-teams">
      <div className={TEAM_CARD_COLUMN_CLASS}>
        <SectionHeader
          title="Teams"
          description="Groups of your agents you can add to a channel together."
        />
      </div>

      {isLoading ? (
        <div className={TEAM_CARD_GRID_CLASS}>
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-14"
            footerTitleWidthClass="w-24"
            showAction
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-24"
            footerTitleWidthClass="w-32"
            showAction
          />
          <IdentityCardSkeleton
            footerSubtitleWidthClass="w-20"
            footerTitleWidthClass="w-28"
            showAction
          />
        </div>
      ) : null}

      {!isLoading ? (
        <div className={TEAM_CARD_GRID_CLASS}>
          {teams.map((team) => {
            const resolution = resolveTeamPersonas(team, personas);
            const missingPersonaCount = resolution.missingPersonaCount;
            const hasMissingPersonas = resolution.hasMissingPersonas;

            return (
              <TeamIdentityCard
                actions={
                  <DropdownMenu modal={false}>
                    <DropdownMenuTrigger asChild>
                      <button
                        aria-label={`${team.name} team actions`}
                        className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                        type="button"
                      >
                        <EllipsisVertical className="h-4 w-4" />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent
                      align="end"
                      onCloseAutoFocus={(event) => event.preventDefault()}
                    >
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onAddToChannel(team)}
                      >
                        <Rocket className="h-4 w-4" />
                        Deploy to channel
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        disabled={isPending}
                        onClick={() => onEdit(team)}
                      >
                        <Pencil className="h-4 w-4" />
                        Edit
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onDuplicate(team)}
                      >
                        <CopyPlus className="h-4 w-4" />
                        Duplicate
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onShare(team)}
                      >
                        <Share2 className="h-4 w-4" />
                        Share
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        className="text-destructive focus:text-destructive"
                        disabled={isPending}
                        onClick={() => onDelete(team)}
                      >
                        <Trash2 className="h-4 w-4" />
                        Delete
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                }
                dataTestId={`team-card-${team.id}`}
                description={team.description}
                isSymlink={team.isSymlink}
                key={team.id}
                memberCount={team.personaIds.length}
                personas={resolution.resolvedPersonas}
                sourceDir={team.sourceDir}
                symlinkTarget={team.symlinkTarget}
                teamName={team.name}
                version={team.version}
              >
                {hasMissingPersonas ? (
                  <p className="border-t border-destructive/20 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    {missingPersonaCount} agent
                    {missingPersonaCount === 1 ? "" : "s"} in this team{" "}
                    {missingPersonaCount === 1 ? "is" : "are"} no longer in your
                    My Agents. Edit the team to repair it before deploying or
                    sharing.
                  </p>
                ) : null}
              </TeamIdentityCard>
            );
          })}
          <NewTeamCard
            isPending={isPending}
            onCreate={onCreate}
            onImport={onImport}
          />
        </div>
      ) : null}

      {error ? (
        <p
          className={`${TEAM_CARD_COLUMN_CLASS} rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive`}
        >
          {error.message}
        </p>
      ) : null}
    </section>
  );
}

function NewTeamCard({
  isPending,
  onCreate,
  onImport,
}: {
  isPending: boolean;
  onCreate: () => void;
  onImport: () => void;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <CreateIdentityCard
          ariaLabel="New team"
          dataTestId="new-team-card"
          label="New team"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem disabled={isPending} onClick={onCreate}>
          Create team
        </DropdownMenuItem>
        <DropdownMenuItem disabled={isPending} onClick={onImport}>
          <Upload className="h-4 w-4" />
          Import team snapshot
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
