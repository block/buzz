import {
  CopyPlus,
  EllipsisVertical,
  Pencil,
  Rocket,
  Share2,
  Trash2,
} from "lucide-react";

import { useTranslation } from "@/i18n";
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
import { IDENTITY_CARD_GRID_CLASS } from "./UnifiedAgentsSection";
import { teamCatalogCopy } from "./teamLibraryCopy";

const TEAM_CARD_COLUMN_CLASS = "w-full";

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
  onDiscover: () => void;
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
  onDiscover,
  onImport,
}: TeamsSectionProps) {
  const { t } = useTranslation();

  return (
    <section className="relative space-y-4" data-testid="agents-library-teams">
      <div className={TEAM_CARD_COLUMN_CLASS}>
        <SectionHeader
          title={t("agents.teams-section.title")}
          description={t("agents.teams-section.description")}
        />
      </div>

      {isLoading ? (
        <div className={IDENTITY_CARD_GRID_CLASS}>
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
        <div className={IDENTITY_CARD_GRID_CLASS}>
          <NewTeamCard
            isPending={isPending}
            onCreate={onCreate}
            onDiscover={onDiscover}
            onImport={onImport}
          />
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
                        aria-label={t("agents.teams-section.actions-aria", {
                          teamName: team.name,
                        })}
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
                        {t("agents.teams-section.deploy-to-channel")}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        disabled={isPending}
                        onClick={() => onEdit(team)}
                      >
                        <Pencil className="h-4 w-4" />
                        {t("agents.teams-section.edit")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onDuplicate(team)}
                      >
                        <CopyPlus className="h-4 w-4" />
                        {t("agents.teams-section.duplicate")}
                      </DropdownMenuItem>
                      <DropdownMenuItem
                        disabled={isPending || hasMissingPersonas}
                        onClick={() => onShare(team)}
                      >
                        <Share2 className="h-4 w-4" />
                        {t("agents.teams-section.share")}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        className="text-destructive focus:text-destructive"
                        disabled={isPending}
                        onClick={() => onDelete(team)}
                      >
                        <Trash2 className="h-4 w-4" />
                        {t("agents.teams-section.delete")}
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
                    {t("agents.teams-section.missing-in-team", {
                      count: missingPersonaCount,
                    })}
                  </p>
                ) : null}
              </TeamIdentityCard>
            );
          })}
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
  onDiscover,
  onImport,
}: {
  isPending: boolean;
  onCreate: () => void;
  onDiscover: () => void;
  onImport: () => void;
}) {
  const { t } = useTranslation();

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <CreateIdentityCard
          ariaLabel={t("agents.teams-section.new-team")}
          dataTestId="new-team-card"
        />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem disabled={isPending} onClick={onCreate}>
          {t("agents.teams-section.create-team")}
        </DropdownMenuItem>
        <DropdownMenuItem
          data-testid="team-catalog-open"
          disabled={isPending}
          onClick={onDiscover}
        >
          {teamCatalogCopy().chooseFromCatalog}
        </DropdownMenuItem>
        <DropdownMenuItem disabled={isPending} onClick={onImport}>
          {t("agents.teams-section.import")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
