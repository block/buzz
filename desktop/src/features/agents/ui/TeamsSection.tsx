import {
  CopyPlus,
  Download,
  Ellipsis,
  FolderOpen,
  FolderSync,
  Info,
  Link,
  Pencil,
  Rocket,
  Trash2,
  Upload,
  Users,
} from "lucide-react";

import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona, AgentTeam } from "@/shared/api/types";
import { useFileImportZone } from "@/shared/hooks/useFileImportZone";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Card } from "@/shared/ui/card";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { CreateNewButton } from "./CreateNewButton";

const MAX_VISIBLE_AVATARS = 4;

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
  onImportFile: (fileBytes: number[], fileName: string) => void;
  onInstallFromDirectory: () => void;
  onSync: (team: AgentTeam) => void;
  onRevealInFinder: (team: AgentTeam) => void;
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
  onImportFile,
  onInstallFromDirectory,
  onSync,
  onRevealInFinder,
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
            Drop .team.json to import
          </p>
        </div>
      ) : null}

      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold tracking-tight">My teams</h3>
          <p className="text-sm text-muted-foreground">
            Saved groups from My Agents that you can add to a channel together.
          </p>
        </div>
        <input
          accept=".json,.zip"
          className="hidden"
          onChange={handleFileChange}
          ref={fileInputRef}
          type="file"
        />
        <div className="flex items-center gap-2">
          <button
            className="inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            onClick={onInstallFromDirectory}
            type="button"
          >
            <FolderOpen className="h-4 w-4" />
            Install from directory
          </button>
          <CreateNewButton
            ariaLabel="Create team"
            label="Team"
            onClick={onCreate}
          />
        </div>
      </div>

      {isLoading ? (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {["first", "second", "third"].map((key) => (
            <Card className="p-3" key={key}>
              <div className="flex items-center gap-2.5">
                <Skeleton className="h-8 w-8 rounded-lg" />
                <div className="space-y-2">
                  <Skeleton className="h-4 w-32" />
                  <Skeleton className="h-3 w-20 rounded-full" />
                </div>
              </div>
            </Card>
          ))}
        </div>
      ) : null}

      {!isLoading && teams.length > 0 ? (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {teams.map((team) => {
            const resolution = resolveTeamPersonas(team, personas);
            const visible = resolution.resolvedPersonas.slice(
              0,
              MAX_VISIBLE_AVATARS,
            );
            const overflow =
              resolution.resolvedPersonas.length - visible.length;
            const missingPersonaCount = resolution.missingPersonaCount;
            const hasMissingPersonas = resolution.hasMissingPersonas;

            return (
              <Card className="p-3" key={team.id}>
                <div className="flex items-start justify-between gap-2.5">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <Users className="h-4 w-4 shrink-0 text-muted-foreground" />
                      <p className="truncate text-sm font-semibold tracking-tight">
                        {team.name}
                      </p>
                      {team.isSymlink ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <span className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground">
                              <Link className="h-4 w-4" />
                            </span>
                          </TooltipTrigger>
                          <TooltipContent side="bottom" className="max-w-xs">
                            <p>
                              Linked from {team.symlinkTarget ?? team.sourceDir}
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      ) : null}
                      {team.version ? (
                        <span className="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                          v{team.version}
                        </span>
                      ) : null}
                      {team.description ? (
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <button
                              aria-label="View description"
                              className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:text-foreground"
                              type="button"
                            >
                              <Info className="h-4 w-4" />
                            </button>
                          </TooltipTrigger>
                          <TooltipContent side="bottom" className="max-w-xs">
                            <p>{team.description}</p>
                          </TooltipContent>
                        </Tooltip>
                      ) : null}
                    </div>

                    <div className="mt-2 flex items-center gap-2">
                      <div className="flex -space-x-1.5">
                        {visible.map((persona) => (
                          <ProfileAvatar
                            avatarUrl={persona.avatarUrl}
                            className="h-6 w-6 border-2 border-card text-[10px]"
                            key={persona.id}
                            label={persona.displayName}
                          />
                        ))}
                        {overflow > 0 ? (
                          <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-card bg-muted text-[10px] font-medium text-muted-foreground">
                            +{overflow}
                          </span>
                        ) : null}
                      </div>
                      <span className="text-xs text-muted-foreground">
                        {team.personaIds.length}{" "}
                        {team.personaIds.length === 1 ? "persona" : "personas"}
                      </span>
                    </div>
                  </div>

                  <DropdownMenu modal={false}>
                    <DropdownMenuTrigger asChild>
                      <button
                        className="flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
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
                </div>

                {hasMissingPersonas ? (
                  <p className="mt-3 rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    {missingPersonaCount} persona
                    {missingPersonaCount === 1 ? "" : "s"} in this team{" "}
                    {missingPersonaCount === 1 ? "is" : "are"} no longer in your
                    My Agents. Edit the team to repair it before deploying or
                    exporting.
                  </p>
                ) : null}
              </Card>
            );
          })}
          <button
            className="flex cursor-pointer items-center justify-center gap-2 rounded-xl border border-dashed border-primary p-3 text-primary transition-colors hover:bg-primary/5"
            onClick={openFilePicker}
            type="button"
          >
            <Upload className="h-4 w-4" />
            <span className="text-xs">Import</span>
          </button>
        </div>
      ) : null}

      {!isLoading && teams.length === 0 ? (
        <button
          className="w-full cursor-pointer rounded-xl border border-dashed border-primary/40 px-6 py-10 text-center transition-colors hover:border-primary hover:bg-primary/5"
          onClick={openFilePicker}
          type="button"
        >
          <p className="text-sm font-semibold tracking-tight">No teams yet</p>
          <p className="mt-2 text-sm text-muted-foreground">
            Create a team from the personas in My Agents for quick deployment to
            channels.
          </p>
          <p className="mt-1 text-xs text-muted-foreground/70">
            Or drop a .team.json file here to import.
          </p>
        </button>
      ) : null}

      {error ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
