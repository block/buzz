import { AlertTriangle } from "lucide-react";
import * as React from "react";

import {
  useCreateChannelManagedAgentsMutation,
  usePersonasQuery,
  useRelayAgentsQuery,
  useTeamsQuery,
  type CreateChannelManagedAgentResult,
} from "@/features/agents/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { getUsableTeams } from "@/features/agents/lib/teamPersonas";
import { AddChannelBotPersonasSection } from "@/features/channels/ui/AddChannelBotPersonasSection";
import { AddChannelBotTeamsSection } from "@/features/channels/ui/AddChannelBotTeamsSection";
import { AddChannelExistingAgentsSection } from "@/features/channels/ui/AddChannelExistingAgentsSection";
import { useInChannelPersonaIds } from "@/features/channels/ui/useInChannelPersonaIds";
import {
  useAddChannelMembersMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import type { AcpRuntime } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
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
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: open });
  const teamsQuery = useTeamsQuery();
  const channelMembersQuery = useChannelMembersQuery(
    channelId,
    open && channelId !== null,
  );
  const inChannelPersonaIds = useInChannelPersonaIds(
    channelId,
    open && channelId !== null,
  );
  const createBotsMutation = useCreateChannelManagedAgentsMutation(channelId);
  const addMembersMutation = useAddChannelMembersMutation(channelId);
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
  const [selectedExistingAgentPubkeys, setSelectedExistingAgentPubkeys] =
    React.useState<string[]>([]);
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
  const relayAgents = React.useMemo(
    () =>
      [...(relayAgentsQuery.data ?? [])].sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    [relayAgentsQuery.data],
  );
  const inChannelPubkeys = React.useMemo(
    () =>
      new Set(
        (channelMembersQuery.data ?? []).map((member) =>
          normalizePubkey(member.pubkey),
        ),
      ),
    [channelMembersQuery.data],
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

  React.useEffect(() => {
    const addablePubkeys = new Set(
      relayAgents
        .map((agent) => normalizePubkey(agent.pubkey))
        .filter((pubkey) => !inChannelPubkeys.has(pubkey)),
    );
    setSelectedExistingAgentPubkeys((current) =>
      current.filter((pubkey) => addablePubkeys.has(normalizePubkey(pubkey))),
    );
  }, [inChannelPubkeys, relayAgents]);

  function reset() {
    setSelectedPersonaIds([]);
    setSelectedExistingAgentPubkeys([]);
    setSubmissionNotice(null);
    setSubmissionError(null);
    createBotsMutation.reset();
    addMembersMutation.reset();
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
    if (
      selectedExistingAgentPubkeys.length === 0 &&
      selectedPersonas.length === 0
    ) {
      return;
    }
    if (selectedPersonas.length > 0 && providers.length === 0) return;

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

    const failures: Array<{ name: string; error: string }> = [];
    let addedCount = 0;

    if (selectedExistingAgentPubkeys.length > 0) {
      try {
        const result = await addMembersMutation.mutateAsync({
          pubkeys: selectedExistingAgentPubkeys,
          role: "bot",
        });
        addedCount += result.added.length;
        const failedPubkeys = new Set(
          result.errors.map((failure) => normalizePubkey(failure.pubkey)),
        );
        setSelectedExistingAgentPubkeys((current) =>
          current.filter((pubkey) =>
            failedPubkeys.has(normalizePubkey(pubkey)),
          ),
        );
        for (const failure of result.errors) {
          failures.push({
            name:
              relayAgents.find(
                (agent) =>
                  normalizePubkey(agent.pubkey) ===
                  normalizePubkey(failure.pubkey),
              )?.name ?? "agent",
            error: failure.error,
          });
        }
      } catch (error) {
        failures.push({
          name: "existing agents",
          error:
            error instanceof Error ? error.message : "Could not add agents.",
        });
      }
    }

    if (inputs.length > 0) {
      try {
        const result = await createBotsMutation.mutateAsync(inputs);
        addedCount += result.successes.length;
        if (result.successes[0]) onAdded?.(result.successes[0]);
        const failedPersonaIds = new Set(
          result.failures
            .map((failure) => failure.personaId)
            .filter((personaId): personaId is string => Boolean(personaId)),
        );
        setSelectedPersonaIds((current) =>
          current.filter((personaId) => failedPersonaIds.has(personaId)),
        );
        failures.push(...result.failures);
      } catch (error) {
        failures.push({
          name: "new agents",
          error:
            error instanceof Error ? error.message : "Could not create agents.",
        });
      }
    }

    if (failures.length === 0) {
      handleOpenChange(false);
      return;
    }
    if (addedCount > 0) {
      setSubmissionNotice(
        `Added ${addedCount} ${formatAgentCountLabel(addedCount)}.`,
      );
    }
    setSubmissionError(formatBatchFailureSummary(failures));
  }

  const totalSelected =
    selectedExistingAgentPubkeys.length + selectedPersonas.length;
  const isPending =
    createBotsMutation.isPending || addMembersMutation.isPending;
  const canSubmit =
    totalSelected > 0 &&
    (selectedPersonas.length === 0 ||
      (providers.length > 0 && !providersLoading)) &&
    !isPending;
  const addButtonLabel = isPending
    ? totalSelected > 1
      ? `Adding ${totalSelected}…`
      : "Adding…"
    : totalSelected > 1
      ? `Add ${totalSelected} agents`
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
        <AddChannelExistingAgentsSection
          agents={relayAgents}
          canToggleSelections={!isPending}
          inChannelPubkeys={inChannelPubkeys}
          isLoading={
            relayAgentsQuery.isLoading || channelMembersQuery.isLoading
          }
          onToggleAgent={(pubkey) => {
            setSelectedExistingAgentPubkeys((current) =>
              toggleValue(current, pubkey),
            );
            setSubmissionNotice(null);
            setSubmissionError(null);
          }}
          selectedPubkeys={selectedExistingAgentPubkeys}
        />

        {providers.length > 0 || providersLoading ? (
          <AddChannelBotPersonasSection
            availableLabel="Create another agent on this computer"
            canToggleSelections={!isPending && providers.length > 0}
            inChannelPersonaIds={inChannelPersonaIds}
            isLoading={personasQuery.isLoading}
            onCreateAgent={handleCreateAgent}
            onTogglePersona={(personaId) => {
              setSelectedPersonaIds((current) =>
                toggleValue(current, personaId),
              );
              setSubmissionNotice(null);
              setSubmissionError(null);
            }}
            personas={personas}
            selectedPersonaIds={selectedPersonaIds}
          />
        ) : null}

        {teams.length > 0 && providers.length > 0 ? (
          <AddChannelBotTeamsSection
            canToggleSelections={!isPending}
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
              This computer cannot create a new agent until an agent runtime is
              installed. Existing agents can still be added.
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
        {relayAgentsQuery.error instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {relayAgentsQuery.error.message}
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
