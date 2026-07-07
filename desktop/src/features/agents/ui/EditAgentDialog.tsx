import * as React from "react";

import {
  useAcpRuntimesQuery,
  useAgentConfigSurface,
  usePersonasQuery,
  useRuntimeFileConfigQuery,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import type {
  ManagedAgent,
  RespondToMode,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getProviderApiKeyEnvVar,
  isMissingRequiredDropdownField,
  NO_RUNTIME_DROPDOWN_VALUE,
  runtimeSupportsLlmProviderSelection,
  requiredCredentialEnvKeys,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
  type PersonaDropdownOption,
} from "./personaDialogPickers";
import { shouldClearModelForRuntimeChange } from "./personaRuntimeModel";
import {
  AgentModelField,
  AgentProviderField,
} from "./personaProviderModelFields";
import {
  CreateAgentBasicsFields,
  CreateAgentRuntimeFields,
} from "./CreateAgentDialogSections";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import { CreateAgentRespondToField } from "./RespondToField";
import { usePersonaModelDiscovery } from "./usePersonaModelDiscovery";

export function EditAgentDialog({
  agent,
  open,
  onOpenChange,
  onUpdated,
}: {
  agent: ManagedAgent;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
}) {
  const updateMutation = useUpdateManagedAgentMutation();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const configSurfaceQuery = useAgentConfigSurface(open ? agent.pubkey : null);
  const runtimes = runtimesQuery.data ?? [];

  const [name, setName] = React.useState(agent.name);
  const [relayUrl, setRelayUrl] = React.useState(agent.relayUrl);
  const [acpCommand, setAcpCommand] = React.useState(agent.acpCommand);
  const [agentCommand, setAgentCommand] = React.useState(agent.agentCommand);
  // Whether the harness inherits from the linked persona (no explicit pin).
  // Only meaningful when a persona is linked; seeded from the override field
  // so an unset override shows as "inherit" rather than re-pinning on save.
  const [inheritHarness, setInheritHarness] = React.useState(
    agent.personaId != null && agent.agentCommandOverride == null,
  );
  const [agentArgs, setAgentArgs] = React.useState(agent.agentArgs.join(","));
  const [mcpCommand, setMcpCommand] = React.useState(agent.mcpCommand);
  const [mcpToolsets, setMcpToolsets] = React.useState(agent.mcpToolsets ?? "");
  const [turnTimeoutSeconds, setTurnTimeoutSeconds] = React.useState(
    String(agent.turnTimeoutSeconds),
  );
  const [parallelism, setParallelism] = React.useState(
    String(agent.parallelism),
  );
  const [systemPrompt, setSystemPrompt] = React.useState(
    agent.systemPrompt ?? "",
  );
  const [model, setModel] = React.useState(agent.model ?? "");
  const [isCustomModelEditing, setIsCustomModelEditing] = React.useState(false);
  const [provider, setProvider] = React.useState(agent.provider ?? "");
  const [isCustomProviderEditing, setIsCustomProviderEditing] =
    React.useState(false);
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>(agent.envVars);
  const personasQuery = usePersonasQuery();
  const linkedPersona = React.useMemo(
    () =>
      agent.personaId
        ? (personasQuery.data?.find((p) => p.id === agent.personaId) ?? null)
        : null,
    [agent.personaId, personasQuery.data],
  );
  const inheritedEnvVars = linkedPersona?.envVars ?? {};
  const [respondTo, setRespondTo] = React.useState<RespondToMode>(
    agent.respondTo,
  );
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    agent.respondToAllowlist,
  );

  // Runtime selector: defaults to "custom" until the dialog opens and the
  // catalog loads. The open-effect re-derives the correct id from the catalog.
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState("custom");

  // Tracks whether the user has made an in-dialog runtime selection. When true,
  // the catalog-arrival effect must not overwrite it (the user's choice wins).
  // Reset to false each time the dialog opens so a fresh open always re-derives.
  const runtimeTouched = React.useRef(false);

  // Reset form state only when the dialog opens or when switching to a different
  // agent. Omitting the full agent object and its array fields from deps prevents
  // the effect from firing on every 5s background poll (arrays are never
  // reference-equal across renders), which would wipe in-progress user edits.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — including agent fields would re-fire on every 5s poll and wipe edits
  React.useEffect(() => {
    if (open) {
      setName(agent.name);
      setRelayUrl(agent.relayUrl);
      setAcpCommand(agent.acpCommand);
      setAgentCommand(agent.agentCommand);
      setInheritHarness(
        agent.personaId != null && agent.agentCommandOverride == null,
      );
      setAgentArgs(agent.agentArgs.join(","));
      setMcpCommand(agent.mcpCommand);
      setMcpToolsets(agent.mcpToolsets ?? "");
      setTurnTimeoutSeconds(String(agent.turnTimeoutSeconds));
      setParallelism(String(agent.parallelism));
      setSystemPrompt(agent.systemPrompt ?? "");
      setModel(agent.model ?? "");
      setIsCustomModelEditing(false);
      setProvider(agent.provider ?? "");
      setIsCustomProviderEditing(false);
      setEnvVars(agent.envVars);
      setRespondTo(agent.respondTo);
      setRespondToAllowlist(agent.respondToAllowlist);
      // Re-derive the runtime id from whatever catalog entries have loaded.
      // If the catalog hasn't arrived yet, the catalog-arrival effect below
      // will re-derive once it does (guarded by runtimeTouched).
      runtimeTouched.current = false;
      // Match by command path first (explicit pins store the resolved path).
      // Fall back to id-match for agents where agentCommand is the short name
      // (e.g. "buzz-agent") while the catalog stores the resolved binary path —
      // the same id-fallback used in effectiveRuntimeIdForSubmit.
      const matched =
        runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim()) ??
        runtimes.find((r) => r.id === agent.agentCommand.trim());
      setSelectedRuntimeId(matched ? matched.id : "custom");
      updateMutation.reset();
    }
  }, [open, agent.pubkey]);

  // Re-derive the runtime id when the catalog loads, but ONLY while the user
  // has not made a manual runtime selection (runtimeTouched === false). This
  // handles the async race where the dialog opens before runtimes have loaded:
  // the open-effect sees [], falls back to "custom", and this effect corrects
  // it once the catalog arrives — without re-firing the full open reset (which
  // would wipe other edits).
  React.useEffect(() => {
    if (!open || runtimeTouched.current || runtimes.length === 0) {
      return;
    }
    // Same dual-match as the open-effect: command path first, then id fallback
    // for agents whose agentCommand is the short name (e.g. "buzz-agent").
    const matched =
      runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim()) ??
      runtimes.find((r) => r.id === agent.agentCommand.trim());
    if (matched) {
      setSelectedRuntimeId(matched.id);
    }
  }, [open, runtimes, agent.agentCommand]);

  // Build the sorted runtime catalog for the dropdown.
  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
  );

  // selectedRuntime: catalog entry for the live-selected runtime id.
  // When "custom" or an unknown id, falls back to undefined.
  const selectedRuntime = React.useMemo(
    () => runtimes.find((r) => r.id === selectedRuntimeId),
    [runtimes, selectedRuntimeId],
  );

  // Runtime dropdown options: catalog entries plus "Custom command" fallback.
  // Always include the current id in case it came from an unavailable runtime.
  const runtimeDropdownValue = selectedRuntimeId || NO_RUNTIME_DROPDOWN_VALUE;

  const runtimeDropdownOptions: PersonaDropdownOption[] = React.useMemo(() => {
    const options: PersonaDropdownOption[] = [
      ...sortedRuntimes.map((candidate) => ({
        label: formatRuntimeOptionLabel(candidate),
        value: candidate.id,
      })),
      { label: "Custom command", value: "custom" },
    ];
    // If the current selection isn't in the list, add it so the dropdown isn't blank.
    if (
      selectedRuntimeId &&
      selectedRuntimeId !== "custom" &&
      !options.some((o) => o.value === selectedRuntimeId)
    ) {
      options.push({
        label: `${selectedRuntimeId} (current)`,
        value: selectedRuntimeId,
      });
    }
    return options;
  }, [sortedRuntimes, selectedRuntimeId]);

  // Provider field is visible only when the LIVE selected runtime supports
  // LLM-provider selection. Keying on the live runtime (not the saved provider)
  // prevents a stale saved provider from staying visible after switching to a
  // locked runtime (e.g. Claude).
  const llmProviderFieldVisible = runtimeSupportsLlmProviderSelection(
    selectedRuntime?.id ?? selectedRuntimeId,
  );

  const providerForDiscovery = llmProviderFieldVisible ? provider : "";
  const normalizedConfig = configSurfaceQuery.data?.normalized;
  const modelRequired = isMissingRequiredDropdownField(
    normalizedConfig?.model,
    model,
  );
  const providerRequired = isMissingRequiredDropdownField(
    normalizedConfig?.provider,
    provider,
  );

  // The runtime id that will actually be active after submit. When inheriting,
  // resolve from agent.agentCommand (the persona's runtime) using the same
  // dual-match used at submit time — command path first, then id fallback for
  // catalog entries where the adapter binary is missing (command:null). This
  // single prospective id feeds BOTH the block-save gate (requiredEnvKeys) and
  // the submit path so they never disagree on which runtime is being saved.
  const prospectiveRuntimeId = React.useMemo(() => {
    if (!inheritHarness) {
      return selectedRuntime?.id ?? selectedRuntimeId;
    }
    return (
      runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim())
        ?.id ??
      runtimes.find((r) => r.id === agent.agentCommand.trim())?.id ??
      ""
    );
  }, [
    inheritHarness,
    runtimes,
    agent.agentCommand,
    selectedRuntime?.id,
    selectedRuntimeId,
  ]);

  // Provider used for required-key validation — keyed off the PROSPECTIVE
  // runtime, not the current dropdown. When the user transitions from a
  // CLI-login pin (claude) to inherit a buzz-agent/goose persona, the current
  // dropdown would suppress provider to "" (llmProviderFieldVisible=false),
  // making requiredCredentialEnvKeys return [] and falsely unblocking the save.
  // Using prospectiveRuntimeId here ensures the gate checks the credential
  // requirements of the runtime that will actually be saved.
  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
  )
    ? provider
    : "";

  // Required credential env keys for the PROSPECTIVE post-submit runtime.
  // Using the prospective id (not the current dropdown) ensures the gate
  // validates what will actually be saved — in particular, on the inherit
  // transition (claude→buzz-agent or buzz-agent→claude) the gate reflects
  // the inherited runtime's requirements, not the old pin's.
  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(
    prospectiveRuntimeId,
    { enabled: open },
  );
  // Credential keys satisfied by the runtime file config — shown as
  // "Set in goose config" rows rather than amber required rows.
  const fileSatisfiedEnvKeys = React.useMemo(() => {
    if (!runtimeFileConfig) return [] as string[];
    const allKeys = requiredCredentialEnvKeys(
      prospectiveRuntimeId,
      providerForRequiredKeys,
    );
    return allKeys.filter(
      (key) =>
        (envVars[key] ?? "").length === 0 &&
        runtimeFileConfig.satisfiedEnvKeys.includes(key),
    );
  }, [
    runtimeFileConfig,
    prospectiveRuntimeId,
    providerForRequiredKeys,
    envVars,
  ]);

  const requiredEnvKeys = React.useMemo(
    () =>
      requiredCredentialEnvKeys(
        prospectiveRuntimeId,
        providerForRequiredKeys,
      ).filter((key) => !fileSatisfiedEnvKeys.includes(key)),
    [prospectiveRuntimeId, providerForRequiredKeys, fileSatisfiedEnvKeys],
  );

  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars,
    isCustomProviderEditing,
    modelFieldVisible: true,
    open,
    provider: providerForDiscovery,
    selectedRuntime,
  });

  // When the provider scope changes and the current model is no longer valid
  // for the new scope, clear it (mirrors Persona's useEffect for the same).
  React.useEffect(() => {
    if (
      !open ||
      isCustomModelEditing ||
      !shouldClearKnownModelForSelectionScope({
        model,
        provider: providerForDiscovery,
        runtime: selectedRuntime?.id ?? selectedRuntimeId,
      })
    ) {
      return;
    }

    setModel("");
    setIsCustomModelEditing(false);
  }, [
    isCustomModelEditing,
    model,
    open,
    providerForDiscovery,
    selectedRuntime,
    selectedRuntimeId,
  ]);

  function handleRuntimeDropdownChange(nextValue: string) {
    const nextRuntimeId = nextValue;
    const previousRuntimeId = selectedRuntimeId;
    const nextRuntime = runtimes.find((r) => r.id === nextRuntimeId);
    const nextCanChooseProvider = runtimeSupportsLlmProviderSelection(
      nextRuntime?.id ?? nextRuntimeId,
    );

    // Mark that the user has made an explicit runtime choice. The catalog-arrival
    // effect will no longer overwrite selectedRuntimeId after this point.
    runtimeTouched.current = true;

    setSelectedRuntimeId(nextRuntimeId);

    // When switching to a catalog-known runtime, update the agent command to
    // its resolved command so the command field stays consistent.
    if (nextRuntime?.command) {
      setAgentCommand(nextRuntime.command);
      const newArgs = nextRuntime.defaultArgs.join(",");
      setAgentArgs(newArgs);
      // Selecting a concrete catalog runtime pins the harness — this is the
      // authoritative override. Disabling inheritance ensures the runtime is
      // actually persisted and prevents a mismatched provider from being saved
      // against an inherited runtime that will actually run something else.
      setInheritHarness(false);
    } else if (nextRuntimeId === "custom") {
      // "Custom" means the user wants to type a command; leave agentCommand as-is.
    }

    // Clear model when switching away from a runtime with a different model scope.
    if (
      shouldClearModelForRuntimeChange(previousRuntimeId, nextRuntimeId) ||
      shouldClearKnownModelForSelectionScope({
        model,
        provider,
        runtime: nextRuntime?.id ?? nextRuntimeId,
      })
    ) {
      setModel("");
      setIsCustomModelEditing(false);
    }

    // When switching to a provider-locked runtime, clear provider state so no
    // conflicting provider is persisted on a runtime that doesn't support it.
    if (!nextCanChooseProvider) {
      const previousProviderApiKeyEnvVar = getProviderApiKeyEnvVar(provider);
      if (previousProviderApiKeyEnvVar) {
        setEnvVars((current) => {
          const next = { ...current };
          delete next[previousProviderApiKeyEnvVar];
          return next;
        });
      }
      setIsCustomModelEditing(false);
      setIsCustomProviderEditing(false);
      setProvider("");
    }
  }

  function handleProviderDropdownChange(nextValue: string) {
    if (nextValue === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
      const previousProviderApiKeyEnvVar = getProviderApiKeyEnvVar(provider);
      if (previousProviderApiKeyEnvVar) {
        setEnvVars((current) => {
          const next = { ...current };
          delete next[previousProviderApiKeyEnvVar];
          return next;
        });
      }
      setIsCustomProviderEditing(true);
      setProvider("");
      return;
    }

    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;

    // Clear the old provider API key when switching providers.
    const previousProviderApiKeyEnvVar = getProviderApiKeyEnvVar(provider);
    const nextProviderApiKeyEnvVar = getProviderApiKeyEnvVar(nextProvider);
    if (
      previousProviderApiKeyEnvVar &&
      previousProviderApiKeyEnvVar !== nextProviderApiKeyEnvVar
    ) {
      setEnvVars((current) => {
        const next = { ...current };
        delete next[previousProviderApiKeyEnvVar];
        return next;
      });
    }

    setIsCustomProviderEditing(false);
    setProvider(nextProvider);

    // Clear the model when switching to a provider that requires a different
    // explicit model selection.
    if (
      !isCustomModelEditing &&
      shouldClearKnownModelForSelectionScope({
        model,
        provider: nextProvider,
        runtime: selectedRuntime?.id ?? selectedRuntimeId,
      })
    ) {
      setModel("");
      setIsCustomModelEditing(false);
    }
  }

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
  }

  const parallelismValid =
    parallelism.trim() === "" ||
    !Number.isNaN(Number.parseInt(parallelism, 10));
  const timeoutValid =
    turnTimeoutSeconds.trim() === "" ||
    !Number.isNaN(Number.parseInt(turnTimeoutSeconds, 10));
  // Block clearing a previously-set command to empty — sending an empty string
  // for a required command field would cause a runtime failure at spawn.
  const acpCommandValid = !(agent.acpCommand && acpCommand.trim() === "");
  // Allowlist mode requires at least one entry — mirrors the harness's own
  // validation. The backend would reject the request anyway; we block early
  // so the user sees the disabled button instead of a round-tripped error.
  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  const canSubmit =
    name.trim().length > 0 &&
    parallelismValid &&
    timeoutValid &&
    acpCommandValid &&
    respondToValid &&
    !updateMutation.isPending;

  async function handleSubmit() {
    try {
      const parsedParallelism = Number.parseInt(parallelism, 10);
      const parsedTimeout = Number.parseInt(turnTimeoutSeconds, 10);
      const parsedArgs = agentArgs
        .split(",")
        .map((v) => v.trim())
        .filter((v) => v.length > 0);
      const normalizedModel = model.trim() || null;
      const normalizedProvider = provider.trim() || null;

      // Harness pin resolution. The backend treats an empty string as the
      // "inherit from persona" sentinel (clears the override) and any concrete
      // command as an explicit pin. When inheriting, only send the sentinel if
      // there's a pin to clear — a name-only edit must leave the record alone.
      // When pinning, send the command only if it diverges from the resolved
      // value the dialog opened with, so an unchanged save stays a no-op.
      const agentCommandUpdate = inheritHarness
        ? agent.agentCommandOverride != null
          ? ""
          : undefined
        : agentCommand.trim() !== agent.agentCommand
          ? agentCommand.trim()
          : undefined;

      // Derive the effective runtime at submit time — the one that will
      // actually run AFTER submit. This is the component-scope prospectiveRuntimeId,
      // which is shared with the block-save gate (requiredEnvKeys) so both
      // always agree on which runtime is being saved.
      const effectiveRuntimeIdForSubmit = prospectiveRuntimeId;

      // Classify the effective runtime's provider capability as a tri-state so
      // the provider submit branch can distinguish "known-locked" (clear) from
      // "unknown" (omit). Clearing must ONLY happen when we KNOW the runtime is
      // provider-locked (e.g. Claude). When capability is unknown — because the
      // catalog is still loading, the query errored, or the inherited command
      // matched nothing — we OMIT the field rather than sending null, so a
      // transient discovery/loading state never becomes a destructive write.
      type ProviderRuntimeCapability = "capable" | "locked" | "unknown";
      const matchedCatalogEntry =
        effectiveRuntimeIdForSubmit.length > 0
          ? runtimes.find((r) => r.id === effectiveRuntimeIdForSubmit)
          : undefined;
      const providerRuntimeCapability: ProviderRuntimeCapability =
        matchedCatalogEntry === undefined
          ? "unknown"
          : runtimeSupportsLlmProviderSelection(matchedCatalogEntry.id)
            ? "capable"
            : "locked";

      const input: UpdateManagedAgentInput = {
        pubkey: agent.pubkey,
        name: name.trim() !== agent.name ? name.trim() : undefined,
        relayUrl:
          relayUrl.trim() !== agent.relayUrl ? relayUrl.trim() : undefined,
        acpCommand:
          acpCommand.trim() !== agent.acpCommand
            ? acpCommand.trim()
            : undefined,
        agentCommand: agentCommandUpdate,
        agentArgs:
          parsedArgs.join(",") !== agent.agentArgs.join(",")
            ? parsedArgs
            : undefined,
        mcpCommand:
          mcpCommand.trim() !== agent.mcpCommand
            ? mcpCommand.trim()
            : undefined,
        mcpToolsets:
          (mcpToolsets.trim() || null) !== agent.mcpToolsets
            ? mcpToolsets.trim() || null
            : undefined,
        turnTimeoutSeconds:
          parsedTimeout > 0 && parsedTimeout !== agent.turnTimeoutSeconds
            ? parsedTimeout
            : undefined,
        parallelism:
          parsedParallelism > 0 && parsedParallelism !== agent.parallelism
            ? parsedParallelism
            : undefined,
        // Use tri-state: send null to clear, value to set, omit if unchanged.
        systemPrompt:
          (systemPrompt.trim() || null) !== agent.systemPrompt
            ? systemPrompt.trim() || null
            : undefined,
        model:
          normalizedModel !== (agent.model ?? null)
            ? normalizedModel
            : undefined,
        // Tri-state provider persistence keyed on providerRuntimeCapability:
        //   "capable"  → persist: value if changed, omit if unchanged.
        //   "locked"   → clear: send null if provider was set, else omit.
        //   "unknown"  → omit always (never send null for a transient state).
        // llmProviderFieldVisible is for UX visibility only; not used here.
        provider:
          providerRuntimeCapability === "capable"
            ? normalizedProvider !== (agent.provider ?? null)
              ? normalizedProvider
              : undefined
            : providerRuntimeCapability === "locked"
              ? (agent.provider ?? null) !== null
                ? null
                : undefined
              : undefined, // "unknown" → omit always
        envVars: envVarsChanged(envVars, agent.envVars) ? envVars : undefined,
        respondTo: respondTo !== agent.respondTo ? respondTo : undefined,
        // The allowlist is preserved across mode toggles in local UI state
        // (so a user can flip away from allowlist and back without losing
        // their entries), but we only send it on the wire when (a) it
        // actually changed, AND (b) the saved mode will need it. Sending
        // an allowlist while switching to a non-allowlist mode would be
        // harmless server-side, but it's noise in the persisted record.
        respondToAllowlist:
          respondTo === "allowlist" &&
          respondToAllowlist.join(",") !== agent.respondToAllowlist.join(",")
            ? respondToAllowlist
            : undefined,
      };

      const result = await updateMutation.mutateAsync(input);
      if (result.profileSyncError) {
        console.warn("Relay profile sync failed:", result.profileSyncError);
      }
      handleOpenChange(false);
      onUpdated?.(result.agent);
    } catch {
      // React Query stores the error; keep dialog open and render it inline.
    }
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-3xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Edit agent</DialogTitle>
            <DialogDescription>
              Update configuration for{" "}
              <span className="font-medium">{agent.name}</span>. Changes take
              effect on the next start.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <CreateAgentBasicsFields name={name} onNameChange={setName} />

            <CreateAgentRespondToField
              allowlist={respondToAllowlist}
              mode={respondTo}
              onAllowlistChange={setRespondToAllowlist}
              onModeChange={setRespondTo}
            />

            <AgentModelField
              disabled={updateMutation.isPending}
              discoveredModelOptions={discoveredModelOptions}
              isCustomModelEditing={isCustomModelEditing}
              isRequired={modelRequired}
              model={model}
              modelDiscoveryLoading={modelDiscoveryLoading}
              modelDiscoveryStatus={modelDiscoveryStatus}
              onIsCustomModelEditingChange={setIsCustomModelEditing}
              onModelChange={setModel}
            />

            {llmProviderFieldVisible ? (
              <AgentProviderField
                disabled={updateMutation.isPending}
                isCustomProviderEditing={isCustomProviderEditing}
                isRequired={providerRequired}
                onProviderChange={handleProviderDropdownChange}
                provider={provider}
                selectedRuntime={selectedRuntime}
              />
            ) : null}

            {linkedPersona ? (
              <div className="space-y-1.5">
                <label
                  className="flex items-center gap-2 text-sm font-medium"
                  htmlFor="agent-inherit-harness"
                >
                  <input
                    checked={inheritHarness}
                    id="agent-inherit-harness"
                    onChange={(event) =>
                      setInheritHarness(event.target.checked)
                    }
                    type="checkbox"
                  />
                  Inherit runtime from persona
                </label>
                <p className="text-xs text-muted-foreground">
                  {inheritHarness
                    ? `Uses the ${linkedPersona.displayName} persona's runtime${
                        linkedPersona.runtime
                          ? ` (${linkedPersona.runtime})`
                          : ""
                      }. Editing the persona and respawning propagates the new runtime.`
                    : "Pins this agent to a specific runtime command, overriding the persona's runtime."}
                </p>
              </div>
            ) : null}

            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="agent-runtime">
                Agent runtime
              </label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                disabled={updateMutation.isPending}
                id="agent-runtime"
                onChange={(event) =>
                  handleRuntimeDropdownChange(event.target.value)
                }
                value={runtimeDropdownValue}
              >
                {runtimeDropdownOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              {selectedRuntime ? (
                <p className="text-xs text-muted-foreground">
                  Detected at{" "}
                  <span className="font-medium">
                    {selectedRuntime.binaryPath ??
                      selectedRuntime.command ??
                      selectedRuntime.id}
                  </span>
                </p>
              ) : null}
            </div>

            <CreateAgentRuntimeFields
              acpCommand={acpCommand}
              agentArgs={agentArgs}
              agentCommand={agentCommand}
              mcpCommand={mcpCommand}
              mcpToolsets={mcpToolsets}
              onAcpCommandChange={setAcpCommand}
              onAgentArgsChange={setAgentArgs}
              onAgentCommandChange={setAgentCommand}
              onMcpCommandChange={setMcpCommand}
              onMcpToolsetsChange={setMcpToolsets}
              onParallelismChange={setParallelism}
              onRelayUrlChange={setRelayUrl}
              onSystemPromptChange={setSystemPrompt}
              onTurnTimeoutChange={setTurnTimeoutSeconds}
              parallelism={parallelism}
              relayUrl={relayUrl}
              // "custom" surfaces the agent-command input so a user can pin a
              // harness; when inheriting we hide it (any non-"custom" id) since
              // the command comes from the persona's runtime.
              selectedRuntimeId={
                inheritHarness
                  ? "inherit"
                  : selectedRuntimeId === "custom"
                    ? "custom"
                    : "inherit"
              }
              systemPrompt={systemPrompt}
              turnTimeoutSeconds={turnTimeoutSeconds}
            />

            <EnvVarsEditor
              disabled={updateMutation.isPending}
              fileSatisfiedKeys={fileSatisfiedEnvKeys}
              helperText="Per-agent env vars. Override the persona's vars on collision."
              inheritedFrom={inheritedEnvVars}
              inheritedLabel="persona"
              onChange={setEnvVars}
              requiredKeys={requiredEnvKeys}
              value={envVars}
            />

            {updateMutation.error instanceof Error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {updateMutation.error.message}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
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
              {updateMutation.isPending ? "Saving..." : "Save changes"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function envVarsChanged(
  a: Record<string, string>,
  b: Record<string, string>,
): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return true;
  for (const k of aKeys) {
    if (a[k] !== b[k]) return true;
  }
  return false;
}
