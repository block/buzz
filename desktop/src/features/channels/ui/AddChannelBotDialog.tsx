import { AlertTriangle } from "lucide-react";
import * as React from "react";

import {
  useCreateChannelManagedAgentsMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useRelayAgentsQuery,
  useTeamsQuery,
  type CreateChannelManagedAgentResult,
} from "@/features/agents/hooks";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { getRelayAgentPickerCandidates } from "@/features/agents/lib/relayAgentPickerEligibility";
import { getUsableTeams } from "@/features/agents/lib/teamPersonas";
import { AddChannelBotPersonasSection } from "@/features/channels/ui/AddChannelBotPersonasSection";
import { AddChannelBotRelayAgentsSection } from "@/features/channels/ui/AddChannelBotRelayAgentsSection";
import { AddChannelBotTeamsSection } from "@/features/channels/ui/AddChannelBotTeamsSection";
import {
  useAddChannelMembersMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { useInChannelPersonaIds } from "@/features/channels/ui/useInChannelPersonaIds";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { AcpRuntime, RelayAgent } from "@/shared/api/types";
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
  const identityQuery = useIdentityQuery();
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: open });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: open });
  const teamsQuery = useTeamsQuery();
  const channelMembersQuery = useChannelMembersQuery(channelId, open);
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
  const [selectedRelayAgentPubkeys, setSelectedRelayAgentPubkeys] =
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
  const localManagedAgentPubkeys = React.useMemo(
    () =>
      new Set(
        (managedAgentsQuery.data ?? []).map((agent) =>
          normalizePubkey(agent.pubkey),
        ),
      ),
    [managedAgentsQuery.data],
  );
  const relayAgentProfilesQuery = useUsersBatchQuery(
    (relayAgentsQuery.data ?? []).map((agent) => agent.pubkey),
    { enabled: open && (relayAgentsQuery.data?.length ?? 0) > 0 },
  );
  const relaySourceError =
    identityQuery.error ??
    managedAgentsQuery.error ??
    relayAgentsQuery.error ??
    relayAgentProfilesQuery.error ??
    channelMembersQuery.error;
  const relaySourcesReady =
    relaySourceError === null &&
    identityQuery.data !== undefined &&
    managedAgentsQuery.data !== undefined &&
    relayAgentsQuery.data !== undefined &&
    ((relayAgentsQuery.data?.length ?? 0) === 0 ||
      relayAgentProfilesQuery.data !== undefined) &&
    channelMembersQuery.data !== undefined;
  const relayAgents = React.useMemo(
    () =>
      !relaySourcesReady
        ? []
        : getRelayAgentPickerCandidates({
            currentPubkey: identityQuery.data?.pubkey,
            managedAgentPubkeys: localManagedAgentPubkeys,
            memberPubkeys: (channelMembersQuery.data ?? []).map(
              (member) => member.pubkey,
            ),
            profiles: relayAgentProfilesQuery.data?.profiles ?? {},
            relayAgents: relayAgentsQuery.data ?? [],
          }).map((candidate) => candidate.agent),
    [
      channelMembersQuery.data,
      identityQuery.data?.pubkey,
      localManagedAgentPubkeys,
      relayAgentProfilesQuery.data,
      relayAgentsQuery.data,
      relaySourcesReady,
    ],
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

  const selectedRelayAgents = React.useMemo(
    () =>
      relayAgents.filter((agent) =>
        selectedRelayAgentPubkeys.includes(normalizePubkey(agent.pubkey)),
      ),
    [relayAgents, selectedRelayAgentPubkeys],
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
    const eligiblePubkeys = new Set(
      relayAgents.map((agent) => normalizePubkey(agent.pubkey)),
    );
    setSelectedRelayAgentPubkeys((current) =>
      current.filter(
        (pubkey) =>
          eligiblePubkeys.has(pubkey) && !inChannelPubkeys.has(pubkey),
      ),
    );
  }, [inChannelPubkeys, relayAgents]);

  function reset() {
    setSelectedPersonaIds([]);
    setSelectedRelayAgentPubkeys([]);
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
    if (selectedPersonas.length === 0 && selectedRelayAgents.length === 0)
      return;
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

    let relayResult: {
      added: string[];
      errors: { pubkey: string; error: string }[];
    } = { added: [], errors: [] };
    let result: Awaited<ReturnType<typeof createBotsMutation.mutateAsync>> = {
      successes: [],
      failures: [],
    };
    let relayTransportError: string | null = null;
    let managedTransportError: string | null = null;

    if (selectedRelayAgents.length > 0) {
      try {
        relayResult = await addMembersMutation.mutateAsync({
          pubkeys: selectedRelayAgents.map((agent) => agent.pubkey),
          role: "bot",
        });
      } catch (error) {
        relayTransportError =
          error instanceof Error ? error.message : "Failed to add relay agents";
      }
    }
    if (inputs.length > 0) {
      try {
        result = await createBotsMutation.mutateAsync(inputs);
      } catch (error) {
        managedTransportError =
          error instanceof Error
            ? error.message
            : "Failed to add managed agents";
      }
    }

    const relayAgentsByPubkey = new Map<string, RelayAgent>(
      selectedRelayAgents.map((agent) => [
        normalizePubkey(agent.pubkey),
        agent,
      ]),
    );
    const relayFailures = relayResult.errors.map(({ pubkey, error }) => ({
      name: relayAgentsByPubkey.get(normalizePubkey(pubkey))?.name ?? pubkey,
      error,
    }));
    const failures = [...result.failures, ...relayFailures];
    if (
      failures.length === 0 &&
      relayTransportError === null &&
      managedTransportError === null
    ) {
      if (result.successes[0]) onAdded?.(result.successes[0]);
      handleOpenChange(false);
      return;
    }

    const failedPersonaIds = new Set(
      managedTransportError
        ? selectedPersonaIds
        : result.failures
            .map((failure) => failure.personaId)
            .filter((personaId): personaId is string => Boolean(personaId)),
    );
    setSelectedPersonaIds((current) =>
      current.filter((personaId) => failedPersonaIds.has(personaId)),
    );
    const failedRelayPubkeys = new Set(
      relayTransportError
        ? selectedRelayAgentPubkeys
        : relayResult.errors.map(({ pubkey }) => normalizePubkey(pubkey)),
    );
    setSelectedRelayAgentPubkeys((current) =>
      current.filter((pubkey) => failedRelayPubkeys.has(pubkey)),
    );
    const successCount = result.successes.length + relayResult.added.length;
    if (successCount > 0) {
      setSubmissionNotice(
        `Added ${successCount} ${formatAgentCountLabel(successCount)}.`,
      );
    }
    setSubmissionError(
      [
        failures.length > 0 ? formatBatchFailureSummary(failures) : null,
        relayTransportError,
        managedTransportError,
      ]
        .filter((message): message is string => Boolean(message))
        .join("; "),
    );
  }

  const canSubmit =
    (selectedPersonas.length > 0 || selectedRelayAgents.length > 0) &&
    (selectedPersonas.length === 0 || providers.length > 0) &&
    !providersLoading &&
    !createBotsMutation.isPending &&
    !addMembersMutation.isPending;
  const selectionCount = selectedPersonas.length + selectedRelayAgents.length;
  const isSubmitting =
    createBotsMutation.isPending || addMembersMutation.isPending;
  const addButtonLabel = isSubmitting
    ? selectionCount > 1
      ? `Adding ${selectionCount}…`
      : "Adding…"
    : selectionCount > 1
      ? `Add ${selectionCount} agents`
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
          canToggleSelections={!isSubmitting}
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

        <AddChannelBotRelayAgentsSection
          canToggleSelections={!isSubmitting}
          inChannelPubkeys={inChannelPubkeys}
          isLoading={!relaySourcesReady && relaySourceError === null}
          onToggleAgent={(pubkey) => {
            setSelectedRelayAgentPubkeys((current) =>
              toggleValue(current, pubkey),
            );
            setSubmissionNotice(null);
            setSubmissionError(null);
          }}
          profiles={relayAgentProfilesQuery.data?.profiles ?? {}}
          relayAgents={relayAgents}
          selectedPubkeys={selectedRelayAgentPubkeys}
        />

        {teams.length > 0 ? (
          <AddChannelBotTeamsSection
            canToggleSelections={!isSubmitting}
            inChannelPersonaIds={inChannelPersonaIds}
            isLoading={teamsQuery.isLoading}
            onToggleTeam={handleToggleTeam}
            personas={personas}
            selectedPersonaIds={selectedPersonaIds}
            teams={teams}
          />
        ) : null}

        {providers.length === 0 &&
        selectedPersonas.length > 0 &&
        !providersLoading ? (
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
        {relaySourceError instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            Relay agents are unavailable: {relaySourceError.message}
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
        ) : createBotsMutation.error instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {createBotsMutation.error.message}
          </p>
        ) : addMembersMutation.error instanceof Error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {addMembersMutation.error.message}
          </p>
        ) : null}
      </ChooserDialogContent>
    </Dialog>
  );
}
