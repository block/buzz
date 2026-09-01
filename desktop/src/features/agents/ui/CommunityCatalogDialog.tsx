import * as React from "react";
import { ChevronDown, Plus, Upload } from "lucide-react";

import { isCatalogPersonaSelected } from "@/features/agents/lib/catalog";
import { effectiveAgentDescription } from "@/features/agents/lib/agentDescription";
import { isCatalogPersona } from "@/features/agents/lib/personaCatalogRelay";
import type { CatalogTeam } from "@/features/agents/lib/teamCatalogRelay";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Dialog } from "@/shared/ui/dialog";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Skeleton } from "@/shared/ui/skeleton";

import agentOutlineUrl from "../assets/agent-outline.svg";
import { AgentDefinitionMetadata } from "./AgentDefinitionMetadata";
import { PersonaAddedBy } from "./PersonaAddedBy";
import { resolveCatalogOwnerLabel } from "./catalogOwnerLabel";

// ── Type-tagged selection keys ────────────────────────────────────────────────

// The detail pane is a single-select surface across four kinds of content:
// the "create" and "import" panes carried from the unified add-agent dialog
// (#5015), plus a persona or team browsed from the community catalog. Persona
// IDs and team coordinates are prefixed so they cannot collide with each other
// or with the fixed "create"/"import" keys.
type CatalogSelectionKey =
  | { kind: "persona"; id: string }
  | { kind: "team"; key: string };

function teamSelectionKey(team: CatalogTeam): string {
  return `${team.ownerPubkey}:${team.teamDTag}`;
}

function encodeKey(k: CatalogSelectionKey): string {
  return k.kind === "persona" ? `p:${k.id}` : `t:${k.key}`;
}

function personaKey(persona: AgentPersona): string {
  return encodeKey({ kind: "persona", id: persona.id });
}

function teamKey(team: CatalogTeam): string {
  return encodeKey({ kind: "team", key: teamSelectionKey(team) });
}

// ── Props ─────────────────────────────────────────────────────────────────────

type CommunityCatalogDialogProps = {
  // Create pane (unified add-agent flow, #5015). Rendered when the "create"
  // navigation item is active. The render-prop reports dirty state so the
  // dialog can guard navigation away from an in-progress draft.
  createContent: (controls: {
    onDirtyChange: (dirty: boolean) => void;
    onRequestClose: () => void;
  }) => React.ReactNode;
  onImportFile: (fileBytes: number[], fileName: string) => void;

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

type PendingNavigation =
  | { type: "close" }
  | { type: "selection"; selection: string };

// ── CommunityCatalogDialog ────────────────────────────────────────────────────

export function CommunityCatalogDialog({
  createContent,
  onImportFile,
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
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const dragDepthRef = React.useRef(0);
  const createDirtyRef = React.useRef(false);
  const didInitRef = React.useRef(false);
  const [isDragOver, setIsDragOver] = React.useState(false);
  const [pendingNavigation, setPendingNavigation] =
    React.useState<PendingNavigation | null>(null);

  // Active detail selection. Defaults to the create pane (the unified
  // add-agent entry point); a teams-launch may switch to the first team once
  // the teams query settles.
  const [selection, setSelection] = React.useState("create");
  const [userHasSelected, setUserHasSelected] = React.useState(false);

  const firstTeamKey = teams.length > 0 ? teamKey(teams[0]) : null;

  // Reset to a fresh create-pane state each time the dialog opens.
  React.useEffect(() => {
    if (open) {
      createDirtyRef.current = false;
      didInitRef.current = false;
      setUserHasSelected(false);
      setSelection("create");
      setPendingNavigation(null);
      dragDepthRef.current = 0;
      setIsDragOver(false);
    }
  }, [open]);

  // Teams-launch: once the teams query settles, select the first team so the
  // catalog opens on browsable content rather than the create pane. Runs at
  // most once per open and never overrides an explicit user navigation.
  React.useEffect(() => {
    if (!open || didInitRef.current || userHasSelected) return;
    if (preferSection !== "teams") {
      didInitRef.current = true;
      return;
    }
    if (teamsLoading) return;
    didInitRef.current = true;
    if (firstTeamKey) setSelection(firstTeamKey);
  }, [open, preferSection, teamsLoading, firstTeamKey, userHasSelected]);

  // Drop a selected catalog item that a live refresh retracted so the detail
  // pane never points at something that no longer exists.
  React.useEffect(() => {
    if (selection.startsWith("p:")) {
      const id = selection.slice(2);
      if (!personas.some((p) => p.id === id)) setSelection("create");
    } else if (selection.startsWith("t:")) {
      const key = selection.slice(2);
      if (!teams.some((t) => teamSelectionKey(t) === key))
        setSelection("create");
    }
  }, [personas, teams, selection]);

  useFeedbackToasts(feedbackNoticeMessage, feedbackErrorMessage);

  // Resolve current selection.
  const selectedPersona = selection.startsWith("p:")
    ? (personas.find((p) => p.id === selection.slice(2)) ?? null)
    : null;
  const selectedTeamKey = selection.startsWith("t:")
    ? selection.slice(2)
    : null;
  const selectedTeam = selectedTeamKey
    ? (teams.find((t) => teamSelectionKey(t) === selectedTeamKey) ?? null)
    : null;

  const isCreateSelected = selection === "create";
  const isImportSelected = selection === "import";

  const selectedPersonaIsActive = selectedPersona
    ? isCatalogPersonaSelected(selectedPersona)
    : false;
  const selectedTeamIsAdded = selectedTeam?.localTeam != null;

  const bothEmpty =
    !personasLoading &&
    !teamsLoading &&
    personas.length === 0 &&
    teams.length === 0;
  const noError = !personasError && !teamsError;

  const handleCreateDirtyChange = React.useCallback((dirty: boolean) => {
    createDirtyRef.current = dirty;
  }, []);

  function requestSelection(nextSelection: string) {
    if (
      isCreateSelected &&
      nextSelection !== "create" &&
      createDirtyRef.current
    ) {
      setPendingNavigation({ type: "selection", selection: nextSelection });
      return;
    }
    setSelection(nextSelection);
  }

  function requestClose() {
    if (isCreateSelected && createDirtyRef.current) {
      setPendingNavigation({ type: "close" });
      return;
    }
    onOpenChange(false);
  }

  function discardChangesAndNavigate() {
    const navigation = pendingNavigation;
    createDirtyRef.current = false;
    setPendingNavigation(null);
    if (navigation?.type === "selection") {
      setSelection(navigation.selection);
    } else if (navigation?.type === "close") {
      onOpenChange(false);
    }
  }

  function handleUseAgent() {
    if (!selectedPersona || selectedPersonaIsActive) return;
    onClearFeedback();
    onSelectPersona(selectedPersona, true);
  }

  function handleAddTeam() {
    if (!selectedTeam || selectedTeamIsAdded) return;
    onAddTeam(selectedTeam);
  }

  React.useEffect(() => {
    if (!isImportSelected) {
      dragDepthRef.current = 0;
      setIsDragOver(false);
    }
  }, [isImportSelected]);

  function hasFiles(event: React.DragEvent) {
    return event.dataTransfer.types.includes("Files");
  }

  function isAgentSnapshot(file: File) {
    const lowerName = file.name.toLowerCase();
    return (
      lowerName.endsWith(".agent.json") || lowerName.endsWith(".agent.png")
    );
  }

  async function importFile(file: File) {
    if (!isAgentSnapshot(file)) return;
    const buffer = await file.arrayBuffer();
    onOpenChange(false);
    onImportFile(Array.from(new Uint8Array(buffer)), file.name);
  }

  return (
    <>
      <Dialog
        onOpenChange={(nextOpen) => {
          if (!nextOpen && personasPending) return;
          if (!nextOpen) {
            requestClose();
            return;
          }
          onOpenChange(true);
        }}
        open={open}
      >
        <ChooserDialogContent
          className="h-[42rem] max-w-4xl"
          contentClassName="flex min-h-0 min-w-0 flex-1 p-0"
          data-testid="community-catalog-dialog"
          description="Create, discover, and import agents and teams."
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
          title="Add agent"
          onDragEnter={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            dragDepthRef.current += 1;
            setIsDragOver(true);
          }}
          onDragLeave={(event) => {
            if (!isImportSelected) return;
            event.preventDefault();
            dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
            if (dragDepthRef.current === 0) setIsDragOver(false);
          }}
          onDragOver={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            event.dataTransfer.dropEffect = "copy";
          }}
          onDrop={(event) => {
            if (!isImportSelected || !hasFiles(event)) return;
            event.preventDefault();
            dragDepthRef.current = 0;
            setIsDragOver(false);
            const file = event.dataTransfer.files[0];
            if (file) void importFile(file);
          }}
        >
          <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden bg-sidebar sm:flex-row">
            {isImportSelected && isDragOver ? (
              <div
                className="pointer-events-none absolute inset-2 z-50 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary bg-primary/10 backdrop-blur-sm"
                data-testid="agent-catalog-drop-overlay"
              >
                <p className="rounded-full bg-background/90 px-4 py-2 text-sm font-medium text-primary shadow-sm">
                  Drop .agent.json or .agent.png to import
                </p>
              </div>
            ) : null}

            {/* Sidebar navigation + catalog lists */}
            <div className="flex max-h-56 min-h-0 flex-col sm:max-h-none sm:w-56">
              <div
                className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
                data-testid="community-catalog-dialog-scroll-area"
              >
                <div className="space-y-1">
                  <CatalogNavigationButton
                    icon={<Plus className="h-4 w-4" />}
                    isCurrent={isCreateSelected}
                    label="Create agent"
                    onClick={() => requestSelection("create")}
                    testId="agent-catalog-create"
                  />
                  <CatalogNavigationButton
                    icon={<Upload className="h-4 w-4" />}
                    isCurrent={isImportSelected}
                    label="Import"
                    onClick={() => requestSelection("import")}
                    testId="agent-catalog-import"
                  />
                </div>

                <div className="my-3 border-t border-sidebar-border/60" />

                {personasLoading ? <CatalogListSkeleton /> : null}

                {!personasLoading && personas.length > 0 ? (
                  <div className="mb-1">
                    <p className="px-4 pb-1 pt-2 text-2xs font-semibold uppercase tracking-wider text-sidebar-foreground/60">
                      Agents
                    </p>
                    <div className="space-y-1">
                      {personas.map((persona) => {
                        const key = personaKey(persona);
                        const isCurrent = key === selection;
                        const description = effectiveAgentDescription(persona);
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
                              requestSelection(key);
                            }}
                            type="button"
                          >
                            <ProfileAvatar
                              avatarUrl={persona.avatarUrl}
                              className="h-6 w-6 text-3xs"
                              label={persona.displayName}
                              untrusted
                            />
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-sm font-medium">
                                {persona.displayName}
                              </span>
                              {description ? (
                                <span
                                  className="line-clamp-2 block break-all text-xs leading-4 text-sidebar-foreground/60"
                                  data-testid={`community-catalog-agent-description-${persona.id}`}
                                >
                                  {description}
                                </span>
                              ) : null}
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
                        const key = teamKey(team);
                        const isCurrent = key === selection;
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
                              requestSelection(key);
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

                {bothEmpty && noError ? <CatalogEmptyState /> : null}
              </div>
            </div>

            {/* Detail pane */}
            <div className="relative z-10 mb-3 ml-px mr-3 flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-background shadow-[-1px_0_0_0_hsl(var(--sidebar-border)/0.45)]">
              {isCreateSelected
                ? createContent({
                    onDirtyChange: handleCreateDirtyChange,
                    onRequestClose: requestClose,
                  })
                : null}

              {isImportSelected ? (
                <ImportAgentPane
                  onImport={() => fileInputRef.current?.click()}
                />
              ) : null}

              {selectedPersona || selectedTeam ? (
                <>
                  <div
                    className="min-h-0 min-w-0 max-w-full flex-1 overflow-x-hidden overflow-y-auto px-5 pb-24 pt-5"
                    data-testid="community-catalog-detail-pane"
                  >
                    {selectedPersona ? (
                      <PersonaCatalogDetail persona={selectedPersona} />
                    ) : null}
                    {selectedTeam ? (
                      <TeamCatalogDetail team={selectedTeam} />
                    ) : null}
                  </div>

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
                        disabled={selectedPersonaIsActive || personasPending}
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
                            : `Add ${selectedTeam.name} from Community Catalog`
                        }
                        className="pointer-events-auto"
                        data-testid="community-catalog-add-team"
                        disabled={selectedTeamIsAdded || teamsAdding}
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
                </>
              ) : null}

              {/* Per-section errors — only blank the section that failed */}
              {personasError ? (
                <p className="m-5 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {personasError.message}
                </p>
              ) : null}
              {teamsError ? (
                <p className="m-5 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {teamsError.message}
                </p>
              ) : null}
            </div>
          </div>

          <input
            accept=".agent.json,.agent.png"
            className="hidden"
            data-testid="agent-catalog-import-input"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) void importFile(file);
              event.target.value = "";
            }}
            ref={fileInputRef}
            type="file"
          />
        </ChooserDialogContent>
      </Dialog>

      <AlertDialog
        onOpenChange={(nextOpen) => {
          if (!nextOpen) setPendingNavigation(null);
        }}
        open={pendingNavigation !== null}
      >
        <AlertDialogContent data-testid="discard-create-agent-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle>Discard agent changes?</AlertDialogTitle>
            <AlertDialogDescription>
              Your changes to this agent will be lost.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button onClick={discardChangesAndNavigate} variant="destructive">
                Discard changes
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

// ── Navigation button ─────────────────────────────────────────────────────────

function CatalogNavigationButton({
  icon,
  isCurrent,
  label,
  onClick,
  testId,
}: {
  icon: React.ReactNode;
  isCurrent: boolean;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      aria-current={isCurrent ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-lg px-4 py-2 text-left text-sm font-medium transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
        isCurrent
          ? "bg-sidebar-active text-sidebar-active-foreground"
          : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      )}
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

// ── Import pane ───────────────────────────────────────────────────────────────

function ImportAgentPane({ onImport }: { onImport: () => void }) {
  return (
    <button
      className="m-5 flex min-h-0 flex-1 flex-col items-center justify-center rounded-xl border-2 border-dashed border-border bg-muted/20 px-8 py-12 text-center transition-colors hover:border-primary/60 hover:bg-primary/5 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid="agent-catalog-import-dropzone"
      onClick={onImport}
      type="button"
    >
      <span className="flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Upload className="h-6 w-6" />
      </span>
      <span className="mt-4 text-base font-semibold">Import an agent</span>
      <span className="mt-2 max-w-sm text-sm text-muted-foreground">
        Drop an .agent.json or .agent.png file anywhere in this window, or
        choose a file.
      </span>
      <span className="mt-5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground">
        Choose file
      </span>
    </button>
  );
}

// ── Empty state ───────────────────────────────────────────────────────────────

function CatalogEmptyState() {
  return (
    <div
      className="flex flex-col items-center px-2 py-6 text-center"
      data-testid="community-catalog-empty-state"
    >
      <img
        alt=""
        aria-hidden="true"
        className="h-24 w-24 dark:opacity-60"
        data-testid="community-catalog-empty-artwork"
        src={agentOutlineUrl}
      />
      <p className="mt-3 text-sm font-semibold text-sidebar-foreground">
        Nothing shared yet
      </p>
      <p className="mt-1 text-xs text-sidebar-foreground/60">
        Shared agents and teams will appear here.
      </p>
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

// ── Persona detail ────────────────────────────────────────────────────────────

/**
 * Security review surface for instructions that will execute verbatim.
 *
 * Do not replace this with the chat Markdown renderer: Markdown intentionally
 * hides spoiler bodies, link destinations, and image sources, so the reviewed
 * text would differ from the system prompt sent to the agent.
 */
export function AgentInstructionReview({
  instructions,
}: {
  instructions: string;
}) {
  return (
    <pre
      className="mt-3 w-full min-w-0 max-w-full whitespace-pre-wrap break-words font-sans text-sm leading-6 text-muted-foreground"
      data-testid="persona-catalog-exact-instructions"
    >
      {instructions || "No instructions included."}
    </pre>
  );
}

function PersonaCatalogDetail({ persona }: { persona: AgentPersona }) {
  const description = effectiveAgentDescription(persona);
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
          untrusted
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

      {description ? (
        <p
          className="min-w-0 max-w-prose break-all text-sm leading-6 text-muted-foreground"
          data-testid="persona-catalog-description"
        >
          {description}
        </p>
      ) : null}

      <AgentDefinitionMetadata
        isBuiltIn={persona.isBuiltIn}
        model={persona.model}
        provider={persona.provider}
        runtime={persona.runtime}
      />

      <div className="min-w-0 max-w-full pt-3">
        <p className="text-base font-semibold text-foreground">
          Agent instructions
        </p>
        <AgentInstructionReview instructions={persona.systemPrompt} />
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
          <AgentInstructionReview instructions={team.instructions ?? ""} />
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
          untrusted
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
              Agent instructions
            </p>
            {member.systemPrompt.trim().length > 0 ? (
              <AgentInstructionReview instructions={member.systemPrompt} />
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
