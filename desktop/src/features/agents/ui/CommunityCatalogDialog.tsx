import * as React from "react";

import { isCatalogPersonaSelected } from "@/features/agents/lib/catalog";
import { isCatalogPersona } from "@/features/agents/lib/personaCatalogRelay";
import type { CatalogTeam } from "@/features/agents/lib/teamCatalogRelay";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Dialog } from "@/shared/ui/dialog";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Markdown } from "@/shared/ui/markdown";
import { Skeleton } from "@/shared/ui/skeleton";
import { ChevronDown } from "lucide-react";

import agentOutlineUrl from "../assets/agent-outline.svg";
import { AgentDefinitionMetadata } from "./AgentDefinitionMetadata";
import { PersonaAddedBy } from "./PersonaAddedBy";
import { resolveCatalogOwnerLabel } from "./catalogOwnerLabel";
import { nextCatalogSelection } from "./communityCatalogSelection";

// Inline instruction markdown class — no external consumers so kept internal.
const agentInstructionMarkdownClassName = [
  "mt-3 w-full min-w-0 max-w-full overflow-x-hidden leading-6 text-muted-foreground [&>*]:min-w-0 [&>*]:max-w-full [&_.code-block-lines]:min-w-0 [&_.code-block-lines]:max-w-full [&_.code-block-lines]:whitespace-pre-wrap [&_.code-block-lines]:[overflow-wrap:anywhere] [&_.inline-code-chip]:max-w-full [&_.inline-code-chip]:whitespace-pre-wrap [&_.inline-code-chip]:[overflow-wrap:anywhere] [&_blockquote]:!text-muted-foreground [&_code]:!text-muted-foreground [&_li]:text-muted-foreground [&_ol]:text-muted-foreground [&_p]:text-muted-foreground [&_strong]:text-muted-foreground [&_td]:text-muted-foreground [&_ul]:text-muted-foreground",
  "[&>h1]:!text-sm [&>h1]:!font-semibold [&>h1]:!leading-6 [&>h1]:!tracking-normal [&>h1]:!text-foreground",
  "[&>h2]:!text-sm [&>h2]:!font-semibold [&>h2]:!leading-6 [&>h2]:!tracking-normal [&>h2]:!text-foreground",
  "[&>h3]:!text-sm [&>h3]:!font-semibold [&>h3]:!leading-6 [&>h3]:!tracking-normal [&>h3]:!text-foreground",
  "[&>h4]:!text-sm [&>h4]:!font-semibold [&>h4]:!leading-6 [&>h4]:!tracking-normal [&>h4]:!text-foreground",
  "[&>h5]:!text-sm [&>h5]:!font-semibold [&>h5]:!leading-6 [&>h5]:!tracking-normal [&>h5]:!text-foreground",
  "[&>h6]:!text-sm [&>h6]:!font-semibold [&>h6]:!leading-6 [&>h6]:!tracking-normal [&>h6]:!text-foreground",
].join(" ");

// ── Type-tagged selection keys ────────────────────────────────────────────────

// IDs are implementation-defined strings; prefixing prevents collision between
// persona IDs and team coordinates.
type CatalogSelectionKey =
  | { kind: "persona"; id: string }
  | { kind: "team"; key: string };

function selectionKeyFor(persona: AgentPersona): CatalogSelectionKey {
  return { kind: "persona", id: persona.id };
}

function teamSelectionKey(team: CatalogTeam): string {
  return `${team.ownerPubkey}:${team.teamDTag}`;
}

function teamKeyFor(team: CatalogTeam): CatalogSelectionKey {
  return { kind: "team", key: teamSelectionKey(team) };
}

function encodeKey(k: CatalogSelectionKey): string {
  return k.kind === "persona" ? `p:${k.id}` : `t:${k.key}`;
}

// ── Props ─────────────────────────────────────────────────────────────────────

type CommunityCatalogDialogProps = {
  // Persona side
  personas: AgentPersona[];
  personasError: Error | null;
  personasLoading: boolean;
  personasPending: boolean;
  feedbackErrorMessage: string | null;
  feedbackNoticeMessage: string | null;
  onClearFeedback: () => void;
  onSelectPersona: (persona: AgentPersona, active: boolean) => void;

  // Team side
  teams: CatalogTeam[];
  teamsError: Error | null;
  teamsLoading: boolean;
  teamsAdding: boolean;
  onAddTeam: (team: CatalogTeam) => void;

  // Dialog
  open: boolean;
  preferSection: "agents" | "teams";
  onOpenChange: (open: boolean) => void;
};

// ── CommunityCatalogDialog ────────────────────────────────────────────────────

export function CommunityCatalogDialog({
  personas,
  personasError,
  personasLoading,
  personasPending,
  feedbackErrorMessage,
  feedbackNoticeMessage,
  onClearFeedback,
  onSelectPersona,
  teams,
  teamsError,
  teamsLoading,
  teamsAdding,
  onAddTeam,
  open,
  preferSection,
  onOpenChange,
}: CommunityCatalogDialogProps) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);

  // Selected item — a type-tagged key guarantees persona IDs and team
  // coordinates cannot collide.
  const [selectedEncodedKey, setSelectedEncodedKey] = React.useState<
    string | null
  >(null);

  // Tracks whether the user has explicitly clicked an item since the dialog
  // opened. When true, automatic initialization must not overwrite it.
  const [userHasSelected, setUserHasSelected] = React.useState(false);

  // Compute the preferred first item for each section.
  const firstPersonaKey =
    personas.length > 0 ? encodeKey(selectionKeyFor(personas[0])) : null;
  const firstTeamKey =
    teams.length > 0 ? encodeKey(teamKeyFor(teams[0])) : null;

  // Whether the current selection still exists in the live data. Used to
  // detect when a live relay refresh retracts an item the user had selected.
  const currentIsValid =
    selectedEncodedKey !== null &&
    (personas.some(
      (p) => encodeKey(selectionKeyFor(p)) === selectedEncodedKey,
    ) ||
      teams.some((t) => encodeKey(teamKeyFor(t)) === selectedEncodedKey));

  // When the dialog opens or any dependency changes, run the selection
  // initialization logic. Respects the launch preference without committing
  // a cross-section fallback while the preferred section is still loading,
  // and never overwrites a still-valid explicit user selection.
  React.useEffect(() => {
    if (!open) {
      // Reset user-selection guard whenever the dialog closes so the next
      // open gets a fresh auto-init.
      setUserHasSelected(false);
      return;
    }

    // If the user's explicit selection was retracted by a live refresh, clear
    // the intent guard so the replacement is treated as auto-init.
    if (userHasSelected && !currentIsValid) {
      setUserHasSelected(false);
    }

    setSelectedEncodedKey((current) =>
      nextCatalogSelection(
        current,
        userHasSelected,
        currentIsValid,
        preferSection,
        personasLoading,
        teamsLoading,
        firstPersonaKey,
        firstTeamKey,
      ),
    );
  }, [
    open,
    preferSection,
    personasLoading,
    teamsLoading,
    firstPersonaKey,
    firstTeamKey,
    userHasSelected,
    currentIsValid,
  ]);

  useFeedbackToasts(feedbackNoticeMessage, feedbackErrorMessage);

  // Resolve current selection.
  const selectedPersona = selectedEncodedKey?.startsWith("p:")
    ? (personas.find((p) => p.id === selectedEncodedKey.slice(2)) ?? null)
    : null;
  const selectedTeamKey = selectedEncodedKey?.startsWith("t:")
    ? selectedEncodedKey.slice(2)
    : null;
  const selectedTeam = selectedTeamKey
    ? (teams.find((t) => teamSelectionKey(t) === selectedTeamKey) ?? null)
    : null;

  const selectedPersonaIsActive = selectedPersona
    ? isCatalogPersonaSelected(selectedPersona)
    : false;
  const selectedTeamIsAdded = selectedTeam?.localTeam != null;
  const selectedTeamIsInvalid = (selectedTeam?.invalidMemberCount ?? 0) > 0;

  const bothEmpty =
    !personasLoading &&
    !teamsLoading &&
    personas.length === 0 &&
    teams.length === 0;
  const noError = !personasError && !teamsError;

  function handleUseAgent() {
    if (!selectedPersona || selectedPersonaIsActive) return;
    onClearFeedback();
    onSelectPersona(selectedPersona, true);
  }

  function handleAddTeam() {
    if (!selectedTeam || selectedTeamIsAdded || selectedTeamIsInvalid) return;
    onAddTeam(selectedTeam);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="h-[42rem] max-w-4xl"
        contentClassName="flex min-h-0 min-w-0 flex-1 p-0"
        data-testid="community-catalog-dialog"
        description="Browse agents and teams shared to this relay."
        headerClassName="bg-sidebar pb-3 text-sidebar-foreground"
        headerTestId="community-catalog-dialog-header"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          contentRef.current?.focus();
        }}
        ref={contentRef}
        scrollAreaClassName="flex min-h-0 overflow-hidden px-0"
        scrollAreaTestId="community-catalog-dialog-body"
        tabIndex={-1}
        title="Community Catalog"
      >
        {bothEmpty && noError ? (
          <CatalogEmptyState />
        ) : (
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-sidebar sm:flex-row">
            {/* Sidebar list */}
            <div className="flex max-h-56 min-h-0 flex-col sm:max-h-none sm:w-56">
              <div
                className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
                data-testid="community-catalog-dialog-scroll-area"
              >
                {personasLoading ? <CatalogListSkeleton /> : null}

                {!personasLoading && personas.length > 0 ? (
                  <div className="mb-1">
                    <p className="px-4 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wider text-sidebar-foreground/60">
                      Agents
                    </p>
                    <div className="space-y-1">
                      {personas.map((persona) => {
                        const key = encodeKey(selectionKeyFor(persona));
                        const isCurrent = key === selectedEncodedKey;
                        return (
                          <button
                            aria-current={isCurrent ? "true" : undefined}
                            className={cn(
                              "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
                              isCurrent
                                ? "bg-sidebar-active text-sidebar-active-foreground"
                                : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                            )}
                            data-testid={`community-catalog-agent-${persona.id}`}
                            key={persona.id}
                            onClick={() => {
                              setUserHasSelected(true);
                              setSelectedEncodedKey(key);
                            }}
                            type="button"
                          >
                            <ProfileAvatar
                              avatarUrl={persona.avatarUrl}
                              className="h-6 w-6 text-3xs"
                              label={persona.displayName}
                            />
                            <span className="min-w-0 flex-1 truncate text-sm font-medium">
                              {persona.displayName}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ) : null}

                {teamsLoading ? <CatalogListSkeleton /> : null}

                {!teamsLoading && teams.length > 0 ? (
                  <div className="mb-1">
                    <p className="px-4 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wider text-sidebar-foreground/60">
                      Teams
                    </p>
                    <div className="space-y-1">
                      {teams.map((team) => {
                        const key = encodeKey(teamKeyFor(team));
                        const isCurrent = key === selectedEncodedKey;
                        return (
                          <button
                            aria-current={isCurrent ? "true" : undefined}
                            className={cn(
                              "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
                              isCurrent
                                ? "bg-sidebar-active text-sidebar-active-foreground"
                                : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                            )}
                            data-testid={`community-catalog-team-${teamSelectionKey(team)}`}
                            key={teamSelectionKey(team)}
                            onClick={() => {
                              setUserHasSelected(true);
                              setSelectedEncodedKey(key);
                            }}
                            type="button"
                          >
                            <span className="min-w-0 flex-1 truncate text-sm font-medium">
                              {team.name}
                            </span>
                            <span className="shrink-0 text-xs tabular-nums opacity-70">
                              {team.members.length}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ) : null}
              </div>
            </div>

            {/* Detail pane */}
            <div className="relative z-10 mb-3 ml-px mr-3 flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-background shadow-[-1px_0_0_0_hsl(var(--sidebar-border)/0.45)]">
              <div
                className="min-h-0 min-w-0 max-w-full flex-1 overflow-x-hidden overflow-y-auto px-5 pb-24 pt-5"
                data-testid="community-catalog-detail-pane"
              >
                {personasLoading && teamsLoading ? (
                  <CatalogDetailSkeleton />
                ) : null}

                {!personasLoading && selectedPersona ? (
                  <PersonaCatalogDetail persona={selectedPersona} />
                ) : null}

                {!teamsLoading && selectedTeam ? (
                  <TeamCatalogDetail team={selectedTeam} />
                ) : null}

                {/* Per-section errors — only blank the section that failed */}
                {personasError ? (
                  <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {personasError.message}
                  </p>
                ) : null}
                {teamsError ? (
                  <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    {teamsError.message}
                  </p>
                ) : null}

                {selectedTeam && selectedTeamIsInvalid ? (
                  <p
                    className="mt-4 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
                    data-testid="community-catalog-invalid-member-warning"
                  >
                    {selectedTeam.invalidMemberCount === 1
                      ? "1 member in this team could not be verified and cannot be added."
                      : `${selectedTeam.invalidMemberCount} members in this team could not be verified and cannot be added.`}
                  </p>
                ) : null}
              </div>

              {/* Footer action — context-sensitive per selection type */}
              <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-end bg-gradient-to-t from-background via-background/95 to-transparent px-5 pb-4 pt-12">
                {selectedPersona ? (
                  <Button
                    aria-label={
                      selectedPersonaIsActive
                        ? `${selectedPersona.displayName} is already in My Agents`
                        : `Add ${selectedPersona.displayName} from Community Catalog`
                    }
                    className="pointer-events-auto"
                    data-testid={`community-catalog-use-agent-${selectedPersona.id}`}
                    disabled={
                      !selectedPersona ||
                      selectedPersonaIsActive ||
                      personasPending
                    }
                    onClick={handleUseAgent}
                    size="sm"
                    type="button"
                  >
                    {selectedPersonaIsActive
                      ? "Added to My Agents"
                      : "Add agent"}
                  </Button>
                ) : selectedTeam ? (
                  <Button
                    aria-label={
                      selectedTeamIsAdded
                        ? `${selectedTeam.name} is already in your teams`
                        : selectedTeamIsInvalid
                          ? `${selectedTeam.name} cannot be added because it contains invalid members`
                          : `Add ${selectedTeam.name} from Community Catalog`
                    }
                    className="pointer-events-auto"
                    data-testid="community-catalog-add-team"
                    disabled={
                      !selectedTeam ||
                      selectedTeamIsAdded ||
                      teamsAdding ||
                      selectedTeamIsInvalid
                    }
                    onClick={handleAddTeam}
                    size="sm"
                    type="button"
                  >
                    {selectedTeamIsAdded
                      ? "Added to my teams"
                      : teamsAdding
                        ? "Adding…"
                        : "Add team"}
                  </Button>
                ) : null}
              </div>
            </div>
          </div>
        )}
      </ChooserDialogContent>
    </Dialog>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function CatalogEmptyState() {
  return (
    <div
      className="flex min-h-0 flex-1 items-center justify-center bg-sidebar px-6 py-10 text-center"
      data-testid="community-catalog-empty-state"
    >
      <div className="flex max-w-sm flex-col items-center">
        <img
          alt=""
          aria-hidden="true"
          className="h-32 w-32 dark:opacity-60"
          data-testid="community-catalog-empty-artwork"
          src={agentOutlineUrl}
        />
        <p className="mt-4 text-sm font-semibold">Nothing shared yet</p>
        <p className="mt-2 text-sm text-muted-foreground">
          Shared agents and teams will appear here.
        </p>
      </div>
    </div>
  );
}

// ── Skeleton loaders ──────────────────────────────────────────────────────────

function CatalogListSkeleton() {
  return (
    <div className="space-y-2">
      {["first", "second", "third", "fourth", "fifth"].map((key) => (
        <div
          className="flex items-center gap-2 rounded-lg px-4 py-1.5"
          key={key}
        >
          <Skeleton className="h-6 w-6 rounded-full" />
          <Skeleton className="h-4 w-28" />
        </div>
      ))}
    </div>
  );
}

function CatalogDetailSkeleton() {
  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Skeleton className="h-12 w-12 rounded-full" />
        <Skeleton className="h-6 w-40" />
      </div>
      <div className="grid overflow-hidden rounded-lg border border-border/70 sm:grid-cols-3">
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
        <Skeleton className="h-20 rounded-none" />
      </div>
      <Skeleton className="h-48 rounded-lg" />
    </div>
  );
}

// ── Persona detail ────────────────────────────────────────────────────────────

function PersonaCatalogDetail({ persona }: { persona: AgentPersona }) {
  const isCommunityEntry =
    isCatalogPersona(persona) && !persona.catalogSource.isOwn;
  const ownerPubkey = isCommunityEntry
    ? persona.catalogSource.ownerPubkey
    : undefined;
  const ownerBatchQuery = useUsersBatchQuery(ownerPubkey ? [ownerPubkey] : [], {
    enabled: !!ownerPubkey,
  });

  let addedByLabel: string;
  if (!isCommunityEntry) {
    addedByLabel = "You";
  } else {
    const summary = ownerPubkey
      ? ownerBatchQuery.data?.profiles[ownerPubkey.toLowerCase()]
      : undefined;
    addedByLabel = resolveCatalogOwnerLabel(summary);
  }

  return (
    <div className="w-full min-w-0 max-w-full space-y-6 overflow-x-hidden">
      <div className="flex items-center gap-3">
        <ProfileAvatar
          avatarUrl={persona.avatarUrl}
          className="h-12 w-12 text-sm"
          label={persona.displayName}
        />
        <div className="min-w-0">
          <h3 className="truncate text-xl font-semibold leading-snug">
            {persona.displayName}
          </h3>
          {persona.isBuiltIn ? null : (
            <PersonaAddedBy className="mt-0.5" label={addedByLabel} />
          )}
        </div>
      </div>

      <AgentDefinitionMetadata
        isBuiltIn={persona.isBuiltIn}
        model={persona.model}
        provider={persona.provider}
        runtime={persona.runtime}
      />

      <div className="min-w-0 max-w-full pt-3">
        <p className="text-base font-semibold text-foreground">
          Agent instruction
        </p>
        <Markdown
          className={agentInstructionMarkdownClassName}
          content={persona.systemPrompt}
          interactive={false}
        />
      </div>
    </div>
  );
}

// ── Team detail ───────────────────────────────────────────────────────────────

function TeamCatalogDetail({ team }: { team: CatalogTeam }) {
  const ownerPubkey = team.isOwn ? undefined : team.ownerPubkey;
  const ownerBatchQuery = useUsersBatchQuery(ownerPubkey ? [ownerPubkey] : [], {
    enabled: !!ownerPubkey,
  });

  let addedByLabel: string;
  if (team.isOwn) {
    addedByLabel = "You";
  } else {
    const summary = ownerPubkey
      ? ownerBatchQuery.data?.profiles[ownerPubkey.toLowerCase()]
      : undefined;
    addedByLabel = resolveCatalogOwnerLabel(summary);
  }

  const hasInstructions =
    team.instructions !== null && team.instructions.trim().length > 0;

  return (
    <div className="w-full min-w-0 max-w-full space-y-6 overflow-x-hidden">
      <div className="min-w-0">
        <h3 className="truncate text-xl font-semibold leading-snug">
          {team.name}
        </h3>
        <PersonaAddedBy className="mt-0.5" label={addedByLabel} />
        {team.description ? (
          <p className="mt-3 text-sm text-muted-foreground">
            {team.description}
          </p>
        ) : null}
      </div>

      {hasInstructions ? (
        <div className="min-w-0 max-w-full pt-3">
          <p className="text-base font-semibold text-foreground">
            Team instructions
          </p>
          <Markdown
            className={agentInstructionMarkdownClassName}
            content={team.instructions ?? ""}
            interactive={false}
          />
        </div>
      ) : null}

      <div className="min-w-0">
        <p className="text-base font-semibold text-foreground">
          {team.members.length}{" "}
          {team.members.length === 1 ? "member" : "members"}
        </p>
        <ul className="mt-3 space-y-2">
          {team.members.map((member) => (
            <TeamCatalogMemberRow key={member.memberKey} member={member} />
          ))}
        </ul>
      </div>
    </div>
  );
}

type TeamCatalogMemberRowProps = {
  member: CatalogTeam["members"][number];
};

function TeamCatalogMemberRow({ member }: TeamCatalogMemberRowProps) {
  const [expanded, setExpanded] = React.useState(false);

  return (
    <li
      className="overflow-hidden rounded-lg border border-border/70 bg-card/70"
      data-testid={`community-catalog-member-${member.memberKey}`}
    >
      <button
        aria-expanded={expanded}
        className="flex w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-muted/30 focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
        data-testid={`community-catalog-member-expand-${member.memberKey}`}
        onClick={() => setExpanded((v) => !v)}
        type="button"
      >
        <ProfileAvatar
          avatarUrl={member.avatarUrl}
          className="h-8 w-8 shrink-0 text-3xs"
          label={member.displayName}
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{member.displayName}</p>
          <p className="truncate text-xs text-muted-foreground">
            {member.model ?? "Use app default"}
          </p>
        </div>
        <ChevronDown
          className={cn(
            "h-4 w-4 shrink-0 text-muted-foreground transition-transform",
            expanded && "rotate-180",
          )}
        />
      </button>

      {expanded ? (
        <div className="space-y-4 border-t border-border/60 px-3 pb-4 pt-3">
          <AgentDefinitionMetadata
            isBuiltIn={false}
            model={member.model}
            provider={member.provider}
            runtime={member.runtime}
          />
          <div className="min-w-0 max-w-full">
            <p className="text-sm font-semibold text-foreground">
              Agent instruction
            </p>
            {member.systemPrompt.trim().length > 0 ? (
              <Markdown
                className={agentInstructionMarkdownClassName}
                content={member.systemPrompt}
                interactive={false}
              />
            ) : (
              <p className="mt-3 text-sm italic text-muted-foreground/60">
                No instructions
              </p>
            )}
          </div>
        </div>
      ) : null}
    </li>
  );
}
