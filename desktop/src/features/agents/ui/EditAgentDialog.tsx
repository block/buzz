import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

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
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { EditAgentAdvancedFields } from "./EditAgentAdvancedFields";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getModelSelectValue,
  getPersonaProviderOptions,
  getProviderApiKeyEnvVar,
  hasPersonaModelOption,
  isMissingRequiredDropdownField,
  NO_RUNTIME_DROPDOWN_VALUE,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  runtimeSupportsLlmProviderSelection,
  requiredCredentialEnvKeys,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
  type PersonaDropdownOption,
  type PersonaModelOption,
} from "./personaDialogPickers";
import {
  computeEditAgentFormValidity,
  resolveAgentCommandUpdate,
  shouldClearModelForRuntimeChange,
} from "./personaRuntimeModel";
import { AgentCreationPreview } from "./AgentCreationPreview";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { CreateAgentRespondToField } from "./RespondToField";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  MODEL_DISCOVERY_LOADING_VALUE,
  usePersonaModelDiscovery,
} from "./usePersonaModelDiscovery";

const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

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
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [avatarUrl, setAvatarUrl] = React.useState(agent.avatarUrl ?? "");
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const shouldReduceMotion = useReducedMotion();

  // Runtime selector: defaults to "custom" until the dialog opens and the
  // catalog loads. The open-effect re-derives the correct id from the catalog.
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState("custom");

  // Tracks whether the user has made an in-dialog runtime selection.
  const runtimeTouched = React.useRef(false);

  // Reset form state only when the dialog opens or when switching to a different agent.
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
      setAvatarUrl(agent.avatarUrl ?? "");
      setShowAdvancedFields(false);
      setIsAvatarUploadPending(false);
      runtimeTouched.current = false;
      const matched =
        runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim()) ??
        runtimes.find((r) => r.id === agent.agentCommand.trim());
      setSelectedRuntimeId(matched ? matched.id : "custom");
      updateMutation.reset();
    }
  }, [open, agent.pubkey]);

  // Re-derive the runtime id when the catalog loads.
  React.useEffect(() => {
    if (!open || runtimeTouched.current || runtimes.length === 0) {
      return;
    }
    const matched =
      runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim()) ??
      runtimes.find((r) => r.id === agent.agentCommand.trim());
    if (matched) {
      setSelectedRuntimeId(matched.id);
    }
  }, [open, runtimes, agent.agentCommand]);

  const sortedRuntimes = React.useMemo(
    () => sortPersonaRuntimes(runtimes),
    [runtimes],
  );

  const selectedRuntime = React.useMemo(
    () => runtimes.find((r) => r.id === selectedRuntimeId),
    [runtimes, selectedRuntimeId],
  );

  const runtimeDropdownValue = selectedRuntimeId || NO_RUNTIME_DROPDOWN_VALUE;

  const runtimeDropdownOptions: PersonaDropdownOption[] = React.useMemo(() => {
    const options: PersonaDropdownOption[] = [
      ...sortedRuntimes.map((candidate) => ({
        label: formatRuntimeOptionLabel(candidate),
        value: candidate.id,
      })),
      { label: "Custom command", value: "custom" },
    ];
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

  // Clear model when provider scope changes and current model is no longer valid.
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
    const nextRuntimeId =
      nextValue === NO_RUNTIME_DROPDOWN_VALUE ? "" : nextValue;
    const previousRuntimeId = selectedRuntimeId;
    const nextRuntime = runtimes.find((r) => r.id === nextRuntimeId);
    const nextCanChooseProvider = runtimeSupportsLlmProviderSelection(
      nextRuntime?.id ?? nextRuntimeId,
    );

    // Mark that the user has made an explicit runtime choice. The catalog-arrival
    // effect will no longer overwrite selectedRuntimeId after this point.
    runtimeTouched.current = true;

    const resolvedRuntimeId = nextRuntimeId || "custom";
    setSelectedRuntimeId(resolvedRuntimeId);

    const isCustomCommand = resolvedRuntimeId === "custom";

    // Only pin the harness when the selection can actually supply a command:
    //   - "Custom command": the Advanced command input becomes editable, so the
    //     user provides the command.
    //   - a catalog entry with a concrete command: we set it below.
    // A catalog entry with command:null (availability adapter_missing /
    // not_installed) can't produce a runnable command — clearing inheritance
    // there would omit agentCommand on Save (command unchanged) while the
    // provider/model logic treats the new runtime as effective, so an inherited
    // Claude agent could persist a Databricks provider while still running
    // Claude. Keep inheriting in that case.
    if (isCustomCommand || nextRuntime?.command) {
      setInheritHarness(false);
    }

    // "Custom command" is the only selection whose command must be typed by
    // the user, and that input lives inside the collapsed Advanced section.
    // Auto-expand Advanced so the command field is visible — otherwise the
    // user can Save without ever seeing it, leaving agentCommand equal to the
    // original effective command (so the update is omitted) and the custom
    // selection silently no-ops. See handleSubmit's customCommandPinned gate,
    // which blocks Save when the revealed field is still empty.
    if (isCustomCommand) {
      setShowAdvancedFields(true);
    }

    // When switching to a catalog-known runtime, update the agent command to
    // its resolved command so the command field stays consistent.
    if (nextRuntime?.command) {
      setAgentCommand(nextRuntime.command);
      const newArgs = nextRuntime.defaultArgs.join(",");
      setAgentArgs(newArgs);
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

  function handleModelDropdownChange(nextValue: string) {
    if (nextValue === CUSTOM_MODEL_DROPDOWN_VALUE) {
      setIsCustomModelEditing(true);
      return;
    }
    if (nextValue === AUTO_MODEL_DROPDOWN_VALUE) {
      setIsCustomModelEditing(false);
      setModel("");
      return;
    }
    setIsCustomModelEditing(false);
    setModel(nextValue);
  }

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
  }

  const canSubmit =
    computeEditAgentFormValidity({
      name,
      parallelism,
      turnTimeoutSeconds,
      agentAcpCommand: agent.acpCommand,
      acpCommand,
      respondTo,
      respondToAllowlistLength: respondToAllowlist.length,
      selectedRuntimeId,
      inheritHarness,
      agentCommand,
    }) &&
    !updateMutation.isPending &&
    !isAvatarUploadPending;

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

      // Harness pin resolution — see resolveAgentCommandUpdate for the full
      // sentinel/pin/no-op contract, including the inherit→pin transition where
      // the prefilled command equals the original but must still be pinned.
      const agentCommandUpdate = resolveAgentCommandUpdate({
        inheritHarness,
        agentCommand,
        originalAgentCommand: agent.agentCommand,
        agentCommandOverride: agent.agentCommandOverride ?? null,
      });

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

  // Model field derived state
  const trimmedModel = model.trim();
  const staticModelOptions: readonly PersonaModelOption[] = [
    { id: "", label: "Default model" },
  ];
  const effectiveModelOptions = discoveredModelOptions ?? staticModelOptions;
  const isModelCustom = !hasPersonaModelOption(
    effectiveModelOptions,
    trimmedModel,
  );
  const modelSelectValue = getModelSelectValue({
    isCustomModelEditing,
    isModelCustom,
    model,
  });
  const showCustomModelInput = isCustomModelEditing || isModelCustom;
  const modelDropdownOptions: PersonaDropdownOption[] = [
    ...effectiveModelOptions.map((option) => ({
      label: option.label,
      value: option.id || AUTO_MODEL_DROPDOWN_VALUE,
    })),
    ...(modelDiscoveryLoading && discoveredModelOptions === null
      ? [
          {
            disabled: true,
            label: "Loading models...",
            value: MODEL_DISCOVERY_LOADING_VALUE,
          },
        ]
      : []),
    { label: "Custom model...", value: CUSTOM_MODEL_DROPDOWN_VALUE },
  ];

  // Provider field derived state
  const trimmedProvider = provider.trim();
  const providerOptions = getPersonaProviderOptions(
    trimmedProvider,
    selectedRuntime?.id ?? "",
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : trimmedProvider || AUTO_PROVIDER_DROPDOWN_VALUE;
  const providerDropdownOptions: PersonaDropdownOption[] = [
    ...providerOptions.map((option) => ({
      label: option.label,
      value: option.id || AUTO_PROVIDER_DROPDOWN_VALUE,
    })),
    { label: "Custom provider...", value: CUSTOM_PROVIDER_DROPDOWN_VALUE },
  ];

  const previewLabel = name.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;
  const advancedFieldsTransition = shouldReduceMotion
    ? { duration: 0 }
    : ADVANCED_FIELDS_MOTION_TRANSITION;

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <ChooserDialogContent
        className="max-w-3xl border-0"
        contentClassName="pt-3"
        data-testid="edit-agent-dialog"
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        title={`Edit ${agent.name}`}
        footer={
          <div className="flex w-full items-center justify-end gap-2">
            <Button
              disabled={updateMutation.isPending || isAvatarUploadPending}
              onClick={() => handleOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="edit-agent-dialog-submit"
              disabled={!canSubmit}
              onClick={() => void handleSubmit()}
              type="button"
            >
              {updateMutation.isPending ? "Saving..." : "Save changes"}
            </Button>
          </div>
        }
      >
        <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
          {/* Avatar is display-only — UpdateManagedAgentInput has no
              avatarUrl field, so edits can't be persisted yet. Keep the
              preview disabled until the backend supports avatar updates. */}
          <AgentCreationPreview
            avatarUrl={previewAvatarUrl}
            disabled
            label={previewLabel}
            onClearAvatar={() => setAvatarUrl("")}
            onUploadPendingChange={setIsAvatarUploadPending}
            onSelectAvatar={setAvatarUrl}
          />

          <div className="space-y-5">
            {/* Agent name */}
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="edit-agent-name"
              >
                Agent name
              </label>
              <div
                className={cn(
                  "flex min-h-11 items-center px-3",
                  PERSONA_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={updateMutation.isPending}
                  id="edit-agent-name"
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Agent name"
                  value={name}
                />
              </div>
            </div>

            {/* Who can talk to this agent */}
            <CreateAgentRespondToField
              allowlist={respondToAllowlist}
              disabled={updateMutation.isPending}
              mode={respondTo}
              onAllowlistChange={setRespondToAllowlist}
              onModeChange={setRespondTo}
              variant="persona"
            />

            {/* Provider (runtime) */}
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="edit-agent-runtime"
              >
                Provider
              </label>
              <PersonaDropdownField
                disabled={updateMutation.isPending}
                id="edit-agent-runtime"
                onValueChange={handleRuntimeDropdownChange}
                options={runtimeDropdownOptions}
                placeholder="Choose a provider"
                value={runtimeDropdownValue}
              />
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

            {/* LLM provider */}
            {llmProviderFieldVisible ? (
              <div className="space-y-1.5">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="edit-agent-llm-provider"
                >
                  LLM provider
                  {providerRequired ? (
                    <span className="ml-1 text-destructive" aria-hidden="true">
                      *
                    </span>
                  ) : (
                    <span className={PERSONA_LABEL_OPTIONAL_CLASS}>
                      Optional
                    </span>
                  )}
                </label>
                <PersonaDropdownField
                  disabled={updateMutation.isPending}
                  id="edit-agent-llm-provider"
                  onValueChange={handleProviderDropdownChange}
                  options={providerDropdownOptions}
                  placeholder="Default (auto)"
                  value={providerSelectValue}
                />
                {isCustomProviderEditing ? (
                  <div
                    className={cn(
                      "mt-2 flex min-h-11 items-center px-3",
                      PERSONA_FIELD_SHELL_CLASS,
                    )}
                  >
                    <Input
                      aria-label="Custom provider ID"
                      autoCorrect="off"
                      className={cn(
                        "h-8 px-0 py-0 leading-6",
                        PERSONA_FIELD_CONTROL_CLASS,
                      )}
                      disabled={updateMutation.isPending}
                      id="edit-agent-custom-provider"
                      onChange={(event) => setProvider(event.target.value)}
                      placeholder="Custom provider ID"
                      value={provider}
                    />
                  </div>
                ) : null}
              </div>
            ) : null}

            {/* Model */}
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium text-foreground"
                htmlFor="edit-agent-model"
              >
                Model
                {modelRequired ? (
                  <span className="ml-1 text-destructive" aria-hidden="true">
                    *
                  </span>
                ) : (
                  <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
                )}
              </label>
              <PersonaDropdownField
                disabled={updateMutation.isPending || modelDiscoveryLoading}
                id="edit-agent-model"
                onValueChange={handleModelDropdownChange}
                options={modelDropdownOptions}
                placeholder="Default model"
                value={modelSelectValue}
              />
              {showCustomModelInput ? (
                <div
                  className={cn(
                    "mt-2 flex min-h-11 items-center px-3",
                    PERSONA_FIELD_SHELL_CLASS,
                  )}
                >
                  <Input
                    aria-label="Custom model ID"
                    autoCorrect="off"
                    className={cn(
                      "h-8 px-0 py-0 leading-6",
                      PERSONA_FIELD_CONTROL_CLASS,
                    )}
                    disabled={updateMutation.isPending}
                    id="edit-agent-custom-model"
                    onChange={(event) => setModel(event.target.value)}
                    placeholder="Custom model ID"
                    value={model}
                  />
                </div>
              ) : null}
              <p className="text-xs text-muted-foreground">
                {modelDiscoveryLoading
                  ? "Loading models..."
                  : modelDiscoveryStatus !== null
                    ? modelDiscoveryStatus.message
                    : discoveredModelOptions !== null
                      ? "Saved changes take effect on the next start."
                      : "Select a provider above to see available models."}
              </p>
            </div>

            {/* Advanced settings */}
            <div className="space-y-3">
              <button
                aria-expanded={showAdvancedFields}
                className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={() => setShowAdvancedFields((current) => !current)}
                type="button"
              >
                <span>Advanced</span>
                <ChevronDown
                  className={cn(
                    "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
                    showAdvancedFields && "rotate-180",
                  )}
                />
              </button>

              <AnimatePresence initial={false}>
                {showAdvancedFields ? (
                  <motion.div
                    animate={{ height: "auto", opacity: 1, scale: 1 }}
                    className="origin-top overflow-hidden"
                    exit={{ height: 0, opacity: 0, scale: 0.98 }}
                    initial={{ height: 0, opacity: 0, scale: 0.98 }}
                    key="edit-agent-advanced-fields"
                    transition={advancedFieldsTransition}
                  >
                    <EditAgentAdvancedFields
                      acpCommand={acpCommand}
                      agentArgs={agentArgs}
                      agentCommand={agentCommand}
                      disabled={updateMutation.isPending}
                      envVars={envVars}
                      fileSatisfiedEnvKeys={fileSatisfiedEnvKeys}
                      inheritedEnvVars={inheritedEnvVars}
                      inheritHarness={inheritHarness}
                      linkedPersona={linkedPersona}
                      mcpCommand={mcpCommand}
                      mcpToolsets={mcpToolsets}
                      parallelism={parallelism}
                      relayUrl={relayUrl}
                      requiredEnvKeys={requiredEnvKeys}
                      selectedRuntimeId={selectedRuntimeId}
                      systemPrompt={systemPrompt}
                      turnTimeoutSeconds={turnTimeoutSeconds}
                      onAcpCommandChange={setAcpCommand}
                      onAgentArgsChange={setAgentArgs}
                      onAgentCommandChange={setAgentCommand}
                      onEnvVarsChange={setEnvVars}
                      onInheritHarnessChange={setInheritHarness}
                      onMcpCommandChange={setMcpCommand}
                      onMcpToolsetsChange={setMcpToolsets}
                      onParallelismChange={setParallelism}
                      onRelayUrlChange={setRelayUrl}
                      onSystemPromptChange={setSystemPrompt}
                      onTurnTimeoutChange={setTurnTimeoutSeconds}
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>

            {/* Error */}
            {updateMutation.error instanceof Error ? (
              <p className="text-sm text-destructive">
                {updateMutation.error.message}
              </p>
            ) : null}
          </div>
        </div>
      </ChooserDialogContent>
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
