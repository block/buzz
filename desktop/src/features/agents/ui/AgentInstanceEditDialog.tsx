import * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  useAcpRuntimesQuery,
  useAgentConfigSurface,
  useBakedBuildEnvKeysQuery,
  usePersonasQuery,
  useStartManagedAgentMutation,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import { agentLocationLabel } from "@/features/agents/lib/agentLocationLabel";
import { providerRecordHarness } from "@/features/agents/lib/pinnedHarness";
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
  EditAgentHarnessFields,
  EditAgentModelField,
} from "./EditAgentHarnessFields";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  hiddenProviderIdsForBuild,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  getDefaultLlmModelLabel,
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
} from "./agentConfigOptions";
import {
  resolveDialogRuntimeId,
  resolveOriginalRuntimeSupportsProvider,
  resolveProspectiveRuntimeId,
} from "./editAgentRuntimeResolution";
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
import { getProviderApiKeyEnvVar } from "./agentConfigOptions";
import { modelFieldStatus } from "./agentAiConfigurationPolicy";
import { useAgentDialogDefaults } from "./useAgentDialogDefaults";
import { AgentAiDefaultsNotice } from "./AgentAiDefaults";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import { useProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import { resolveModelFieldStatusMessage } from "./agentConfigControls";
import { AdvancedRequiredBadge } from "./AdvancedRequiredBadge";
import {
  showAgentProfileSyncWarning,
  showAgentSavedWhileStoppedToast,
} from "./agentProfileSyncWarning";
import { useInstanceModelDefinitionWrite } from "./instanceModelDefinitionWrite";
import { AddCustomHarnessDialog } from "./AddCustomHarnessDialog";
import {
  ADD_CUSTOM_HARNESS_OPTION,
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness";

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
  /** Present only when the linked definition is editable (non-built-in, resolved). Caller closes this dialog and enters definition-edit. */
  onEditLinkedPersona?: () => void;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
}) {
  // The record's own harness pin, or null when it runs on this computer. Every
  // "read the record, not the local catalog" branch below hangs off this one
  // question — see `providerRecordHarness`. Null keeps the local path
  // byte-identical.
  const pinnedHarness = providerRecordHarness(agent);
  // Primitive so the memos below re-run on a real change, not on the new object
  // identity every 5s poll hands them. `null` = local, `""` = a pinned host
  // binary this app cannot name.
  const pinnedRuntimeId = pinnedHarness && (pinnedHarness.id ?? "");
  const updateMutation = useUpdateManagedAgentMutation();
  const startMutation = useStartManagedAgentMutation();
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const configSurfaceQuery = useAgentConfigSurface(open ? agent.pubkey : null);
  const runtimes = runtimesQuery.data ?? [];

  const [name, setName] = React.useState(agent.name);
  const [aiDefaultsOpen, setAiDefaultsOpen] = React.useState(false);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
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
  const [isAddHarnessOpen, setIsAddHarnessOpen] = React.useState(false);
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
      setIsAddHarnessOpen(false);
      runtimeTouched.current = false;
      setSelectedRuntimeId(
        resolveDialogRuntimeId(
          runtimes,
          agent.agentCommand,
          pinnedRuntimeId !== null,
        ) ?? "custom",
      );
      updateMutation.reset();
    }
  }, [open, agent.pubkey]);

  // Re-derive the runtime id when the catalog loads.
  React.useEffect(() => {
    if (!open || runtimeTouched.current || runtimes.length === 0) {
      return;
    }
    const matched = resolveDialogRuntimeId(
      runtimes,
      agent.agentCommand,
      pinnedRuntimeId !== null,
    );
    if (matched) {
      setSelectedRuntimeId(matched);
    }
  }, [open, runtimes, agent.agentCommand, pinnedRuntimeId]);

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
    options.push(ADD_CUSTOM_HARNESS_OPTION);
    return options;
  }, [sortedRuntimes, selectedRuntimeId]);

  // The provider capability of the runtime the dialog OPENED with — a pinned
  // remote harness answers from its own id, never this computer's catalog.
  const originalRuntimeSupportsProvider = React.useMemo(
    () =>
      pinnedRuntimeId !== null
        ? runtimeSupportsLlmProviderSelection(pinnedRuntimeId)
        : resolveOriginalRuntimeSupportsProvider(
            runtimes,
            originalAgentCommand,
          ),
    [pinnedRuntimeId, runtimes, originalAgentCommand],
  );

  // The runtime id that will actually be active after submit — the single value
  // feeding BOTH the block-save credential gate and the submit path, so they can
  // never disagree about which runtime is being saved. See
  // `resolveProspectiveRuntimeId` for the remote/local split.
  const prospectiveRuntimeId = React.useMemo(
    () =>
      resolveProspectiveRuntimeId({
        runtimes,
        pinnedRuntimeId,
        inheritHarness,
        personaRuntimeId: linkedPersona?.runtime,
        agentCommand: agent.agentCommand,
        selectedRuntimeId,
      }),
    [
      runtimes,
      pinnedRuntimeId,
      inheritHarness,
      linkedPersona?.runtime,
      agent.agentCommand,
      selectedRuntimeId,
    ],
  );

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
  // post-submit runtime — see the hook for the inherit-transition rationale.
  // Pass globalProvider so the hook uses it as a fallback when the per-agent
  // provider is empty (global-provider-only configs must surface required keys).
  // Pass globalEnvVars so keys satisfied by global config are excluded from
  // requiredEnvKeys and do not block Save (display and gate agree).
  const { requiredEnvKeys, fileSatisfiedEnvKeys, requiredEnvKeyMissing } =
    useRequiredCredentialState({
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
    // Discovery runs a harness binary on THIS computer. For a pinned record
    // that harness is on the host, so the probe would describe the wrong
    // machine — and its credentials would be spent doing it. Stated here rather
    // than left to the fact that a pinned record selects no local runtime.
    modelFieldVisible: pinnedRuntimeId === null,
    open,
    provider: providerForDiscovery,
    selectedRuntime,
  });

  // D2: derive advancedRequiredEnvKeys for EnvVarsEditor display.
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
    personaSatisfied,
    provider: effectiveProvider,
    requiredEnvKeys,
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
    // A provider-backed record's harness lives on the host and is fixed when the
    // agent is created, so no control renders the dropdown for one. The single
    // remaining caller is the relay-mesh branch below, and a record that can
    // reach it is already pinned to `buzz-agent` (relay-mesh is offered to no
    // other pin). Running the body would overwrite that pin with THIS
    // computer's buzz-agent path and default args, and deploy them to the host.
    if (pinnedRuntimeId !== null) return;
    const action = runtimeDropdownAction(nextValue);
    if (action.kind === "add-custom-harness") {
      setIsAddHarnessOpen(true);
      return;
    }
    const nextRuntimeId = action.runtimeId;
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

  // Routed through the normal change handler so a harness registered inline
  // pins its command and resets model/provider like a hand-picked one. Scoped
  // to `open` so a pending id can't outlive the dialog that started the
  // registration.
  const selectSavedHarness = usePendingHarnessSelection(
    runtimes,
    handleRuntimeDropdownChange,
    open,
  );

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

  // A provider-backed linked record's model has to reach the DEFINITION — the
  // record's own column is never read for one, and this dialog is its only
  // editable surface. See `useInstanceModelDefinitionWrite`.
  const modelDefinitionWrite = useInstanceModelDefinitionWrite({
    isProviderRecord: pinnedRuntimeId !== null,
    personaId: agent.personaId,
    linkedPersona,
    model,
    originalModel: agent.model,
    resetKey: open ? agent.pubkey : null,
  });

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
        // relayUrl deliberately never submitted: the legacy per-record pin is
        // ignored (#2122) and the stored value is preserved as-is.
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
        // Linked instances defer model/provider/systemPrompt to the definition.
        systemPrompt:
          linkedPersona != null
            ? undefined
            : (systemPrompt.trim() || null) !== agent.systemPrompt
              ? systemPrompt.trim() || null
              : undefined,
        model:
          linkedPersona != null
            ? undefined
            : normalizedModel !== (agent.model ?? null)
              ? normalizedModel
              : undefined,
        // Tri-state provider persistence keyed on providerRuntimeCapability:
        //   "capable"  → persist: value if changed, omit if unchanged.
        //   "locked"   → clear: send null if provider was set, else omit.
        //   "unknown"  → omit always (never send null for a transient state).
        // llmProviderFieldVisible is for UX visibility only; not used here.
        provider:
          linkedPersona != null
            ? undefined
            : providerRuntimeCapability === "capable"
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

      // Definition-first: the model write lands before the record update, so a
      // failure surfaces as an error with nothing saved rather than as a record
      // update the owner reads as "the model saved too". The subsequent record
      // update is what re-reads the definition into the summary.
      await modelDefinitionWrite.perform();

      const result = await updateMutation.mutateAsync(input);
      if (autoRestartOnConfigChange !== agent.autoRestartOnConfigChange) {
        // Standalone setter (mirrors start-on-app-launch) — not part of
        // UpdateManagedAgentInput, so the frozen update shape stays frozen.
        await setManagedAgentAutoRestart(
          agent.pubkey,
          autoRestartOnConfigChange,
        );
      }
      showAgentProfileSyncWarning(result.agent.name, result.profileSyncError);
      handleOpenChange(false);
      onUpdated?.(result.agent);
      showAgentSavedWhileStoppedToast(result.agent, (pubkey, handlers) =>
        startMutation.mutate(pubkey, handlers),
      );
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
  const { blocked: modelBlocked, status: modelStatus } = modelFieldStatus({
    catalog: discoveredModelOptions,
    discoveryStatus: modelDiscoveryStatus,
    isTypedEntry: isCustomModelEditing && showCustomModelInput,
    model,
  });
  const modelStatusMessage = modelBlocked
    ? modelStatus?.message
    : resolveModelFieldStatusMessage({
        discoveredModelOptions,
        loading: modelDiscoveryLoading,
        status: modelDiscoveryStatus,
      });

  // Declared after the model catalog so a typed model the harness never
  // offered blocks Save rather than silently resolving to its default.
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
    !modelBlocked &&
    // A model change with no honest destination blocks Save rather than
    // being accepted and dropped.
    !modelDefinitionWrite.blocked &&
    !updateMutation.isPending &&
    !modelDefinitionWrite.isPending &&
    !isAvatarUploadPending;

  // Provider field derived state
  const trimmedProvider = provider.trim();
  const hideProviderIds = hiddenProviderIdsForBuild(bakedEnvKeys);
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

            {/* Harness — the local catalog's dropdown, or this record's own pin */}
            <EditAgentHarnessFields
              agentCommand={agentCommand}
              disabled={updateMutation.isPending}
              locationLabel={agentLocationLabel(agent.backend)}
              onAgentCommandChange={setAgentCommand}
              onRuntimeChange={handleRuntimeDropdownChange}
              pinnedHarness={pinnedHarness}
              runtimeOptions={runtimeDropdownOptions}
              runtimeValue={runtimeDropdownValue}
              selectedRuntime={selectedRuntime}
              showCommandInput={
                selectedRuntimeId === "custom" && !inheritHarness
              }
            />
            {/* Only the local branch above renders a harness dropdown, so only
                it can raise "Add custom harness". */}
            {pinnedHarness ? null : (
              <AddCustomHarnessDialog
                onOpenChange={setIsAddHarnessOpen}
                onSaved={selectSavedHarness}
                open={isAddHarnessOpen}
              />
            )}
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

            <EditAgentModelField
              customModelVisible={showCustomModelInput}
              disabled={updateMutation.isPending}
              discoveryLoading={modelDiscoveryLoading}
              model={model}
              modelBlocked={modelBlocked}
              modelBlockedMessage={modelDefinitionWrite.blockedMessage}
              onModelChange={setModel}
              onModelSelect={handleModelDropdownChange}
              options={modelDropdownOptions}
              pinnedHarness={pinnedHarness}
              required={modelRequired}
              selectValue={modelSelectValue}
              statusMessage={modelStatusMessage}
            />

            <AgentAiDefaultsNotice
              onEditDefaults={() => setAiDefaultsOpen(true)}
              triggerRef={aiDefaultsTriggerRef}
              explicitModel={inheritedSubmission.model ?? ""}
              explicitProvider={inheritedSubmission.provider ?? ""}
              inheritedModel={inheritedModelDefault}
              inheritedProvider={inheritedProviderDefault}
            />

            <AgentDefaultsDialog
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
                <AdvancedRequiredBadge
                  envVars={inheritedSubmission.envVars}
                  requiredEnvKeys={advancedRequiredEnvKeys}
                  testId="edit-agent-advanced-required-badge"
                />
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
                      requiredEnvKeys={advancedRequiredEnvKeys}
                      systemPrompt={systemPrompt}
                      onAcpCommandChange={setAcpCommand}
                      onAgentArgsChange={setAgentArgs}
                      onAutoRestartChange={setAutoRestartOnConfigChange}
                      onEnvVarsChange={setEnvVars}
                      onInheritHarnessChange={setInheritHarness}
                      onParallelismChange={setParallelism}
                      onSystemPromptChange={setSystemPrompt}
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>

            {/* Error — either leg of the save can fail; the definition write
                runs first, so its message is the one to show when both are set. */}
            {modelDefinitionWrite.error ? (
              <p className="text-sm text-destructive">
                {modelDefinitionWrite.error.message}
              </p>
            ) : updateMutation.error instanceof Error ? (
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
