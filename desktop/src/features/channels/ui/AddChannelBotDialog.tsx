import { AlertTriangle } from "lucide-react";
import * as React from "react";

import {
  useCreateChannelManagedAgentsMutation,
  usePersonasQuery,
  useTeamsQuery,
  type CreateChannelManagedAgentResult,
} from "@/features/agents/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { getUsableTeams } from "@/features/agents/lib/teamPersonas";
import { AddChannelBotPersonasSection } from "@/features/channels/ui/AddChannelBotPersonasSection";
import { AddChannelBotTeamsSection } from "@/features/channels/ui/AddChannelBotTeamsSection";
import { useInChannelPersonaIds } from "@/features/channels/ui/useInChannelPersonaIds";
import type { AcpRuntime } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

type AddChannelBotDialogProps = {
  channelId: string | null;
  open: boolean;
  providers: AcpRuntime[];
  providersErrorMessage?: string | null;
  providersLoading?: boolean;
  onAdded?: (result: CreateChannelManagedAgentResult) => void;
  onCreateAgent: () => void;
  onOpenChange: (open: boolean) => void;
};

function toggleValue(values: readonly string[], value: string) {
  return values.includes(value)
    ? values.filter((candidate) => candidate !== value)
    : [...values, value];
}

function formatAgentCountLabel(count: number) {
  return count === 1 ? "agent" : "agents";
}

function formatBatchFailureSummary(
  failures: ReadonlyArray<{ name: string; error: string }>,
) {
  if (failures.length === 1) {
    const [failure] = failures;
    return `Failed to add ${failure.name}: ${failure.error}`;
  }

  return failures
    .map((failure) => `${failure.name}: ${failure.error}`)
    .join("; ");
}

export function AddChannelBotDialog({
  channelId,
  open,
  providers,
  providersErrorMessage,
  providersLoading = false,
  onAdded,
  onCreateAgent,
  onOpenChange,
}: AddChannelBotDialogProps) {
  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const inChannelPersonaIds = useInChannelPersonaIds(
    channelId,
    open && channelId !== null,
  );
  const createBotsMutation = useCreateChannelManagedAgentsMutation(channelId);
  const personas = React.useMemo(
    () => getActivePersonas(personasQuery.data ?? []),
    [personasQuery.data],
  );
  const teams = React.useMemo(
    () => getUsableTeams(teamsQuery.data ?? [], personas),
    [personas, teamsQuery.data],
  );
  const [selectedPersonaIds, setSelectedPersonaIds] = React.useState<string[]>(
    [],
  );
  const [submissionNotice, setSubmissionNotice] = React.useState<string | null>(
    null,
  );
  const [submissionError, setSubmissionError] = React.useState<string | null>(
    null,
  );

  const selectedPersonas = React.useMemo(
    () => personas.filter((persona) => selectedPersonaIds.includes(persona.id)),
    [personas, selectedPersonaIds],
  );

  React.useEffect(() => {
    setSelectedPersonaIds((current) =>
      current.filter(
        (id) =>
          personas.some((persona) => persona.id === id) &&
          !inChannelPersonaIds.has(id),
      ),
    );
  }, [inChannelPersonaIds, personas]);

  function reset() {
    setSelectedPersonaIds([]);
    setSubmissionNotice(null);
    setSubmissionError(null);
    createBotsMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) reset();
    onOpenChange(next);
  }

  function handleCreateAgent() {
    handleOpenChange(false);
    onCreateAgent();
  }

  function handleToggleTeam(personaIds: string[]) {
    const addableIds = personaIds.filter(
      (personaId) => !inChannelPersonaIds.has(personaId),
    );
    setSelectedPersonaIds((current) => {
      const allSelected = addableIds.every((id) => current.includes(id));
      if (allSelected) {
        return current.filter((id) => !addableIds.includes(id));
      }
      return [...new Set([...current, ...addableIds])];
    });
    setSubmissionNotice(null);
    setSubmissionError(null);
  }

  async function handleSubmit() {
    if (providers.length === 0 || selectedPersonas.length === 0) return;

    const inputs = selectedPersonas.map((persona) => {
      const resolved = resolvePersonaRuntime(
        persona.runtime,
        providers,
        providers[0] ?? null,
        false,
      );
      return {
        runtime: resolved.runtime ?? providers[0],
        name: persona.displayName,
        personaId: persona.id,
        harnessOverride: false,
        systemPrompt: persona.systemPrompt,
        avatarUrl: persona.avatarUrl ?? undefined,
        model: persona.model ?? undefined,
        role: "bot" as const,
        backend: { type: "local" as const },
      };
    });

    setSubmissionNotice(null);
    setSubmissionError(null);

    try {
      const result = await createBotsMutation.mutateAsync(inputs);
      if (result.failures.length === 0) {
        if (result.successes[0]) onAdded?.(result.successes[0]);
        handleOpenChange(false);
        return;
      }

      const failedPersonaIds = new Set(
        result.failures
          .map((failure) => failure.personaId)
          .filter((personaId): personaId is string => Boolean(personaId)),
      );
      setSelectedPersonaIds((current) =>
        current.filter((personaId) => failedPersonaIds.has(personaId)),
      );
      if (result.successes.length > 0) {
        setSubmissionNotice(
          `Added ${result.successes.length} ${formatAgentCountLabel(
            result.successes.length,
          )}.`,
        );
      }
      setSubmissionError(formatBatchFailureSummary(result.failures));
    } catch {
      // The mutation error is rendered inline.
    }
  }

  const canSubmit =
    providers.length > 0 &&
    selectedPersonas.length > 0 &&
    !providersLoading &&
    !createBotsMutation.isPending;
  const addButtonLabel = createBotsMutation.isPending
    ? selectedPersonas.length > 1
      ? `Adding ${selectedPersonas.length}…`
      : "Adding…"
    : selectedPersonas.length > 1
      ? `Add ${selectedPersonas.length} agents`
      : "Add agent";

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-xl"
        data-testid="add-channel-bot-dialog"
        description="Choose from your agents, or create a new one."
        footer={
          <>
            <Button
              onClick={() => handleOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={!canSubmit}
              onClick={() => void handleSubmit()}
              size="sm"
              type="button"
            >
              {addButtonLabel}
            </Button>
          </>
        }
        footerClassName="justify-end gap-2"
        footerTestId="add-channel-bot-dialog-footer"
        headerTestId="add-channel-bot-dialog-header"
        scrollAreaClassName="space-y-5"
        scrollAreaTestId="add-channel-bot-dialog-scroll-area"
        title="Add agents"
      >
        <AddChannelBotPersonasSection
          canToggleSelections={!createBotsMutation.isPending}
          inChannelPersonaIds={inChannelPersonaIds}
          isLoading={personasQuery.isLoading}
          onCreateAgent={handleCreateAgent}
          onTogglePersona={(personaId) => {
            setSelectedPersonaIds((current) => toggleValue(current, personaId));
            setSubmissionNotice(null);
            setSubmissionError(null);
          }}
          personas={personas}
          selectedPersonaIds={selectedPersonaIds}
        />

        {teams.length > 0 ? (
          <AddChannelBotTeamsSection
            canToggleSelections={!createBotsMutation.isPending}
            inChannelPersonaIds={inChannelPersonaIds}
            isLoading={teamsQuery.isLoading}
            onToggleTeam={handleToggleTeam}
            personas={personas}
            selectedPersonaIds={selectedPersonaIds}
            teams={teams}
          />
        ) : null}

        {providers.length === 0 && !providersLoading ? (
          <div className="flex gap-3 rounded-lg border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              Install an agent runtime before adding an agent to this channel.
            </p>
          </div>
        ) : null}

        {providersErrorMessage ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {providersErrorMessage}
          </p>
        ) : null}
        {personasQuery.error instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {personasQuery.error.message}
          </p>
        ) : null}
        {submissionNotice ? (
          <p className="rounded-lg bg-muted px-4 py-3 text-sm text-foreground">
            {submissionNotice}
          </p>
        ) : null}
        {submissionError ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {submissionError}
          </p>
        ) : null}
        {createBotsMutation.error instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {createBotsMutation.error.message}
          </p>
        ) : null}
      </ChooserDialogContent>
    </Dialog>
  );
}
