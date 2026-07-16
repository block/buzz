import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { toast } from "sonner";

import {
  useAcpRuntimesQuery,
  useAgentConfigSurface,
  useBakedBuildEnvKeysQuery,
  usePersonasQuery,
  useStartManagedAgentMutation,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type {
  ManagedAgent,
  RespondToMode,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { setManagedAgentAutoRestart } from "@/shared/api/tauriManagedAgents";
import { EditAgentAdvancedFields } from "./EditAgentAdvancedFields";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  BLOCK_BUILD_HIDDEN_PROVIDER_IDS,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getDefaultLlmModelLabel,
  getDefaultPersonaRuntime,
  getPersonaProviderOptions,
  isMissingRequiredDropdownField,
  NO_RUNTIME_DROPDOWN_VALUE,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  runtimeSupportsLlmProviderSelection,
  shouldClearKnownModelForSelectionScope,
  sortPersonaRuntimes,
  type PersonaDropdownOption,
} from "./personaDialogPickers";
import {
  modelDropdownOptions as buildModelDropdownOptions,
  relayMeshModelPickerState,
} from "./relayMeshModelPicker";
import {
  computeEditAgentFormValidity,
  envVarsEqual,
  isEditAgentProviderSaveValid,
  resolveAgentCommandUpdate,
  resolveInheritedRuntimeSubmission,
  resolveRuntimeProviderCapability,
} from "./personaRuntimeModel";
import {
  selectionOnModelDropdownChange,
  selectionOnProviderDropdownChange,
  selectionOnRuntimeChange,
  type RuntimeModelProviderSelection,
} from "./runtimeModelProviderSelection";
import { AgentCreationPreview } from "./AgentCreationPreview";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { useRequiredCredentialState } from "./useRequiredCredentialState";
import { CreateAgentRespondToField } from "./RespondToField";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  MODEL_DISCOVERY_LOADING_VALUE,
  usePersonaModelDiscovery,
} from "./usePersonaModelDiscovery";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import {
  getBakedModelInheritLabel,
  getBakedProviderInheritLabel,
} from "./bakedEnvHelpers";
import { getProviderApiKeyEnvVar } from "./personaDialogPickers";
import { useAgentDialogDefaults } from "./useAgentDialogDefaults";
import { AgentAiDefaultsNotice } from "./AgentAiDefaults";
import { AgentAiDefaultsDialog } from "./AgentAiDefaultsDialog";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";

const ADVANCED_FIELDS_MOTION_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function AgentInstanceEditDialog({
  agent,
  initialFocus,
  open,
  onEditLinkedPersona,
  onOpenChange,
  onUpdated,
}: {
  agent: ManagedAgent;
  /** Optional field to scroll/focus when the dialog opens from a card deep-link. */
  initialFocus?: EditAgentFocusTarget;
  open: boolean;
  /** Present only when the linked definition is editable (non-built-in,
   * resolved). Caller closes this dialog and enters definition-edit. */
  onEditLinkedPersona?: () => void;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
}) {
  const updateMutation = useUpdateManagedAgentMutation();
  const startMutation = useStartManagedAgentMutation();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const configSurfaceQuery = useAgentConfigSurface(open ? agent.pubkey : null);
  const runtimes = runtimesQuery.data ?? [];

  const [name, setName] = React.useState(agent.name);
  const [aiDefaultsOpen, setAiDefaultsOpen] = React.useState(false);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [relayUrl, setRelayUrl] = React.useState(agent.relayUrl);
  const [acpCommand, setAcpCommand] = React.useState(agent.acpCommand);
  const [agentCommand, setAgentCommand] = React.useState(agent.agentCommand);
  const [originalAgentCommand, setOriginalAgentCommand] = React.useState(
    agent.agentCommand,
  );
  const [inheritHarness, setInheritHarness] = React.useState(
    agent.personaId != null && agent.agentCommandOverride == null,
  );
  const [agentArgs, setAgentArgs] = React.useState(agent.agentArgs.join(","));
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
  const [autoRestartOnConfigChange, setAutoRestartOnConfigChange] =
    React.useState(agent.autoRestartOnConfigChange);
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
      setOriginalAgentCommand(agent.agentCommand);
      setInheritHarness(
        agent.personaId != null && agent.agentCommandOverride == null,
      );
      setAgentArgs(agent.agentArgs.join(","));
      setParallelism(String(agent.parallelism));
      setSystemPrompt(agent.systemPrompt ?? "");
      setModel(agent.model ?? "");
      setIsCustomModelEditing(false);
      setProvider(agent.provider ?? "");
      setIsCustomProviderEditing(false);
      setEnvVars(agent.envVars);
      setAutoRestartOnConfigChange(agent.autoRestartOnConfigChange);
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

  // Build the sorted runtime catalog for the dropdown.
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

  // Resolve the dialog-opening command as the catalog loads. Edit-state runtime
  // ids mutate during selection changes and cannot identify the original state.
  const originalRuntimeSupportsProvider = React.useMemo(() => {
    const originalCommand = originalAgentCommand.trim();
    const matched =
      runtimes.find((r) => r.command?.trim() === originalCommand) ??
      runtimes.find((r) => r.id === originalCommand);
    return runtimeSupportsLlmProviderSelection(matched?.id ?? "");
  }, [runtimes, originalAgentCommand]);

  // The runtime id that will actually be active after submit. When inheriting,
  // resolve from the LINKED PERSONA's runtime — that is what will run once the
  // override is cleared. Deriving from agent.agentCommand here is wrong for a
  // pinned agent that just toggled "Inherit runtime from template": the override
  // (e.g. a Claude pin) is still present on the record, so it would resolve to
  // the old pin instead of the persona's runtime, hiding required credentials.
  // Fall back to the agent.agentCommand dual-match (command path, then id) only
  // when there is no linked persona or its runtime is unset. This single
  // prospective id feeds BOTH the block-save gate (requiredEnvKeys) and the
  // submit path so they never disagree on which runtime is being saved.
  const prospectiveRuntimeId = React.useMemo(() => {
    if (!inheritHarness) {
      return selectedRuntime?.id ?? selectedRuntimeId;
    }
    const personaRuntimeId = linkedPersona?.runtime?.trim();
    if (personaRuntimeId) {
      return (
        runtimes.find((r) => r.id === personaRuntimeId)?.id ?? personaRuntimeId
      );
    }
    return (
      runtimes.find((r) => r.command?.trim() === agent.agentCommand.trim())
        ?.id ??
      runtimes.find((r) => r.id === agent.agentCommand.trim())?.id ??
      // Fall back to the app default runtime so discovery can run for agents
      // whose persona has no runtime set (e.g. freshly-added catalog builtins).
      getDefaultPersonaRuntime(runtimes)?.id ??
      ""
    );
  }, [
    inheritHarness,
    linkedPersona?.runtime,
    runtimes,
    agent.agentCommand,
    selectedRuntime?.id,
    selectedRuntimeId,
  ]);

  const llmProviderFieldVisible =
    runtimeSupportsLlmProviderSelection(prospectiveRuntimeId);

  // One-shot focus: when the dialog opens from a card deep-link, scroll and
  // focus the relevant field. The effect re-runs when `llmProviderFieldVisible`
  // changes so a provider-field focus request fires once the field materializes.
  const normalizedFieldFocusFiredRef = React.useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset guard on these three; llmProviderFieldVisible drives the focus attempt below
  React.useEffect(() => {
    normalizedFieldFocusFiredRef.current = false;
  }, [open, initialFocus, agent.pubkey]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — llmProviderFieldVisible is the availability signal that re-triggers the focus attempt; agent.pubkey handles agent-switch
  React.useEffect(() => {
    if (!open || !initialFocus) return;
    if (initialFocus.type !== "normalized_field") return;
    if (normalizedFieldFocusFiredRef.current) return;

    const targetId =
      initialFocus.field === "provider"
        ? "edit-agent-llm-provider"
        : "edit-agent-model";
    const el = document.getElementById(targetId);
    if (!(el instanceof HTMLElement)) return;

    normalizedFieldFocusFiredRef.current = true;

    const id = requestAnimationFrame(() => {
      el.scrollIntoView({ block: "nearest" });
      el.focus();
    });

    return () => cancelAnimationFrame(id);
  }, [open, initialFocus, agent.pubkey, llmProviderFieldVisible]);

  // Provider + env to PERSIST on submit — also fed to the credential gate so
  // gate, saved record, and spawn snapshot all agree on one resolved value.
  // See resolveInheritedRuntimeSubmission for the inherit/transition contract.
  const inheritedSubmission = React.useMemo(
    () =>
      resolveInheritedRuntimeSubmission({
        inheritHarness,
        // Inherit-transition vs. Default-clear — see resolveInheritedRuntimeSubmission.
        agentWasHarnessPinned: agent.agentCommandOverride != null,
        provider,
        personaProvider: linkedPersona?.provider ?? "",
        model,
        personaModel: linkedPersona?.model ?? null,
        envVars,
        personaEnvVars: inheritedEnvVars,
      }),
    [
      inheritHarness,
      agent.agentCommandOverride,
      provider,
      linkedPersona?.provider,
      model,
      linkedPersona?.model,
      envVars,
      inheritedEnvVars,
    ],
  );

  const {
    globalConfig,
    inheritedDefaults: {
      provider: inheritedProviderDefault,
      model: inheritedModelDefault,
    },
    inheritedEnvVars: inheritedEnvVarsForAdvanced,
  } = useAgentDialogDefaults({ inheritedEnvVars, open });

  // Runtime/provider-required credential state, derived from the PROSPECTIVE
  // post-submit runtime — see the hook for the inherit-transition and
  // Advanced-auto-expand rationale.
  // Pass globalProvider so the hook uses it as a fallback when the per-agent
  // provider is empty (global-provider-only configs must surface required keys).
  // Pass globalEnvVars so keys satisfied by global config are excluded from
  // requiredEnvKeys and do not block Save (display and gate agree).
  const {
    requiredEnvKeys,
    fileSatisfiedEnvKeys,
    requiredEnvKeyMissing,
    settled: credentialSettled,
  } = useRequiredCredentialState({
    open,
    prospectiveRuntimeId,
    provider: inheritedSubmission.provider ?? "",
    globalProvider: inheritedProviderDefault.value,
    envVars: inheritedSubmission.envVars,
    globalEnvVars: globalConfig.env_vars,
    personaEnvVars: inheritHarness ? inheritedEnvVars : undefined,
  });

  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });

  // Merge global env as the base layer so credential keys satisfied via global
  // config (e.g. ANTHROPIC_API_KEY) are available to model discovery. Use
  // `inheritedSubmission.envVars` (the same snapshot the credential gate
  // validates) rather than raw `envVars`, so an inherit-transition that layers
  // in persona env vars is reflected in discovery. Agent-local env takes
  // precedence, matching the agent → global → file spawn-path precedence.
  const envVarsForDiscovery = React.useMemo(
    () => ({ ...globalConfig.env_vars, ...inheritedSubmission.envVars }),
    [globalConfig.env_vars, inheritedSubmission.envVars],
  );
  const effectiveProvider =
    (inheritedSubmission.provider ?? "").trim() ||
    inheritedProviderDefault.value;
  const providerForDiscovery = llmProviderFieldVisible ? effectiveProvider : "";

  const {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus,
  } = usePersonaModelDiscovery({
    envVars: envVarsForDiscovery,
    isCustomProviderEditing,
    modelFieldVisible: true,
    open,
    provider: providerForDiscovery,
    selectedRuntime,
  });

  // D2: derive advancedRequiredEnvKeys for EnvVarsEditor display + auto-open.
  // The full requiredEnvKeys/requiredEnvKeyMissing continue driving Save gating.
  // D2/D3: the top-level API key owns display, while the readiness gate keeps
  // the complete required-key list. The effective snapshot covers persona
  // inheritance during an instance inherit transition.
  const providerApiKeyEnvVar = getProviderApiKeyEnvVar(effectiveProvider);
  const personaSatisfied =
    providerApiKeyEnvVar != null &&
    !(providerApiKeyEnvVar in envVars) &&
    (inheritedEnvVars[providerApiKeyEnvVar] ?? "").length > 0;
  const apiKeyFieldState = useProviderApiKeyFieldState({
    bakedEnvKeys,
    effectiveEnvVars: inheritedSubmission.envVars,
    envVars,
    fileSatisfiedEnvKeys,
    globalEnvVars: globalConfig.env_vars,
    open,
    personaSatisfied,
    provider: effectiveProvider,
    requiredEnvKeys,
    satisfactionSettled: credentialSettled,
    setShowAdvancedFields,
  });
  const {
    advancedRequiredEnvKeys,
    inheritedLabel: apiKeyInheritedLabel,
    isInherited: apiKeyIsInherited,
    isRequired: apiKeyIsRequired,
    secretEnvVar: topLevelSecretEnvVar,
    value: apiKeyValue,
  } = apiKeyFieldState;

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

  const selection: RuntimeModelProviderSelection = {
    provider,
    model,
    isCustomProviderEditing,
    isCustomModelEditing,
    envVars,
  };

  function applySelection(next: RuntimeModelProviderSelection) {
    setProvider(next.provider);
    setModel(next.model);
    setIsCustomProviderEditing(next.isCustomProviderEditing);
    setIsCustomModelEditing(next.isCustomModelEditing);
    setEnvVars(next.envVars);
  }

  function handleRuntimeDropdownChange(nextValue: string) {
    const nextRuntimeId =
      nextValue === NO_RUNTIME_DROPDOWN_VALUE ? "" : nextValue;
    const previousRuntimeId = selectedRuntimeId;
    const nextRuntime = runtimes.find((r) => r.id === nextRuntimeId);

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

    applySelection(
      selectionOnRuntimeChange(selection, {
        previousRuntime: previousRuntimeId,
        nextRuntime: nextRuntime?.id ?? nextRuntimeId,
        nextRuntimeCanChooseProvider: runtimeSupportsLlmProviderSelection(
          nextRuntime?.id ?? nextRuntimeId,
        ),
        lockedRuntimeReset: "full",
      }),
    );
  }

  function handleProviderDropdownChange(nextValue: string) {
    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    if (nextProvider === "relay-mesh" && selectedRuntimeId !== "buzz-agent") {
      handleRuntimeDropdownChange("buzz-agent");
    }
    const nextSelection = selectionOnProviderDropdownChange(selection, {
      runtime:
        nextProvider === "relay-mesh"
          ? "buzz-agent"
          : (selectedRuntime?.id ?? selectedRuntimeId),
      nextValue,
      clearModelWhenApiKeyMissing: false,
    });
    applySelection({
      ...nextSelection,
      model: nextProvider === "relay-mesh" ? "auto" : nextSelection.model,
    });
  }

  function handleModelDropdownChange(nextValue: string) {
    applySelection(
      selectionOnModelDropdownChange(selection, {
        nextValue,
        clearKnownModelOnCustomEntry: false,
        isModelCustom: false,
      }),
    );
  }

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
  }

  const providerValid = isEditAgentProviderSaveValid({
    llmProviderFieldVisible,
    currentProvider: provider,
    originalProvider: agent.provider,
    globalProvider: inheritedProviderDefault.value,
    originalRuntimeSupportsProvider,
  });

  const canSubmit =
    computeEditAgentFormValidity({
      name,
      parallelism,
      agentAcpCommand: agent.acpCommand,
      acpCommand,
      respondTo,
      respondToAllowlistLength: respondToAllowlist.length,
      selectedRuntimeId,
      inheritHarness,
      agentCommand,
      requiredEnvKeyMissing,
    }) &&
    providerValid &&
    !updateMutation.isPending &&
    !isAvatarUploadPending;

  async function handleSubmit() {
    try {
      const parsedParallelism = Number.parseInt(parallelism, 10);
      const parsedArgs = agentArgs
        .split(",")
        .map((v) => v.trim())
        .filter((v) => v.length > 0);
      // Model to persist — from the shared inherited-submission snapshot so a
      // provider-backed inherit-transition carries the persona model (readiness
      // requires one) and a deliberate local model still wins.
      const normalizedModel = inheritedSubmission.model;

      // Harness pin resolution — see resolveAgentCommandUpdate for the full
      // sentinel/pin/no-op contract, including the inherit→pin transition where
      // the prefilled command equals the original but must still be pinned.
      const agentCommandUpdate = resolveAgentCommandUpdate({
        inheritHarness,
        agentCommand,
        originalAgentCommand: agent.agentCommand,
        agentCommandOverride: agent.agentCommandOverride ?? null,
      });

      // Classify the effective post-submit runtime's provider capability as a
      // tri-state: "capable" persists the provider, "locked" clears it (only
      // when we KNOW it's provider-locked, e.g. Claude), "unknown" OMITS it so a
      // transient/custom state never becomes a destructive write. Resolved
      // STATICALLY (by id) so a not-yet-loaded catalog can't misclassify a known
      // runtime as "unknown" — see resolveRuntimeProviderCapability. The runtime
      // id is the shared prospectiveRuntimeId, so submit and the block-save gate
      // always agree on which runtime is being saved.
      const providerRuntimeCapability = resolveRuntimeProviderCapability(
        prospectiveRuntimeId,
        runtimeSupportsLlmProviderSelection(prospectiveRuntimeId),
      );

      // Provider + env to persist — the shared inherited-submission snapshot
      // (same values the credential gate validates), so gate ↔ record ↔ spawn
      // all agree. See resolveInheritedRuntimeSubmission.
      const normalizedSubmitProvider = inheritedSubmission.provider;
      const submitEnvVars = inheritedSubmission.envVars;
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
        // A non-inheriting selection is a deliberate pin — signal it so the
        // backend preserves a Custom/runtime command even when it maps to the
        // linked persona's own runtime (otherwise it would be dropped back to
        // inherit). Omitted (falsy) when inheriting or on a name-only edit.
        harnessOverride:
          agentCommandUpdate != null ? !inheritHarness : undefined,
        agentArgs:
          parsedArgs.join(",") !== agent.agentArgs.join(",")
            ? parsedArgs
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
            ? normalizedSubmitProvider !== (agent.provider ?? null)
              ? normalizedSubmitProvider
              : undefined
            : providerRuntimeCapability === "locked"
              ? (agent.provider ?? null) !== null
                ? null
                : undefined
              : undefined, // "unknown" → omit always
        envVars: envVarsEqual(submitEnvVars, agent.envVars)
          ? undefined
          : submitEnvVars,
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
      if (autoRestartOnConfigChange !== agent.autoRestartOnConfigChange) {
        // Standalone setter (mirrors start-on-app-launch) — not part of
        // UpdateManagedAgentInput, so the frozen update shape stays frozen.
        await setManagedAgentAutoRestart(
          agent.pubkey,
          autoRestartOnConfigChange,
        );
      }
      if (result.profileSyncError) {
        console.warn("Relay profile sync failed:", result.profileSyncError);
      }
      handleOpenChange(false);
      onUpdated?.(result.agent);
      // The auto-restart policy deliberately never fires for a stopped or
      // failing agent (a broken agent must not auto-loop), so an edit meant
      // to FIX one silently waits for a manual start. Offer that start
      // explicitly instead of relying on the user to know the policy.
      if (!isManagedAgentActive(result.agent)) {
        const startedName = result.agent.name;
        toast(`${startedName} saved while stopped.`, {
          action: {
            label: "Start now",
            onClick: () => {
              startMutation.mutate(result.agent.pubkey, {
                onSuccess: () => toast.success(`${startedName} started.`),
                onError: (error) =>
                  toast.error(
                    error instanceof Error
                      ? `${startedName} failed to start: ${error.message}`
                      : `${startedName} failed to start.`,
                  ),
              });
            },
          },
        });
      }
    } catch {
      // React Query stores the error; keep dialog open and render it inline.
    }
  }

  // Model and provider field derived state
  const normalizedConfig = configSurfaceQuery.data?.normalized;
  const modelRequired = isMissingRequiredDropdownField(
    normalizedConfig?.model,
    model,
  );
  const providerRequired = isMissingRequiredDropdownField(
    normalizedConfig?.provider,
    provider,
  );
  const inheritedModelLabel =
    inheritedModelDefault.source === "build"
      ? getBakedModelInheritLabel(inheritedModelDefault.value)
      : getDefaultLlmModelLabel(inheritedModelDefault.value);
  const {
    isRelayMesh,
    options: effectiveModelOptions,
    selectValue: modelSelectValue,
    showCustomInput: showCustomModelInput,
  } = relayMeshModelPickerState({
    discoveredOptions: discoveredModelOptions,
    fallbackOptions: [{ id: "", label: inheritedModelLabel }],
    isCustomEditing: isCustomModelEditing,
    model,
    provider: providerForDiscovery,
  });
  const modelDropdownOptions = buildModelDropdownOptions({
    allowCustom: !isRelayMesh,
    globalModel: isRelayMesh ? undefined : inheritedModelDefault.value,
    globalModelLabel: isRelayMesh ? undefined : inheritedModelLabel,
    loading: modelDiscoveryLoading && discoveredModelOptions === null,
    loadingValue: MODEL_DISCOVERY_LOADING_VALUE,
    options: effectiveModelOptions,
  });

  // Provider field derived state
  const trimmedProvider = provider.trim();
  const hideProviderIds = React.useMemo(
    () =>
      (bakedEnvKeys ?? []).includes("BUZZ_AGENT_PROVIDER")
        ? BLOCK_BUILD_HIDDEN_PROVIDER_IDS
        : new Set<string>(),
    [bakedEnvKeys],
  );
  const providerOptions = getPersonaProviderOptions(
    trimmedProvider,
    selectedRuntime?.id ?? "",
    inheritedProviderDefault.source === "global"
      ? inheritedProviderDefault.value
      : "",
    hideProviderIds,
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : trimmedProvider || AUTO_PROVIDER_DROPDOWN_VALUE;
  const providerDropdownOptions: PersonaDropdownOption[] = [
    ...providerOptions.map((option) => ({
      label:
        option.id === "" && inheritedProviderDefault.source === "build"
          ? getBakedProviderInheritLabel(
              inheritedProviderDefault.value,
              providerOptions,
            )
          : option.label,
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
          {/* Avatar is definition-level identity. hideEditControl suppresses
              the internal pencil badge; the CTA below is the only edit path. */}
          <div className="flex flex-col items-center gap-2">
            <AgentCreationPreview
              avatarUrl={previewAvatarUrl}
              hideEditControl
              label={previewLabel}
              onClearAvatar={() => setAvatarUrl("")}
              onUploadPendingChange={setIsAvatarUploadPending}
              onSelectAvatar={setAvatarUrl}
            />
            {onEditLinkedPersona ? (
              <Button
                className="w-full"
                onClick={() => {
                  handleOpenChange(false);
                  onEditLinkedPersona();
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                Edit avatar
              </Button>
            ) : (
              <p className="text-center text-xs text-muted-foreground">
                Avatar is shared identity
              </p>
            )}
          </div>
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

            {llmProviderFieldVisible && topLevelSecretEnvVar ? (
              <PersonaProviderApiKeyField
                disabled={updateMutation.isPending}
                isInherited={apiKeyIsInherited}
                inheritedLabel={apiKeyInheritedLabel}
                isRequired={apiKeyIsRequired}
                label={
                  effectiveProvider === "anthropic"
                    ? "Anthropic API Key"
                    : "OpenAI API Key"
                }
                onValueChange={(next) => {
                  setEnvVars((prev) => ({
                    ...prev,
                    [topLevelSecretEnvVar]: next,
                  }));
                }}
                value={apiKeyValue}
              />
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

            <AgentAiDefaultsNotice
              onEditDefaults={() => setAiDefaultsOpen(true)}
              triggerRef={aiDefaultsTriggerRef}
              explicitModel={inheritedSubmission.model ?? ""}
              explicitProvider={inheritedSubmission.provider ?? ""}
              inheritedModel={inheritedModelDefault}
              inheritedProvider={inheritedProviderDefault}
            />

            <AgentAiDefaultsDialog
              onOpenChange={setAiDefaultsOpen}
              open={aiDefaultsOpen}
              returnFocusRef={aiDefaultsTriggerRef}
            />

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
                      autoRestartOnConfigChange={autoRestartOnConfigChange}
                      disabled={updateMutation.isPending}
                      envVars={envVars}
                      fileSatisfiedEnvKeys={fileSatisfiedEnvKeys}
                      hiddenEnvKeys={
                        topLevelSecretEnvVar ? [topLevelSecretEnvVar] : []
                      }
                      focusKey={
                        initialFocus?.type === "env_key"
                          ? initialFocus.key
                          : undefined
                      }
                      inheritedEnvVars={inheritedEnvVarsForAdvanced}
                      inheritHarness={inheritHarness}
                      linkedPersona={linkedPersona}
                      model={inheritedSubmission.model ?? ""}
                      modelTuningRuntimeId={prospectiveRuntimeId}
                      parallelism={parallelism}
                      provider={effectiveProvider}
                      relayUrl={relayUrl}
                      requiredEnvKeys={advancedRequiredEnvKeys}
                      selectedRuntimeId={selectedRuntimeId}
                      systemPrompt={systemPrompt}
                      onAcpCommandChange={setAcpCommand}
                      onAgentArgsChange={setAgentArgs}
                      onAgentCommandChange={setAgentCommand}
                      onAutoRestartChange={setAutoRestartOnConfigChange}
                      onEnvVarsChange={setEnvVars}
                      onInheritHarnessChange={setInheritHarness}
                      onParallelismChange={setParallelism}
                      onRelayUrlChange={setRelayUrl}
                      onSystemPromptChange={setSystemPrompt}
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
