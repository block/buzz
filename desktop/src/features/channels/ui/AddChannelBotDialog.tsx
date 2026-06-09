import { AlertTriangle, ChevronDown } from "lucide-react";
import * as React from "react";

import {
  useCreateChannelManagedAgentsMutation,
  usePersonasQuery,
  useTeamsQuery,
  type CreateChannelManagedAgentResult,
} from "@/features/agents/hooks";
import { useInChannelPersonaIds } from "@/features/channels/ui/useInChannelPersonaIds";
import { AddChannelBotGenericSection } from "@/features/channels/ui/AddChannelBotGenericSection";
import { AddChannelBotPersonasSection } from "@/features/channels/ui/AddChannelBotPersonasSection";
import { AddChannelBotReuseGuard } from "@/features/channels/ui/AddChannelBotReuseGuard";
import { useReusableAgentDetection } from "@/features/channels/ui/useReusableAgentDetection";
import { AddChannelBotTeamsSection } from "@/features/channels/ui/AddChannelBotTeamsSection";
import { probeBackendProvider } from "@/shared/api/tauri";
import type {
  AcpRuntime,
  BackendProviderCandidate,
  BackendProviderProbeResult,
  ManagedAgentBackend,
  RespondToMode,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Dialog } from "@/shared/ui/dialog";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  coerceConfigValues,
  ProviderConfigFields,
} from "@/features/agents/ui/ProviderConfigFields";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { useEffectiveRuntimes } from "@/features/channels/ui/useEffectiveRuntimes";
import { getActivePersonas } from "@/features/agents/lib/catalog";
import { getUsableTeams } from "@/features/agents/lib/teamPersonas";
import { useLastRuntime } from "@/features/agents/lib/useLastRuntime";
import { CreateAgentRespondToField } from "@/features/agents/ui/RespondToField";

type AddChannelBotDialogProps = {
  backendProviders?: BackendProviderCandidate[];
  backendProvidersLoading?: boolean;
  channelId: string | null;
  open: boolean;
  providers: AcpRuntime[];
  providersErrorMessage?: string | null;
  providersLoading?: boolean;
  onAdded?: (result: CreateChannelManagedAgentResult) => void;
  onOpenChange: (open: boolean) => void;
};

const RUNTIME_NONE_SENTINEL = "__none__";

function defaultBotName(runtime: AcpRuntime | null) {
  if (!runtime) {
    return "";
  }

  const normalizedId = runtime.id.trim().toLowerCase();
  if (normalizedId.length > 0) {
    return normalizedId;
  }

  return runtime.label.trim().toLowerCase() || "agent";
}

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
  backendProviders,
  backendProvidersLoading,
  channelId,
  open,
  providers,
  providersErrorMessage,
  providersLoading = false,
  onAdded,
  onOpenChange,
}: AddChannelBotDialogProps) {
  const { setLastRuntime } = useLastRuntime();
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
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState("");
  const [selectedPersonaIds, setSelectedPersonaIds] = React.useState<string[]>(
    [],
  );
  const [includeGeneric, setIncludeGeneric] = React.useState(false);
  const [customName, setCustomName] = React.useState("");
  const [customPrompt, setCustomPrompt] = React.useState("");
  const [hasEditedCustomName, setHasEditedCustomName] = React.useState(false);
  const [submissionNotice, setSubmissionNotice] = React.useState<string | null>(
    null,
  );
  const [submissionError, setSubmissionError] = React.useState<string | null>(
    null,
  );
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );
  const [forceNewInstance, setForceNewInstance] = React.useState(false);

  const resolvedBackendProviders = backendProviders ?? [];
  const resolvedBackendProvidersLoading = backendProvidersLoading ?? false;

  const [runOn, setRunOn] = React.useState<"local" | string>("local");
  const [providerConfig, setProviderConfig] = React.useState<
    Record<string, string>
  >({});
  const [probedProvider, setProbedProvider] =
    React.useState<BackendProviderProbeResult | null>(null);
  const [probeError, setProbeError] = React.useState<string | null>(null);

  const selectedRuntime = React.useMemo(
    () =>
      selectedRuntimeId
        ? (providers.find((runtime) => runtime.id === selectedRuntimeId) ??
          null)
        : null,
    [providers, selectedRuntimeId],
  );
  const isOverrideActive = selectedRuntime !== null;
  const selectedPersonas = React.useMemo(
    () => personas.filter((persona) => selectedPersonaIds.includes(persona.id)),
    [personas, selectedPersonaIds],
  );
  const selectedCount = selectedPersonas.length + (includeGeneric ? 1 : 0);

  const reusableAgent = useReusableAgentDetection(
    channelId,
    open && channelId !== null,
    selectedRuntime ?? providers[0] ?? null,
    selectedPersonas,
    includeGeneric,
    customPrompt,
  );

  const { runtimeWarnings, effectiveRuntimes } = useEffectiveRuntimes(
    personas,
    selectedPersonas,
    providers,
    selectedRuntime,
    isOverrideActive,
  );

  const isProviderMode = runOn !== "local";
  const selectedBackendProvider = React.useMemo(
    () => resolvedBackendProviders.find((p) => p.id === runOn) ?? null,
    [resolvedBackendProviders, runOn],
  );
  const providerConfigComplete = React.useMemo(() => {
    if (!isProviderMode || !probedProvider?.config_schema) return true;
    const schema = probedProvider.config_schema as Record<string, unknown>;
    const required: string[] = (schema?.required as string[] | undefined) ?? [];
    return required.every(
      (key) => (providerConfig[key] ?? "").trim().length > 0,
    );
  }, [isProviderMode, probedProvider, providerConfig]);

  React.useEffect(() => {
    if (!selectedRuntime || hasEditedCustomName) {
      return;
    }

    setCustomName(defaultBotName(selectedRuntime));
  }, [hasEditedCustomName, selectedRuntime]);

  React.useEffect(() => {
    setSelectedPersonaIds((current) =>
      current.filter((id) => personas.some((persona) => persona.id === id)),
    );
  }, [personas]);

  React.useEffect(() => {
    if (!isProviderMode || !selectedBackendProvider) {
      setProbedProvider(null);
      setProbeError(null);
      return;
    }

    let cancelled = false;
    setProbeError(null);
    setProbedProvider(null);

    probeBackendProvider(selectedBackendProvider.binaryPath)
      .then((result) => {
        if (!cancelled) {
          setProbedProvider(result);
          if (result.config_schema) {
            const props =
              (result.config_schema as Record<string, unknown>)?.properties ??
              {};

            const defaults: Record<string, string> = {};
            for (const [key, prop] of Object.entries(props) as [
              string,
              Record<string, unknown>,
            ][]) {
              if (prop.default != null) {
                defaults[key] = String(prop.default);
              }
            }
            setProviderConfig(defaults);
          }
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setProbeError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => {
      cancelled = true;
    };
  }, [isProviderMode, selectedBackendProvider]);

  function reset() {
    setSelectedRuntimeId("");
    setSelectedPersonaIds([]);
    setIncludeGeneric(false);
    setCustomName(providers[0] ? defaultBotName(providers[0]) : "");
    setCustomPrompt("");
    setHasEditedCustomName(false);
    setSubmissionNotice(null);
    setSubmissionError(null);
    setRunOn("local");
    setProviderConfig({});
    setProbedProvider(null);
    setProbeError(null);
    setRespondTo("owner-only");
    setRespondToAllowlist([]);
    setForceNewInstance(false);
    createBotsMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      reset();
    }

    onOpenChange(next);
  }

  function handleToggleTeam(personaIds: string[]) {
    setSelectedPersonaIds((current) => {
      const allSelected = personaIds.every((id) => current.includes(id));
      if (allSelected) {
        return current.filter((id) => !personaIds.includes(id));
      }
      const merged = new Set([...current, ...personaIds]);
      return [...merged];
    });
    setSubmissionNotice(null);
    setSubmissionError(null);
  }

  function handleRunOnChange(value: string) {
    setRunOn(value);
    setProviderConfig({});
    setProbedProvider(null);
    setProbeError(null);
    setSubmissionNotice(null);
    setSubmissionError(null);
  }

  async function handleSubmit() {
    if (providers.length === 0 || selectedCount === 0) {
      return;
    }

    const backend: ManagedAgentBackend = isProviderMode
      ? {
          type: "provider",
          id: runOn,
          config: coerceConfigValues(
            providerConfig,
            probedProvider?.config_schema,
          ),
        }
      : { type: "local" };

    const respondToFields =
      respondTo !== "owner-only"
        ? {
            respondTo,
            respondToAllowlist:
              respondTo === "allowlist" ? respondToAllowlist : undefined,
          }
        : {};

    const inputs = [
      ...(includeGeneric
        ? [
            {
              runtime: selectedRuntime ?? providers[0],
              name: customName,
              systemPrompt: customPrompt,
              role: "bot" as const,
              backend,
              forceNewInstance,
              ...respondToFields,
            },
          ]
        : []),
      ...selectedPersonas.map((persona) => {
        const effectiveFallback = selectedRuntime ?? providers[0] ?? null;
        const resolved = resolvePersonaRuntime(
          persona.runtime,
          providers,
          effectiveFallback,
          isOverrideActive,
        );
        return {
          runtime: resolved.runtime ?? effectiveFallback ?? providers[0],
          name: persona.displayName,
          personaId: persona.id,
          systemPrompt: persona.systemPrompt,
          avatarUrl: persona.avatarUrl ?? undefined,
          model: persona.model ?? undefined,
          role: "bot" as const,
          backend,
          forceNewInstance,
          ...respondToFields,
        };
      }),
    ];

    setSubmissionNotice(null);
    setSubmissionError(null);

    try {
      const result = await createBotsMutation.mutateAsync(inputs);

      if (result.failures.length === 0) {
        if (result.successes[0]) {
          onAdded?.(result.successes[0]);
        }
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
      setIncludeGeneric(
        result.failures.some((failure) => failure.kind === "generic"),
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

  // Allowlist mode requires at least one entry, mirroring the harness's own
  // validation. If we let it through empty, the agent crash-loops at startup
  // with a config error.
  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  const canSubmit =
    (selectedRuntime !== null || providers.length > 0) &&
    selectedCount > 0 &&
    (!includeGeneric || customName.trim().length > 0) &&
    respondToValid &&
    !(isProviderMode && !probedProvider) &&
    providerConfigComplete &&
    !providersLoading &&
    !(isProviderMode && resolvedBackendProvidersLoading) &&
    !createBotsMutation.isPending;
  const canChooseProvider =
    providers.length > 0 && !providersLoading && !createBotsMutation.isPending;
  const canToggleSelections = !createBotsMutation.isPending;
  const runtimeTriggerLabel = providersLoading
    ? "Loading runtimes..."
    : providers.length === 0
      ? "No runtimes found"
      : (selectedRuntime?.label ?? "Use persona defaults");
  const addButtonLabel = createBotsMutation.isPending
    ? selectedCount > 1
      ? `Adding ${selectedCount}...`
      : "Adding..."
    : selectedCount > 1
      ? `Add ${selectedCount} agents`
      : "Add agent";

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-3xl"
        data-testid="add-channel-bot-dialog"
        description="Select any combination of saved personas, or turn on Generic for a one-off custom agent."
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
        {resolvedBackendProviders.length > 0 ? (
          <div className="space-y-1.5">
            <div className="text-sm font-medium">Run on</div>
            <select
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
              disabled={createBotsMutation.isPending}
              onChange={(e) => handleRunOnChange(e.target.value)}
              value={runOn}
            >
              <option value="local">This computer</option>
              {resolvedBackendProviders.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.id}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        {isProviderMode && selectedBackendProvider ? (
          <div className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              This provider at{" "}
              <span className="font-mono font-medium">
                {selectedBackendProvider.binaryPath}
              </span>{" "}
              will receive your agent&apos;s private key. Only use providers
              from trusted sources.
            </p>
          </div>
        ) : null}

        {probeError ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            Could not probe provider: {probeError}
          </p>
        ) : null}

        {isProviderMode && probedProvider?.config_schema ? (
          <ProviderConfigFields
            config={providerConfig}
            onChange={setProviderConfig}
            schema={probedProvider.config_schema}
          />
        ) : null}

        <div className="space-y-1.5">
          <div className="text-sm font-medium">Runtime</div>
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button
                className="h-9 max-w-full justify-start gap-1.5 rounded-full border border-border/50 bg-muted/45 px-3 text-sm font-medium text-foreground shadow-none hover:bg-muted/70"
                disabled={!canChooseProvider}
                size="default"
                type="button"
                variant="ghost"
              >
                <span className="truncate">{runtimeTriggerLabel}</span>
                <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="start"
              className="min-w-40"
              onCloseAutoFocus={(event) => event.preventDefault()}
            >
              <DropdownMenuRadioGroup
                onValueChange={(id) => {
                  if (id === RUNTIME_NONE_SENTINEL) {
                    setSelectedRuntimeId("");
                  } else {
                    setSelectedRuntimeId(id);
                    setLastRuntime(id);
                  }
                }}
                value={selectedRuntime?.id ?? RUNTIME_NONE_SENTINEL}
              >
                <DropdownMenuRadioItem value={RUNTIME_NONE_SENTINEL}>
                  Use persona defaults
                </DropdownMenuRadioItem>
                <DropdownMenuSeparator />
                {providers.map((provider) => (
                  <DropdownMenuRadioItem key={provider.id} value={provider.id}>
                    {provider.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
          <p className="text-xs text-muted-foreground">
            {isOverrideActive
              ? "All agents will use this runtime, overriding persona preferences."
              : "Each persona uses its preferred runtime. Choose a runtime above to override all."}
          </p>
        </div>

        {teams.length > 0 ? (
          <AddChannelBotTeamsSection
            canToggleSelections={canToggleSelections}
            inChannelPersonaIds={inChannelPersonaIds}
            isLoading={teamsQuery.isLoading}
            onToggleTeam={handleToggleTeam}
            personas={personas}
            selectedPersonaIds={selectedPersonaIds}
            teams={teams}
          />
        ) : null}

        <AddChannelBotPersonasSection
          canToggleSelections={canToggleSelections}
          effectiveRuntimes={effectiveRuntimes}
          inChannelPersonaIds={inChannelPersonaIds}
          includeGeneric={includeGeneric}
          isLoading={personasQuery.isLoading}
          onToggleGeneric={() => {
            setIncludeGeneric((current) => !current);
            setSubmissionNotice(null);
            setSubmissionError(null);
          }}
          onTogglePersona={(personaId) => {
            setSelectedPersonaIds((current) => toggleValue(current, personaId));
            setSubmissionNotice(null);
            setSubmissionError(null);
          }}
          personas={personas}
          selectedPersonaIds={selectedPersonaIds}
        />

        {includeGeneric ? (
          <AddChannelBotGenericSection
            disabled={createBotsMutation.isPending}
            name={customName}
            onNameChange={(value) => {
              setHasEditedCustomName(true);
              setCustomName(value);
            }}
            onPromptChange={setCustomPrompt}
            prompt={customPrompt}
          />
        ) : null}

        {reusableAgent ? (
          <div className="pt-2">
            <AddChannelBotReuseGuard
              disabled={createBotsMutation.isPending}
              forceNew={forceNewInstance}
              onForceNewChange={setForceNewInstance}
              reusableAgent={reusableAgent}
            />
          </div>
        ) : null}

        {selectedCount > 0 ? (
          <CreateAgentRespondToField
            allowlist={respondToAllowlist}
            disabled={createBotsMutation.isPending}
            mode={respondTo}
            onAllowlistChange={setRespondToAllowlist}
            onModeChange={setRespondTo}
          />
        ) : null}

        {selectedCount === 0 ? (
          <div className="rounded-2xl border border-dashed border-border/70 bg-muted/15 px-4 py-4 text-sm text-muted-foreground">
            Pick one or more personas, or enable Generic to add a custom agent.
          </div>
        ) : null}

        {providersErrorMessage ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {providersErrorMessage}
          </p>
        ) : null}

        {runtimeWarnings.length > 0
          ? runtimeWarnings.map((warning) => (
              <div
                className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3"
                key={warning}
              >
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
                <p className="text-sm text-warning">{warning}</p>
              </div>
            ))
          : null}

        {personasQuery.error instanceof Error ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {personasQuery.error.message}
          </p>
        ) : null}

        {submissionNotice ? (
          <p className="rounded-2xl border border-border/70 bg-muted/25 px-4 py-3 text-sm text-foreground">
            {submissionNotice}
          </p>
        ) : null}

        {submissionError ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {submissionError}
          </p>
        ) : null}

        {createBotsMutation.error instanceof Error ? (
          <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {createBotsMutation.error.message}
          </p>
        ) : null}
      </ChooserDialogContent>
    </Dialog>
  );
}
