import * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  AgentPersona,
  CreateTeamInput,
  UpdateTeamInput,
} from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
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
      return { id: initialValues.id, ...baseInput };
    }
    return baseInput;
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
      <Dialog onOpenChange={handleOpenChange} open={open}>
        <DialogContent className="max-w-2xl overflow-hidden p-0">
          <div className="flex max-h-[85vh] flex-col">
            <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
              <DialogTitle>{title}</DialogTitle>
              {description.trim().length > 0 ? (
                <DialogDescription>{description}</DialogDescription>
              ) : null}
            </DialogHeader>

            <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
              <div className="space-y-1.5">
                <label className="text-sm font-medium" htmlFor="team-name">
                  Name
                </label>
                <Input
                  autoCorrect="off"
                  disabled={isPending}
                  id="team-name"
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Engineering Squad"
                  value={name}
                />
              </div>

              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium"
                  htmlFor="team-description"
                >
                  Description
                </label>
                <Textarea
                  className="min-h-20"
                  disabled={isPending}
                  id="team-description"
                  onChange={(event) => setTeamDescription(event.target.value)}
                  placeholder="Optional description for this team."
                  value={teamDescription}
                />
              </div>

              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium"
                  htmlFor="team-instructions"
                >
                  Team Instructions
                </label>
                <Textarea
                  className="min-h-24"
                  disabled={isPending}
                  id="team-instructions"
                  onChange={(event) => setInstructions(event.target.value)}
                  placeholder="Optional instructions applied to every deployed team member."
                  value={instructions}
                />
              </div>

              <div className="space-y-2">
                <span className="text-sm font-medium">Agents</span>
                <p className="text-xs text-muted-foreground">
                  Select the agents to include in this team.
                </p>
                {missingInitialPersonaCount > 0 ? (
                  <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                    This team references {missingInitialPersonaCount} agent
                    {missingInitialPersonaCount === 1 ? "" : "s"} that{" "}
                    {missingInitialPersonaCount === 1 ? "is" : "are"} no longer
                    in My Agents. Save to remove them, or add them back to My
                    Agents first.
                  </p>
                ) : null}
                {personas.length === 0 ? (
                  <p className="py-4 text-center text-sm text-muted-foreground">
                    {personaCatalogCopy.teamEmptyState}
                  </p>
                ) : (
                  <div
                    className="max-h-60 space-y-1 overflow-y-auto rounded-lg border border-border/70 p-2"
                    role="listbox"
                    aria-label="Agents"
                    aria-multiselectable="true"
                  >
                    {orderedPersonas.map((persona) => {
                      const isSelected = selectedPersonaIds.includes(
                        persona.id,
                      );

                      return (
                        <div
                          className="flex cursor-pointer items-center gap-3 rounded-md px-2 py-1.5 transition-colors hover:bg-muted/50"
                          key={persona.id}
                          onClick={() => {
                            if (!isPending) {
                              togglePersona(persona.id);
                            }
                          }}
                          onKeyDown={(event) => {
                            if (
                              !isPending &&
                              (event.key === "Enter" || event.key === " ")
                            ) {
                              event.preventDefault();
                              togglePersona(persona.id);
                            }
                          }}
                          role="option"
                          aria-selected={isSelected}
                          tabIndex={0}
                        >
                          <Checkbox
                            checked={isSelected}
                            className="pointer-events-none"
                            disabled={isPending}
                            tabIndex={-1}
                          />
                          <ProfileAvatar
                            avatarUrl={persona.avatarUrl}
                            className="h-6 w-6 text-2xs"
                            label={persona.displayName}
                          />
                          <span className="text-sm">{persona.displayName}</span>
                          {persona.isBuiltIn ? (
                            <Badge variant="secondary">Built-in</Badge>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>

              {error ? (
                <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                  {error.message}
                </p>
              ) : null}
            </div>

            <div className="flex shrink-0 items-center justify-end gap-3 border-t border-border/60 px-6 py-4">
              <div className="flex items-center gap-2">
                <Button
                  onClick={() => handleOpenChange(false)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
                <Button
                  disabled={
                    name.trim().length === 0 ||
                    selectedPersonaIds.length === 0 ||
                    isPending
                  }
                  onClick={() => void handleSubmit()}
                  size="sm"
                  type="button"
                >
                  {isPending ? "Saving..." : submitLabel}
                </Button>
              </div>
            </div>
          </div>
        </DialogContent>
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
