import * as React from "react";
import { Upload } from "lucide-react";

import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  AgentPersona,
  CreateTeamInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { personaCatalogCopy } from "./personaLibraryCopy";
import { RemoveMembersConfirmDialog } from "./RemoveMembersConfirmDialog";
import {
  copySelectedPersonaIds,
  countMissingPersonaIds,
  filterAvailablePersonaIds,
  orderPersonasByInitiallySelected,
} from "./teamDialogSelection";

const TEAM_FORM_ID = "team-form";
const TEAM_ROW_INSET_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:bottom-0 after:left-[3.75rem] after:right-0 after:h-px after:bg-border/60 after:content-[''] last:after:hidden";

type TeamDialogProps = {
  open: boolean;
  title: string;
  description: string;
  submitLabel: string;
  initialValues: CreateTeamInput | UpdateTeamInput | null;
  personas: AgentPersona[];
  error: Error | null;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onImport?: () => void;
  onSubmit: (input: CreateTeamInput | UpdateTeamInput) => Promise<void>;
  onDeleteRemovedPersonas?: (personaIds: string[]) => Promise<void>;
};

export function TeamDialog({
  open,
  title,
  description,
  submitLabel,
  initialValues,
  personas,
  error,
  isPending,
  onOpenChange,
  onImport,
  onSubmit,
  onDeleteRemovedPersonas,
}: TeamDialogProps) {
  const [name, setName] = React.useState("");
  const [teamDescription, setTeamDescription] = React.useState("");
  const [instructions, setInstructions] = React.useState("");
  const [selectedPersonaIds, setSelectedPersonaIds] = React.useState<string[]>(
    [],
  );
  const [
    initialSelectedPersonaIdsForSort,
    setInitialSelectedPersonaIdsForSort,
  ] = React.useState<string[]>([]);
  const [confirmRemovalOpen, setConfirmRemovalOpen] = React.useState(false);
  const isEditMode = Boolean(initialValues && "id" in initialValues);
  const missingInitialPersonaCount = React.useMemo(() => {
    if (!initialValues) {
      return 0;
    }

    return countMissingPersonaIds(initialValues.personaIds, personas);
  }, [initialValues, personas]);

  React.useEffect(() => {
    if (!open || !initialValues) {
      return;
    }

    setName(initialValues.name);
    setTeamDescription(initialValues.description ?? "");
    setInstructions(initialValues.instructions ?? "");
    setSelectedPersonaIds(copySelectedPersonaIds(initialValues.personaIds));
    setInitialSelectedPersonaIdsForSort(
      copySelectedPersonaIds(initialValues.personaIds),
    );
  }, [initialValues, open]);

  function handleOpenChange(next: boolean) {
    if (!next) {
      setName("");
      setTeamDescription("");
      setInstructions("");
      setSelectedPersonaIds([]);
      setInitialSelectedPersonaIdsForSort([]);
      setConfirmRemovalOpen(false);
    }

    onOpenChange(next);
  }

  function togglePersona(personaId: string) {
    setSelectedPersonaIds((current) =>
      current.includes(personaId)
        ? current.filter((id) => id !== personaId)
        : [...current, personaId],
    );
  }

  const removedPersonaIds = React.useMemo(() => {
    if (!isEditMode || !initialValues || !("id" in initialValues)) return [];
    const currentSet = new Set(selectedPersonaIds);
    return initialValues.personaIds.filter(
      (id) => !currentSet.has(id) && personas.some((p) => p.id === id),
    );
  }, [isEditMode, initialValues, selectedPersonaIds, personas]);

  const removedPersonaNames = React.useMemo(
    () =>
      removedPersonaIds
        .map((id) => personas.find((p) => p.id === id)?.displayName)
        .filter(Boolean),
    [removedPersonaIds, personas],
  );

  function buildSubmitInput(): CreateTeamInput | UpdateTeamInput {
    const baseInput = {
      name,
      description: teamDescription.trim() || undefined,
      instructions: instructions.trim() || undefined,
      personaIds: filterAvailablePersonaIds(selectedPersonaIds, personas),
    };

    if (initialValues && "id" in initialValues) {
      return {
        id: initialValues.id,
        ...baseInput,
      };
    }
    return {
      ...baseInput,
      description: teamDescription.trim() || undefined,
      instructions: instructions.trim() || undefined,
    };
  }

  async function handleSubmit() {
    if (!initialValues) return;

    if (removedPersonaIds.length > 0 && isEditMode && onDeleteRemovedPersonas) {
      setConfirmRemovalOpen(true);
      return;
    }

    await onSubmit(buildSubmitInput());
  }

  async function handleSubmitKeepAgents() {
    setConfirmRemovalOpen(false);
    await onSubmit(buildSubmitInput());
  }

  async function handleSubmitDeleteAgents() {
    setConfirmRemovalOpen(false);
    await onSubmit(buildSubmitInput());
    if (onDeleteRemovedPersonas && removedPersonaIds.length > 0) {
      await onDeleteRemovedPersonas(removedPersonaIds);
    }
  }

  const orderedPersonas = React.useMemo(
    () =>
      orderPersonasByInitiallySelected(
        personas,
        initialSelectedPersonaIdsForSort,
      ),
    [initialSelectedPersonaIdsForSort, personas],
  );

  return (
    <>
      <Dialog
        onOpenChange={(nextOpen) => {
          if (!nextOpen && isPending) return;
          handleOpenChange(nextOpen);
        }}
        open={open}
      >
        <ChooserDialogContent
          className="max-w-xl"
          contentClassName="pt-3"
          data-testid="team-dialog"
          description={description}
          footer={
            <div
              className={cn(
                "flex w-full items-center gap-3",
                onImport ? "justify-between" : "justify-end",
              )}
            >
              {onImport ? (
                <Button
                  data-testid="team-import"
                  disabled={isPending}
                  onClick={onImport}
                  type="button"
                  variant="outline"
                >
                  <Upload className="h-4 w-4" />
                  Import
                </Button>
              ) : null}
              <Button
                data-testid="team-submit"
                disabled={
                  name.trim().length === 0 ||
                  selectedPersonaIds.length === 0 ||
                  isPending
                }
                form={TEAM_FORM_ID}
                type="submit"
              >
                {isPending ? "Saving..." : submitLabel}
              </Button>
            </div>
          }
          footerClassName="border-t-0 pt-0"
          headerClassName="pb-2"
          title={title}
        >
          <form
            className="space-y-5"
            id={TEAM_FORM_ID}
            onSubmit={(event) => {
              event.preventDefault();
              void handleSubmit();
            }}
          >
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="team-name"
              >
                Name
              </label>
              <div
                className={cn(
                  "flex min-h-11 items-center px-3",
                  CHANNEL_FORM_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoComplete="off"
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    CHANNEL_FORM_FIELD_CONTROL_CLASS,
                  )}
                  disabled={isPending}
                  id="team-name"
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Enter a team name."
                  spellCheck={false}
                  value={name}
                />
              </div>
            </div>

            {isEditMode ? (
              <>
                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="team-description"
                  >
                    Description
                  </label>
                  <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
                    <Textarea
                      className={cn(
                        "min-h-20 resize-none px-3 py-3 leading-5",
                        CHANNEL_FORM_FIELD_CONTROL_CLASS,
                      )}
                      disabled={isPending}
                      id="team-description"
                      onChange={(event) =>
                        setTeamDescription(event.target.value)
                      }
                      placeholder="Optional description for this team."
                      value={teamDescription}
                    />
                  </div>
                </div>

                <div className="space-y-1.5">
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="team-instructions"
                  >
                    Team Instructions
                  </label>
                  <div className={CHANNEL_FORM_FIELD_SHELL_CLASS}>
                    <Textarea
                      className={cn(
                        "min-h-24 resize-none px-3 py-3 leading-5",
                        CHANNEL_FORM_FIELD_CONTROL_CLASS,
                      )}
                      disabled={isPending}
                      id="team-instructions"
                      onChange={(event) => setInstructions(event.target.value)}
                      placeholder="Optional instructions applied to every deployed team member."
                      value={instructions}
                    />
                  </div>
                </div>
              </>
            ) : null}

            <div className="space-y-2">
              <span className="text-sm font-medium text-foreground">
                Agents
              </span>
              <p className="text-xs text-muted-foreground">
                Select the agents to include in this team.
              </p>
              {missingInitialPersonaCount > 0 ? (
                <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  This team references {missingInitialPersonaCount} agent
                  {missingInitialPersonaCount === 1 ? "" : "s"} that{" "}
                  {missingInitialPersonaCount === 1 ? "is" : "are"} no longer in
                  My Agents. Save to remove them, or add them back to My Agents
                  first.
                </p>
              ) : null}
              {personas.length === 0 ? (
                <p
                  className={cn(
                    "py-4 text-center text-sm text-muted-foreground",
                    CHANNEL_FORM_FIELD_SHELL_CLASS,
                  )}
                >
                  {personaCatalogCopy.teamEmptyState}
                </p>
              ) : (
                <div
                  className="h-[min(50vh,24rem)] overflow-y-auto rounded-xl border border-border/70 bg-background/70"
                  data-testid="team-agent-picker"
                >
                  {orderedPersonas.map((persona) => {
                    const isSelected = selectedPersonaIds.includes(persona.id);

                    return (
                      <div
                        className={cn(
                          "group/add-result relative isolate flex min-h-14 w-full items-center gap-3 px-4 py-3.5 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-within:bg-muted/40",
                          TEAM_ROW_INSET_DIVIDER_CLASS,
                          isSelected && "bg-muted/40",
                        )}
                        data-testid={`team-agent-row-${persona.id}`}
                        key={persona.id}
                      >
                        <button
                          aria-label={`${isSelected ? "Remove" : "Add"} ${persona.displayName}`}
                          className="absolute inset-0 z-0 cursor-pointer focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                          disabled={isPending}
                          onClick={() => {
                            if (!isPending) {
                              togglePersona(persona.id);
                            }
                          }}
                          type="button"
                        />
                        <ProfileAvatar
                          avatarUrl={persona.avatarUrl}
                          className="pointer-events-none relative z-10 h-8 w-8 text-xs shadow-none"
                          label={persona.displayName}
                        />
                        <div className="pointer-events-none relative z-10 min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium tracking-tight">
                            {persona.displayName}
                          </span>
                        </div>
                        <Button
                          aria-label={isSelected ? "Remove" : "Add"}
                          className="relative z-20 shrink-0"
                          disabled={isPending}
                          onClick={(event) => {
                            event.stopPropagation();
                            togglePersona(persona.id);
                          }}
                          size="sm"
                          type="button"
                          variant={isSelected ? "outline" : "default"}
                        >
                          {isSelected ? "Remove" : "Add"}
                        </Button>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {error ? (
              <p className="text-sm text-destructive">{error.message}</p>
            ) : null}
          </form>
        </ChooserDialogContent>
      </Dialog>

      <RemoveMembersConfirmDialog
        open={confirmRemovalOpen}
        onOpenChange={setConfirmRemovalOpen}
        isPending={isPending}
        memberNames={removedPersonaNames as string[]}
        onKeepAgents={() => void handleSubmitKeepAgents()}
        onRemoveAgents={() => void handleSubmitDeleteAgents()}
      />
    </>
  );
}
