import {
  CopyPlus,
  Download,
  Ellipsis,
  FolderOpen,
  FolderSync,
  Pencil,
  Rocket,
  Trash2,
} from "lucide-react";

import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
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
  onExport: (team: AgentTeam) => void;
  onDelete: (team: AgentTeam) => void;
  onAddToChannel: (team: AgentTeam) => void;
  onSync: (team: AgentTeam) => void;
  onRevealInFinder: (team: AgentTeam) => void;
  onImportFile: (fileBytes: number[], fileName: string) => void;
  onInstallFromDirectory?: () => void;
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
  onExport,
  onDelete,
  onAddToChannel,
  onSync,
  onRevealInFinder,
  onImportFile,
  onInstallFromDirectory,
}: TeamsSectionProps) {
  const {
    fileInputRef,
    isDragOver,
    dropHandlers,
    handleFileChange,
    openFilePicker,
  } = useFileImportZone({ onImportFile });

  return (
    <section
      className="relative space-y-4"
      data-testid="agents-library-teams"
      {...dropHandlers}
    >
      {isDragOver ? (
        <div className="pointer-events-none absolute -inset-1 z-10 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/50 bg-background/80 backdrop-blur-sm">
          <p className="text-sm font-medium text-primary">
            Drop .team.json or .zip to import
          </p>
        </div>
      ) : null}
      <input
        accept=".json,.zip"
        className="hidden"
        onChange={handleFileChange}
        ref={fileInputRef}
        type="file"
      />

      <div
        className={`${TEAM_CARD_COLUMN_CLASS} flex items-center justify-between gap-3`}
      >
        <div>
          <h3 className="text-sm font-semibold tracking-tight">Teams</h3>
          <p className="text-sm text-secondary-foreground/75">
            Saved groups from My Agents that you can add to a channel together.
          </p>
        </div>
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
                        className="flex h-7 w-7 items-center justify-center rounded-md bg-transparent text-muted-foreground/80 transition-colors hover:bg-background/85 hover:text-foreground data-[state=open]:bg-background/90 data-[state=open]:text-foreground"
                        type="button"
                      >
                        <Ellipsis className="h-4 w-4" />
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
                        onClick={() => onExport(team)}
                      >
                        <Download className="h-4 w-4" />
                        Export
                      </DropdownMenuItem>
                      {team.sourceDir ? (
                        <>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            disabled={isPending}
                            onClick={() => onSync(team)}
                          >
                            <FolderSync className="h-4 w-4" />
                            Sync from directory
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onClick={() => onRevealInFinder(team)}
                          >
                            <FolderOpen className="h-4 w-4" />
                            Reveal in Finder
                          </DropdownMenuItem>
                        </>
                      ) : null}
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
                    exporting.
                  </p>
                ) : null}
              </TeamIdentityCard>
            );
          })}
          <NewTeamCard
            isPending={isPending}
            onCreate={onCreate}
            onImport={openFilePicker}
            onInstallFromDirectory={onInstallFromDirectory}
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
  onInstallFromDirectory,
}: {
  isPending: boolean;
  onCreate: () => void;
  onImport: () => void;
  onInstallFromDirectory?: () => void;
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
        {onInstallFromDirectory ? (
          <DropdownMenuItem
            disabled={isPending}
            onClick={onInstallFromDirectory}
          >
            Install from directory
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem disabled={isPending} onClick={onImport}>
          Import team file
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
