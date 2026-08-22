/**
 * AgentEditMergedDialog — single merged edit surface for R1–R7.
 *
 * instance-with-definition → D + I + L sections on one form
 * instance-only            → I + L sections
 * definition-only          → D sections only
 *
 * Submit: emitAgentFormDiff → runAgentSaveCoordinator (observed-state settlement).
 * Submit logic extracted to useAgentEditMergedSubmit; section JSX to
 * AgentEditMergedDialogDSection / AgentEditMergedDialogInstanceSection.
 */

import * as React from "react";

import {
  useAcpRuntimesQuery,
  useAgentConfigSurface,
  useBakedBuildEnvKeysQuery,
  usePersonasQuery,
  useStartManagedAgentMutation,
  useUpdateManagedAgentMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { useTeamsQuery } from "@/features/agents/teamHooks";
import { useUpdatePersonaAndPublishMutation } from "@/features/agents/lib/usePersonaCatalogRelay";
import type { ManagedAgent, RespondToMode } from "@/shared/api/types";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

import {
  seedAgentFormModel,
  hasDefinitionContext,
  hasInstanceContext,
  isDefinitionReadOnly,
  definitionFieldsDirty,
  fieldEditable,
  editContextDefinition,
  editContextInstance,
  type AgentEditContext,
} from "./agentFormModel";
import {
  useAgentEditMergedSubmit,
  buildNextAgentFormModel,
  type AgentEditSubmitState,
} from "./useAgentEditMergedSubmit";
import { AgentEditMergedDSection } from "./AgentEditMergedDialogDSection";
import { AgentEditMergedInstanceSection } from "./AgentEditMergedDialogInstanceSection";
import {
  getDefaultPersonaRuntime,
  isMissingRequiredDropdownField,
} from "./agentConfigOptions";
import { useAgentEditRuntimeHandlers } from "./useAgentEditRuntimeHandlers";
import { AgentCreationPreview } from "./AgentCreationPreview";
import { AddCustomHarnessDialog } from "./AddCustomHarnessDialog";
import { useAgentAccessOwnerOnlyQuery } from "@/features/agents/useAgentAccessOwnerOnly";
import { isEditAgentProviderSaveValid } from "./personaRuntimeModel";
import {
  agentAiConfigurationModeSatisfied,
  initialAgentAiConfigurationMode,
} from "./agentAiConfigurationPolicy";
import type { EnvVarsValue } from "./EnvVarsEditor";
import { formatPersonaNamePoolText } from "./personaDialogState";
import { useAgentEditRuntimeState } from "./useAgentEditRuntimeState";
import { useAgentEditDeepLinkFocus } from "./useAgentEditDeepLinkFocus";

// ── Types / Component ────────────────────────────────────────────────────────

export type AgentEditMergedDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  ctx: AgentEditContext;
  /** Optional field to focus when the dialog opens from a card deep-link. */
  initialFocus?: EditAgentFocusTarget;
  onUpdated?: (agent: ManagedAgent) => void;
  /**
   * Optional pre-save validator (R6 origin permission check).
   * Return a non-null string to abort with an error toast; return null to proceed.
   */
  onValidate?: () => string | null;
  /**
   * Optional initial-values override for definition fields (R6 review mode).
   */
  initialValueOverrides?: Partial<{
    displayName: string;
    systemPrompt: string;
    runtime: string | undefined;
    provider: string | undefined;
    model: string | undefined;
    respondTo: string | undefined;
  }>;
  /**
   * Definition admin actions (Definition admin section, Artifact 4).
   * These are optional — omit when the host doesn't support the action.
   */
  /**
   * Called to delete the definition (custom definition delete).
   * When provided, a "Delete agent" action renders in the definition admin section.
   */
  onDeleteDefinition?: () => void;
  /**
   * For built-in definitions: called to remove from My Agents (14a deactivate).
   * When provided and definition isBuiltIn, "Remove from My Agents" renders.
   */
  onRemoveFromMyAgents?: () => void;
  /**
   * Called to open the share/catalog dialog (row 15 — not shown for built-ins).
   * When provided, a "Share agent" action renders in the definition admin section.
   */
  onShare?: () => void;
  /**
   * Number of linked instances for blast-radius copy in the delete confirmation.
   * Only relevant when onDeleteDefinition is provided.
   */
  linkedInstanceCount?: number;
  /**
   * Whether the definition's identity is archived (14b). When true, an
   * "Archived" flair renders. Save does not unarchive.
   */
  isIdentityArchived?: boolean;
};

export function AgentEditMergedDialog({
  open,
  onOpenChange,
  ctx,
  initialFocus,
  onUpdated,
  onValidate,
  initialValueOverrides,
  onDeleteDefinition,
  onRemoveFromMyAgents,
  onShare,
  linkedInstanceCount = 0,
  isIdentityArchived = false,
}: AgentEditMergedDialogProps) {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);
  const showDef = hasDefinitionContext(ctx);
  const showInst = hasInstanceContext(ctx);
  const defReadOnly = isDefinitionReadOnly(ctx);

  // ── Queries / mutations ───────────────────────────────────────────────────
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const runtimes = runtimesQuery.data ?? [];
  const runtimeCatalogStatus = runtimesQuery.isLoading
    ? ("loading" as const)
    : runtimesQuery.isError
      ? ("error" as const)
      : ("ready" as const);

  const updatePersonaMutation = useUpdatePersonaMutation();
  const updatePersonaAndPublishMutation =
    useUpdatePersonaAndPublishMutation(null);
  const updateMutation = useUpdateManagedAgentMutation();
  const startMutation = useStartManagedAgentMutation();

  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });
  // Effort write control (ported from #4557): the config surface supplies the
  // discovered effort configId/options and current effort tier. Gated on an
  // open dialog with a present instance — effort is a per-instance local write.
  const configSurfaceQuery = useAgentConfigSurface(
    open && inst ? inst.pubkey : null,
  );
  const { data: agentAccessOwnerOnly } = useAgentAccessOwnerOnlyQuery({
    // Query in both instance and definition-only contexts: rows 9–10 say
    // access/parallelism are D-owned in definition-only (shown on the form).
    enabled: open && (showInst || ctx.kind === "definition-only"),
  });

  const personasQuery = usePersonasQuery();
  const teamsQuery = useTeamsQuery();
  const sourceTeamName = React.useMemo(() => {
    if (!def?.sourceTeam) return null;
    const team = teamsQuery.data?.find((t) => t.id === def.sourceTeam);
    return team?.name ?? def.sourceTeam;
  }, [def?.sourceTeam, teamsQuery.data]);
  const linkedPersona = React.useMemo(
    () =>
      inst?.personaId
        ? (personasQuery.data?.find((p) => p.id === inst.personaId) ?? null)
        : null,
    [inst?.personaId, personasQuery.data],
  );

  // ── Form state — seeded from AgentFormModel ───────────────────────────────
  // D-fields
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [systemPrompt, setSystemPrompt] = React.useState("");
  const [namePoolText, setNamePoolText] = React.useState("");
  // Definition runtime (D-field) — separate from instance harness pin (I-field)
  const [definitionRuntimeId, setDefinitionRuntimeId] =
    React.useState("custom");
  // Track whether the D-runtime was auto-seeded (not a deliberate user choice).
  // Prevents a no-op save from persisting the app default as the definition runtime.
  const autoSeededDefinitionRuntimeRef = React.useRef<string | null>(null);
  // I-fields — harness pin state (I-section only, independent of D-runtime)
  const [selectedRuntimeId, setSelectedRuntimeId] = React.useState("custom");
  // D-section LLM model/provider — for the definition (D-owned)
  const [dModel, setDModel] = React.useState("");
  const [dProvider, setDProvider] = React.useState("");
  const [dIsCustomModelEditing, setDIsCustomModelEditing] =
    React.useState(false);
  const [dIsCustomProviderEditing, setDIsCustomProviderEditing] =
    React.useState(false);
  // I-section LLM model/provider — for unlinked agents (I-owned, shown only when !showDef)
  const [iModel, setIModel] = React.useState("");
  const [iProvider, setIProvider] = React.useState("");
  const [iIsCustomModelEditing, setIIsCustomModelEditing] =
    React.useState(false);
  const [iIsCustomProviderEditing, setIIsCustomProviderEditing] =
    React.useState(false);
  const runtimeTouched = React.useRef(false);

  // Convenience: the active model/provider — D-state when a definition is present,
  // I-state when instance-only (no definition). Used for the D-section and for
  // useAgentEditRuntimeState, which is always driven by the relevant layer.
  const model = showDef ? dModel : iModel;
  const provider = showDef ? dProvider : iProvider;
  const isCustomModelEditing = showDef
    ? dIsCustomModelEditing
    : iIsCustomModelEditing;
  const isCustomProviderEditing = showDef
    ? dIsCustomProviderEditing
    : iIsCustomProviderEditing;

  // I-fields
  const [instanceName, setInstanceName] = React.useState("");
  const [respondTo, setRespondTo] = React.useState<RespondToMode | null>(
    inst?.respondTo ?? null,
  );
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    inst?.respondToAllowlist ?? [],
  );
  const [parallelism, setParallelism] = React.useState(
    String(inst?.parallelism ?? 1),
  );
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>(
    def?.envVars ?? inst?.envVars ?? {},
  );
  const [instanceEnvVars, setInstanceEnvVars] = React.useState<EnvVarsValue>(
    inst?.envVars ?? {},
  );
  // Harness-pin (I-field)
  const [agentCommand, setAgentCommand] = React.useState(
    inst?.agentCommand ?? "",
  );
  const [inheritHarness, setInheritHarness] = React.useState(
    inst != null && inst.personaId != null && inst.agentCommandOverride == null,
  );
  const [agentArgs, setAgentArgs] = React.useState(
    inst?.agentArgs.join(",") ?? "",
  );
  const [acpCommand, setAcpCommand] = React.useState(inst?.acpCommand ?? "");
  const [originalAgentCommand, setOriginalAgentCommand] = React.useState(
    inst?.agentCommand ?? "",
  );

  // L-fields
  const [autoRestartOnConfigChange, setAutoRestartOnConfigChange] =
    React.useState(inst?.autoRestartOnConfigChange ?? false);
  const [startOnAppLaunch, setStartOnAppLaunch] = React.useState(
    inst?.startOnAppLaunch ?? false,
  );

  // UI state
  const [showAdvancedFields, setShowAdvancedFields] = React.useState(false);
  const [isAvatarUploadPending, setIsAvatarUploadPending] =
    React.useState(false);
  const [aiDefaultsOpen, setAiDefaultsOpen] = React.useState(false);
  const aiDefaultsTriggerRef = React.useRef<HTMLButtonElement>(null);
  const [isAddHarnessOpen, setIsAddHarnessOpen] = React.useState(false);

  // ── Seed form on open ─────────────────────────────────────────────────────
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — only re-seed on open/entity-switch
  React.useEffect(() => {
    if (!open) return;

    const seed = seedAgentFormModel(ctx);
    setDisplayName(seed.displayName);
    setAvatarUrl(seed.avatarUrl);
    setSystemPrompt(seed.systemPrompt);
    setNamePoolText(formatPersonaNamePoolText(seed.namePool ?? []));
    // D-section model/provider — seed from definition (or instance for I-only)
    setDModel(seed.model ?? "");
    setDProvider(seed.provider ?? "");
    setDIsCustomModelEditing(false);
    setDIsCustomProviderEditing(false);
    // I-section model/provider — only relevant for instance-only context
    setIModel(seed.model ?? "");
    setIProvider(seed.provider ?? "");
    setIIsCustomModelEditing(false);
    setIIsCustomProviderEditing(false);
    // Rows 9–10: seed respondTo/allowlist/parallelism from the canonical seed.
    // Preserve null from the seed — do NOT coerce null→"anyone" here; the
    // seed already handles the instance-vs-definition context. Coercing here
    // would cause phantom D-writes when an unset definition behavior opens.
    setRespondTo((seed.respondTo as RespondToMode | null) ?? null);
    setRespondToAllowlist(seed.respondToAllowlist);
    setParallelism(seed.parallelism != null ? String(seed.parallelism) : "");
    setEnvVars(seed.envVars);
    setInstanceEnvVars(seed.instanceEnvVars ?? {});
    setInstanceName(seed.instanceName ?? "");
    setAutoRestartOnConfigChange(seed.autoRestartOnConfigChange ?? false);
    setStartOnAppLaunch(seed.startOnAppLaunch ?? false);
    setShowAdvancedFields(false);
    setIsAvatarUploadPending(false);
    runtimeTouched.current = false;
    autoSeededDefinitionRuntimeRef.current = null;

    // Seed definition runtime (D-field) from definition record.
    const defRuntimeId = def?.runtime?.trim() ?? "";
    setDefinitionRuntimeId(defRuntimeId || "custom");

    if (inst) {
      setAgentCommand(inst.agentCommand);
      setOriginalAgentCommand(inst.agentCommand);
      setInheritHarness(
        inst.personaId != null && inst.agentCommandOverride == null,
      );
      setAgentArgs(inst.agentArgs.join(","));
      setAcpCommand(inst.acpCommand);
      // I-harness pin: match instance agentCommand to catalog entry
      const matched =
        runtimes.find((r) => r.command?.trim() === inst.agentCommand.trim()) ??
        runtimes.find((r) => r.id === inst.agentCommand.trim());
      setSelectedRuntimeId(matched ? matched.id : "custom");
    } else if (def) {
      // definition-only: also seed I-harness selectedRuntimeId from definition for the
      // D-section dropdown (no instance, so both are the same)
      setSelectedRuntimeId(defRuntimeId || "custom");
    }

    // Apply any caller-supplied initial value overrides (R6 review mode).
    if (initialValueOverrides) {
      if (initialValueOverrides.displayName !== undefined)
        setDisplayName(initialValueOverrides.displayName);
      if (initialValueOverrides.systemPrompt !== undefined)
        setSystemPrompt(initialValueOverrides.systemPrompt);
      if (initialValueOverrides.model !== undefined) {
        setDModel(initialValueOverrides.model ?? "");
        setIModel(initialValueOverrides.model ?? "");
      }
      if (initialValueOverrides.provider !== undefined) {
        setDProvider(initialValueOverrides.provider ?? "");
        setIProvider(initialValueOverrides.provider ?? "");
      }
      if (initialValueOverrides.runtime !== undefined)
        setDefinitionRuntimeId(initialValueOverrides.runtime ?? "custom");
      // R6: carry agent-requested respondTo into definition-only form.
      // This makes the requested change VISIBLE in the review dialog —
      // the owner sees the requested value and can accept (Save) or change it.
      if (initialValueOverrides.respondTo !== undefined)
        setRespondTo(
          (initialValueOverrides.respondTo as RespondToMode | null) ?? null,
        );
    }
  }, [open, inst?.pubkey, def?.id]);

  // Auto-seed D-section runtime when the definition has no runtime configured
  // and the catalog has loaded. Matches AgentDefinitionDialog's auto-seed logic.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — only re-seed when catalog loads or dialog opens
  React.useEffect(() => {
    if (
      !open ||
      !showDef ||
      runtimes.length === 0 ||
      definitionRuntimeId !== "custom" || // already set by the user or seed
      (def?.runtime?.trim() ?? "").length > 0 // definition already has a runtime
    )
      return;
    const defaultRuntime = getDefaultPersonaRuntime(runtimes);
    if (!defaultRuntime) return;
    setDefinitionRuntimeId(defaultRuntime.id);
    // Record that this runtime was auto-seeded, not chosen by the user.
    // useAgentEditMergedSubmit uses this to prevent persisting the auto-seed
    // as a definition runtime change when no other D-fields were modified.
    autoSeededDefinitionRuntimeRef.current = defaultRuntime.id;
    // In definition-only context, selectedRuntimeId is the same conceptual field
    // as definitionRuntimeId (there is no separate I-harness pin). Sync it so
    // that model discovery runs: useAgentEditRuntimeState derives `selectedRuntime`
    // from `selectedRuntimeId`, and a stale "custom" value leaves discovery unable
    // to resolve a runtime → model list stays empty.
    if (!inst) {
      setSelectedRuntimeId(defaultRuntime.id);
    }
  }, [open, runtimes.length, showDef]);

  // Re-derive runtime id when catalog loads
  React.useEffect(() => {
    if (!open || runtimeTouched.current || runtimes.length === 0 || !inst)
      return;
    const matched =
      runtimes.find((r) => r.command?.trim() === inst.agentCommand.trim()) ??
      runtimes.find((r) => r.id === inst.agentCommand.trim());
    if (matched) setSelectedRuntimeId(matched.id);
  }, [open, runtimes, inst?.agentCommand, inst]);

  // ── Runtime / model / provider machinery ──────────────────────────────────
  const {
    selectedRuntime,
    runtimeDropdownValue,
    defRuntimeDropdownValue,
    instanceRuntimeDropdownOptions,
    defRuntimeDropdownOptions,
    defBlankLabel,
    originalRuntimeSupportsProvider,
    prospectiveRuntimeId,
    prospectiveRuntime,
    llmProviderFieldVisible,
    inheritedSubmission,
    inheritedModelDefault,
    inheritedProviderDefault,
    inheritedEnvVarsForAdvanced,
    fileSatisfiedEnvKeys,
    requiredEnvKeyMissing,
    effectiveProvider,
    providerDropdownOptions,
    providerSelectValue,
    modelDropdownOptions,
    modelSelectValue,
    showCustomModelInput,
    modelStatusMessage,
    modelDiscoveryLoading,
    apiKeyValue,
    apiKeyIsInherited,
    apiKeyInheritedLabel,
    apiKeyIsRequired,
    topLevelSecretEnvVar,
    advancedRequiredEnvKeys,
    dAdvanced,
  } = useAgentEditRuntimeState({
    open,
    showDef,
    showInst,
    runtimes,
    runtimeCatalogStatus,
    selectedRuntimeId,
    definitionRuntimeId,
    model,
    provider,
    isCustomModelEditing,
    isCustomProviderEditing,
    envVars,
    instanceEnvVars,
    inheritHarness,
    inst,
    def,
    linkedPersona,
    bakedEnvKeys,
    originalAgentCommand,
    setModel: setDModel,
    setIsCustomModelEditing: setDIsCustomModelEditing,
  });

  // ── Submit validity ────────────────────────────────────────────────────────
  const modelRequired = React.useMemo(
    () => isMissingRequiredDropdownField(undefined, model),
    [model],
  );
  const providerRequired = React.useMemo(
    () => isMissingRequiredDropdownField(undefined, provider),
    [provider],
  );

  // Provider Save-gate.
  //
  // Definition-present context (D-section picker): use the AgentDefinitionDialog
  // gate — when the provider picker is visible and the pair is customized
  // (provider or model set), both provider AND model must be non-empty. The
  // global fallback does NOT satisfy the definition gate (that is the
  // instance-dialog rule), so a runtime-less definition with a saved model and
  // an empty provider keeps Save blocked (test-11 regression pin from 87dc4dccba).
  //
  // Instance-only context: keep isEditAgentProviderSaveValid — the instance
  // record inherits the global provider, so the global fallback is valid there.
  const providerValid = showDef
    ? agentAiConfigurationModeSatisfied(
        initialAgentAiConfigurationMode({ provider, model }),
        { provider, model },
        llmProviderFieldVisible,
      )
    : isEditAgentProviderSaveValid({
        llmProviderFieldVisible,
        currentProvider: provider,
        originalProvider: inst?.provider ?? null,
        globalProvider: inheritedProviderDefault.value,
        originalRuntimeSupportsProvider,
      });

  const parsedParallelism = parseInt(parallelism, 10);

  // ── Submit hook (must come before canSubmit so isSaving is available) ─────
  const submitState = {
    ctx,
    displayName,
    avatarUrl,
    systemPrompt,
    namePoolText,
    model: showDef ? dModel : iModel,
    provider: showDef ? dProvider : iProvider,
    respondTo,
    respondToAllowlist,
    parallelism,
    parsedParallelism,
    envVars,
    instanceEnvVars,
    instanceName,
    autoRestartOnConfigChange,
    startOnAppLaunch: showInst ? startOnAppLaunch : undefined,
    definitionRuntimeId,
    autoSeededDefinitionRuntimeRef,
    selectedRuntimeId,
    inheritHarness,
    agentCommand,
    agentArgs,
    acpCommand,
    showInst,
    defReadOnly,
    inheritedSubmissionProvider: inheritedSubmission.provider ?? null,
    runtimes,
    updatePersona: (
      p: Parameters<typeof updatePersonaMutation.mutateAsync>[0],
    ) => updatePersonaMutation.mutateAsync(p),
    updatePersonaAndPublish: (
      p: Parameters<typeof updatePersonaAndPublishMutation.mutateAsync>[0],
    ) => updatePersonaAndPublishMutation.mutateAsync(p),
    updateManagedAgent: (
      input: Parameters<typeof updateMutation.mutateAsync>[0],
    ) => updateMutation.mutateAsync(input),
    startMutate: (
      pubkey: string,
      cbs: { onSuccess: () => void; onError: (err: unknown) => void },
    ) => startMutation.mutate(pubkey, cbs),
    onValidate,
    onOpenChange,
    onUpdated,
  } satisfies AgentEditSubmitState;

  const {
    isSaving,
    saveError,
    handleSubmit: _handleSubmit,
  } = useAgentEditMergedSubmit(submitState);

  const canSubmit =
    !isSaving &&
    !isAvatarUploadPending &&
    displayName.trim().length > 0 &&
    providerValid &&
    // D-side credential gate: whenever the definition is editable, its required
    // credential keys must be satisfied — independent of the instance gate, so a
    // linked instance pin can't unblock a definition that main would reject.
    // Team-managed definitions (defReadOnly) never block: the user can't change
    // their credentials, so their readiness is not this form's concern.
    (!showDef || defReadOnly || !dAdvanced.requiredEnvKeyMissing) &&
    (showInst
      ? !requiredEnvKeyMissing &&
        (parsedParallelism > 0 || parallelism === "") &&
        (selectedRuntimeId !== "custom" ||
          inheritHarness ||
          agentCommand.trim().length > 0)
      : true);

  // ── Runtime/model/provider handlers (extracted to useAgentEditRuntimeHandlers) ──
  const {
    handleDefinitionRuntimeChange,
    handleRuntimeDropdownChange,
    handleProviderDropdownChange,
    handleModelDropdownChange,
    selectSavedHarness,
  } = useAgentEditRuntimeHandlers({
    showDef,
    showInst,
    dProvider,
    dModel,
    dIsCustomProviderEditing,
    dIsCustomModelEditing,
    setDProvider,
    setDModel,
    setDIsCustomProviderEditing,
    setDIsCustomModelEditing,
    envVars,
    setEnvVars,
    definitionRuntimeId,
    setDefinitionRuntimeId,
    iProvider,
    iModel,
    iIsCustomProviderEditing,
    iIsCustomModelEditing,
    setIProvider,
    setIModel,
    setIIsCustomProviderEditing,
    setIIsCustomModelEditing,
    instanceEnvVars,
    setInstanceEnvVars,
    selectedRuntimeId,
    setSelectedRuntimeId,
    setInheritHarness,
    setAgentCommand,
    setAgentArgs,
    runtimeTouched,
    setIsAddHarnessOpen,
    runtimes,
    selectedRuntime,
    open,
  });

  // ── Focus target (R2) ──────────────────────────────────────────────────────
  const { onOpenAutoFocus } = useAgentEditDeepLinkFocus({
    open,
    initialFocus,
    contextKey: `${inst?.pubkey ?? ""}:${def?.id ?? ""}`,
    llmProviderFieldVisible,
    modelDiscoveryLoading,
    setShowAdvancedFields,
  });

  // ── Render ─────────────────────────────────────────────────────────────────
  function handleSubmit() {
    void _handleSubmit(canSubmit);
  }

  const previewLabel = displayName.trim() || "Agent name";
  const previewAvatarUrl = avatarUrl.trim() || null;

  const dialogTitle = inst
    ? `Edit ${inst.name}`
    : def
      ? `Edit ${def.displayName}`
      : "Edit agent";

  // Catalog publish affordance: only shown when the definition is shared
  // and the user has dirtied at least one D-owned field. Derived from the
  // canonical model diff (definitionFieldsDirty over fieldOwner) rather than a
  // parallel dirty boolean — the same authority that routes emits.
  const dFieldsDirty =
    showDef &&
    !defReadOnly &&
    (() => {
      const seed = seedAgentFormModel(ctx);
      return definitionFieldsDirty(
        seed,
        buildNextAgentFormModel(seed, submitState),
        ctx,
      );
    })();

  const publishesCatalogUpdates = !!(
    def?.shared &&
    !defReadOnly &&
    dFieldsDirty
  );

  return (
    <Dialog
      onOpenChange={(next) => {
        if (!isSaving) onOpenChange(next);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-3xl border-0"
        contentClassName="pt-3"
        data-testid="edit-agent-dialog"
        footerClassName="border-t-0 pt-0"
        headerClassName="pb-2"
        onOpenAutoFocus={onOpenAutoFocus}
        title={dialogTitle}
        footer={
          <div className="flex w-full flex-wrap items-center justify-between gap-3">
            <div className="flex min-h-9 min-w-0 flex-wrap items-center gap-3">
              {publishesCatalogUpdates ? (
                <p
                  className="max-w-sm text-xs text-muted-foreground"
                  data-testid="edit-agent-dialog-catalog-publish-notice"
                >
                  This agent is in the community catalog. Your changes will be
                  published when you save.
                </p>
              ) : null}
            </div>
            <div className="flex items-center gap-2">
              <Button
                disabled={isSaving || isAvatarUploadPending}
                onClick={() => onOpenChange(false)}
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
                {isSaving
                  ? "Saving..."
                  : publishesCatalogUpdates
                    ? "Save and publish"
                    : "Save changes"}
              </Button>
            </div>
          </div>
        }
      >
        <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
          {/* ── Identity column: avatar + preview ────────────────────── */}
          <div className="flex flex-col items-center gap-2">
            {/* Row 2 contract: unlinked agent (instance-only) → avatar is read-only.
                Backend has no avatarUrl on UpdateManagedAgentInput, so editing is
                impossible. Show disabled avatar + tooltip explaining the gap. */}
            {!showDef ? (
              <div className="flex flex-col items-center gap-1">
                <AgentCreationPreview
                  avatarUrl={previewAvatarUrl}
                  disabled
                  label={previewLabel}
                  onClearAvatar={() => {}}
                  onUploadPendingChange={() => {}}
                  onSelectAvatar={() => {}}
                />
                <p className="text-xs text-muted-foreground text-center">
                  Avatar is shared identity — edit via the agent definition.
                </p>
              </div>
            ) : (
              <AgentCreationPreview
                avatarUrl={previewAvatarUrl}
                disabled={
                  isSaving ||
                  isAvatarUploadPending ||
                  !fieldEditable("avatarUrl", ctx)
                }
                label={previewLabel}
                onClearAvatar={() => setAvatarUrl("")}
                onUploadPendingChange={setIsAvatarUploadPending}
                onSelectAvatar={setAvatarUrl}
              />
            )}
          </div>

          {/* ── Main form column ──────────────────────────────────────── */}
          <div className="space-y-5">
            {/* Team-managed notice */}
            {defReadOnly && showDef ? (
              <p
                className="text-sm text-muted-foreground"
                data-testid="team-managed-notice"
              >
                Managed by team{sourceTeamName ? ` ${sourceTeamName}` : ""}. Its
                configuration cannot be edited here.
              </p>
            ) : null}

            {/* Built-in notice */}
            {def?.isBuiltIn && !defReadOnly ? (
              <p
                className="text-sm text-muted-foreground"
                data-testid="builtin-notice"
              >
                Built-in agent — fully editable.
              </p>
            ) : null}

            {/* Identity-archived flair (14b) — per agent pubkey, shown for any
                archived agent regardless of definition linkage. Fields remain
                editable; Save does not unarchive. */}
            {isIdentityArchived ? (
              <p
                className="text-sm font-medium text-amber-600 dark:text-amber-400"
                data-testid="identity-archived-flair"
              >
                Archived — this agent's identity is archived on the relay. Save
                will update its profile but will not unarchive.
              </p>
            ) : null}

            {/* ── D-section: Identity + Behavior + Runtime ──────────── */}
            {showDef ? (
              <AgentEditMergedDSection
                fieldEditable={(field) => fieldEditable(field, ctx)}
                isSaving={isSaving}
                displayName={displayName}
                onDisplayNameChange={setDisplayName}
                systemPrompt={systemPrompt}
                onSystemPromptChange={setSystemPrompt}
                namePoolText={namePoolText}
                onNamePoolTextChange={setNamePoolText}
                envVars={envVars}
                onEnvVarsChange={setEnvVars}
                runtimeCatalogStatus={runtimeCatalogStatus}
                runtimeDropdownValue={defRuntimeDropdownValue}
                defRuntimeDropdownOptions={defRuntimeDropdownOptions}
                defBlankLabel={defBlankLabel}
                onRuntimeChange={(v) => {
                  // An explicit selection is a user choice, not an auto-seed —
                  // clear the ref so submit persists it even when the chosen
                  // runtime equals the previously auto-seeded default.
                  autoSeededDefinitionRuntimeRef.current = null;
                  handleDefinitionRuntimeChange(v);
                }}
                llmProviderFieldVisible={llmProviderFieldVisible}
                providerSelectValue={providerSelectValue}
                providerDropdownOptions={providerDropdownOptions}
                onProviderChange={handleProviderDropdownChange}
                isCustomProviderEditing={isCustomProviderEditing}
                provider={provider}
                onProviderTextChange={setDProvider}
                modelSelectValue={modelSelectValue}
                modelDropdownOptions={modelDropdownOptions}
                onModelChange={handleModelDropdownChange}
                modelDiscoveryLoading={modelDiscoveryLoading}
                showCustomModelInput={showCustomModelInput}
                model={model}
                onModelTextChange={setDModel}
                modelStatusMessage={modelStatusMessage}
                showInstancePresent={showInst}
                respondTo={respondTo}
                respondToAllowlist={respondToAllowlist}
                onRespondToChange={setRespondTo}
                onAllowlistChange={setRespondToAllowlist}
                agentAccessOwnerOnly={agentAccessOwnerOnly}
                parallelism={parallelism}
                onParallelismChange={setParallelism}
                dAdvanced={dAdvanced}
              />
            ) : null}

            {/* ── I-section: Instance + Runtime (when instance present) ── */}
            {showInst && inst ? (
              <AgentEditMergedInstanceSection
                inst={inst}
                showDef={showDef}
                isSaving={isSaving}
                runtimeCatalogStatus={runtimeCatalogStatus}
                instanceName={instanceName}
                onInstanceNameChange={setInstanceName}
                respondTo={respondTo ?? "anyone"}
                respondToAllowlist={respondToAllowlist}
                onRespondToChange={setRespondTo}
                onAllowlistChange={setRespondToAllowlist}
                agentAccessOwnerOnly={agentAccessOwnerOnly}
                selectedRuntimeId={selectedRuntimeId}
                runtimeDropdownValue={runtimeDropdownValue}
                instanceRuntimeDropdownOptions={instanceRuntimeDropdownOptions}
                onRuntimeChange={handleRuntimeDropdownChange}
                selectedRuntime={selectedRuntime}
                inheritHarness={inheritHarness}
                agentCommand={agentCommand}
                onAgentCommandChange={setAgentCommand}
                llmProviderFieldVisible={llmProviderFieldVisible}
                providerSelectValue={providerSelectValue}
                providerDropdownOptions={providerDropdownOptions}
                onProviderChange={handleProviderDropdownChange}
                isCustomProviderEditing={isCustomProviderEditing}
                provider={provider}
                onProviderTextChange={setIProvider}
                providerRequired={providerRequired}
                topLevelSecretEnvVar={topLevelSecretEnvVar}
                apiKeyValue={apiKeyValue}
                apiKeyIsInherited={apiKeyIsInherited}
                apiKeyInheritedLabel={apiKeyInheritedLabel}
                apiKeyIsRequired={apiKeyIsRequired}
                effectiveProvider={effectiveProvider}
                onApiKeyChange={(key, value) => {
                  setInstanceEnvVars((prev) => ({ ...prev, [key]: value }));
                }}
                modelSelectValue={modelSelectValue}
                modelDropdownOptions={modelDropdownOptions}
                onModelChange={handleModelDropdownChange}
                modelDiscoveryLoading={modelDiscoveryLoading}
                showCustomModelInput={showCustomModelInput}
                model={model}
                onModelTextChange={setIModel}
                modelStatusMessage={modelStatusMessage}
                modelRequired={modelRequired}
                inheritedSubmission={inheritedSubmission}
                inheritedModelDefault={inheritedModelDefault}
                inheritedProviderDefault={inheritedProviderDefault}
                aiDefaultsOpen={aiDefaultsOpen}
                onAiDefaultsOpenChange={setAiDefaultsOpen}
                aiDefaultsTriggerRef={aiDefaultsTriggerRef}
                showAdvancedFields={showAdvancedFields}
                onShowAdvancedFieldsChange={setShowAdvancedFields}
                acpCommand={acpCommand}
                agentArgs={agentArgs}
                autoRestartOnConfigChange={autoRestartOnConfigChange}
                instanceEnvVars={instanceEnvVars}
                fileSatisfiedEnvKeys={fileSatisfiedEnvKeys}
                advancedRequiredEnvKeys={advancedRequiredEnvKeys}
                initialFocus={initialFocus}
                inheritedEnvVarsForAdvanced={inheritedEnvVarsForAdvanced}
                linkedPersona={linkedPersona}
                prospectiveRuntimeId={prospectiveRuntimeId}
                prospectiveRuntime={prospectiveRuntime}
                parallelism={parallelism}
                onAcpCommandChange={setAcpCommand}
                onAgentArgsChange={setAgentArgs}
                onAutoRestartChange={setAutoRestartOnConfigChange}
                onStartOnAppLaunchChange={setStartOnAppLaunch}
                onEnvVarsChange={setInstanceEnvVars}
                onInheritHarnessChange={setInheritHarness}
                onParallelismChange={setParallelism}
                startOnAppLaunch={startOnAppLaunch}
                systemPrompt={systemPrompt}
                onSystemPromptChange={setSystemPrompt}
                effortConfig={configSurfaceQuery.data}
              />
            ) : null}

            {/* Definition admin section (Artifact 4) — rendered when a definition exists */}
            {showDef && !defReadOnly && def ? (
              <div className="space-y-2 border-t pt-4">
                {/* Share / catalog publish (row 15 — not shown for built-ins) */}
                {onShare && !def.isBuiltIn ? (
                  <div className="flex items-center justify-between">
                    <span className="text-sm text-muted-foreground">
                      Share this agent in the catalog.
                    </span>
                    <Button
                      disabled={isSaving}
                      onClick={onShare}
                      type="button"
                      variant="outline"
                    >
                      Share agent
                    </Button>
                  </div>
                ) : null}
                {/* Custom definition delete with blast-radius copy */}
                {onDeleteDefinition && !def.isBuiltIn ? (
                  <div className="flex items-center justify-between">
                    <span className="text-sm text-muted-foreground">
                      Delete this agent and all{" "}
                      {linkedInstanceCount > 0
                        ? `${linkedInstanceCount} linked agent${linkedInstanceCount === 1 ? "" : "s"}`
                        : "linked agents"}
                      .
                    </span>
                    <Button
                      disabled={isSaving}
                      onClick={onDeleteDefinition}
                      type="button"
                      variant="destructive"
                    >
                      Delete agent
                    </Button>
                  </div>
                ) : null}
                {/* Built-in: Remove from My Agents (14a deactivate) */}
                {onRemoveFromMyAgents && def.isBuiltIn ? (
                  <div className="flex items-center justify-between">
                    <span className="text-sm text-muted-foreground">
                      Remove this built-in agent from My Agents.
                    </span>
                    <Button
                      disabled={isSaving}
                      onClick={onRemoveFromMyAgents}
                      type="button"
                      variant="outline"
                    >
                      Remove from My Agents
                    </Button>
                  </div>
                ) : null}
              </div>
            ) : null}

            {/* Save error */}
            {saveError ? (
              <p className="text-sm text-destructive">{saveError.message}</p>
            ) : null}
          </div>
        </div>
      </ChooserDialogContent>

      {/* AddCustomHarnessDialog at dialog level so it renders in all contexts,
          including definition-only edit where the instance section is absent. */}
      <AddCustomHarnessDialog
        onOpenChange={setIsAddHarnessOpen}
        onSaved={selectSavedHarness}
        open={isAddHarnessOpen}
      />
    </Dialog>
  );
}
