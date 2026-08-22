/**
 * AgentEditMergedDialogInstanceSection.tsx — Instance-field section for the merged edit surface.
 *
 * Renders instance name, respond-to, run-on summary, runtime pin, LLM
 * provider, model, AI defaults, and the Advanced accordion. Shown only when
 * an instance is present (showInst && inst).
 *
 * Extracted from AgentEditMergedDialog to satisfy the desktop file-size gate.
 */

import type * as React from "react";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import type { ManagedAgent, RespondToMode } from "@/shared/api/types";
import type { AgentPersona } from "@/shared/api/types";
import type { RuntimeConfigSurface } from "@/shared/api/types";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";

import {
  ADVANCED_FIELDS_MOTION_TRANSITION,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { getProviderApiKeyLabel } from "./agentConfigOptions";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { InheritedDefault } from "./bakedEnvHelpers";
import { AgentAiDefaultsNotice } from "./AgentAiDefaults";
import { AgentDefaultsDialog } from "./AgentDefaultsDialog";
import { AdvancedRequiredBadge } from "./AdvancedRequiredBadge";
import { EditAgentAdvancedFields } from "./EditAgentAdvancedFields";
import { EffortPickerField } from "./EffortPickerField";
import { OwnerOnlyAccessField } from "./OwnerOnlyAccessField";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { PersonaModelCombobox } from "./PersonaModelCombobox";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import { RunOnSummarySection } from "./RunOnSummarySection";

// ── Props ─────────────────────────────────────────────────────────────────────

export type AgentEditMergedISectionProps = {
  inst: ManagedAgent;
  showDef: boolean;
  isSaving: boolean;
  runtimeCatalogStatus: "loading" | "error" | "ready";
  // Instance name
  instanceName: string;
  onInstanceNameChange: (value: string) => void;
  // RespondTo
  respondTo: RespondToMode;
  respondToAllowlist: string[];
  onRespondToChange: (value: RespondToMode) => void;
  onAllowlistChange: (value: string[]) => void;
  agentAccessOwnerOnly: boolean | undefined;
  // Runtime pin
  selectedRuntimeId: string;
  runtimeDropdownValue: string;
  instanceRuntimeDropdownOptions: PersonaDropdownOption[];
  onRuntimeChange: (value: string) => void;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  inheritHarness: boolean;
  agentCommand: string;
  onAgentCommandChange: (value: string) => void;
  // LLM provider
  llmProviderFieldVisible: boolean;
  providerSelectValue: string;
  providerDropdownOptions: PersonaDropdownOption[];
  onProviderChange: (value: string) => void;
  isCustomProviderEditing: boolean;
  provider: string;
  onProviderTextChange: (value: string) => void;
  providerRequired: boolean;
  // API key field
  topLevelSecretEnvVar: string | null;
  apiKeyValue: string;
  apiKeyIsInherited: boolean;
  apiKeyInheritedLabel: string;
  apiKeyIsRequired: boolean;
  effectiveProvider: string;
  onApiKeyChange: (key: string, value: string) => void;
  // Model
  modelSelectValue: string;
  modelDropdownOptions: PersonaDropdownOption[];
  onModelChange: (value: string) => void;
  modelDiscoveryLoading: boolean;
  showCustomModelInput: boolean;
  model: string;
  onModelTextChange: (value: string) => void;
  modelStatusMessage: string | null;
  modelRequired: boolean;
  // AI defaults
  inheritedSubmission: {
    model: string | null;
    provider: string | null;
    envVars: Record<string, string>;
  };
  inheritedModelDefault: InheritedDefault;
  inheritedProviderDefault: InheritedDefault;
  aiDefaultsOpen: boolean;
  onAiDefaultsOpenChange: (value: boolean) => void;
  aiDefaultsTriggerRef: React.RefObject<HTMLButtonElement | null>;
  // Advanced
  showAdvancedFields: boolean;
  onShowAdvancedFieldsChange: (value: boolean) => void;
  acpCommand: string;
  agentArgs: string;
  autoRestartOnConfigChange: boolean;
  startOnAppLaunch?: boolean;
  instanceEnvVars: Record<string, string>;
  fileSatisfiedEnvKeys: string[];
  advancedRequiredEnvKeys: readonly string[];
  initialFocus?: EditAgentFocusTarget;
  inheritedEnvVarsForAdvanced: Record<string, string>;
  linkedPersona: AgentPersona | null;
  prospectiveRuntimeId: string;
  prospectiveRuntime: AcpRuntimeCatalogEntry | undefined;
  parallelism: string;
  onAcpCommandChange: (value: string) => void;
  onAgentArgsChange: (value: string) => void;
  onAutoRestartChange: (value: boolean) => void;
  onStartOnAppLaunchChange?: (value: boolean) => void;
  onEnvVarsChange: (value: Record<string, string>) => void;
  onInheritHarnessChange: (value: boolean) => void;
  onParallelismChange: (value: string) => void;
  systemPrompt: string;
  onSystemPromptChange: (value: string) => void;
  effortConfig: RuntimeConfigSurface | undefined;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentEditMergedInstanceSection({
  inst,
  showDef,
  isSaving,
  runtimeCatalogStatus,
  instanceName,
  onInstanceNameChange,
  respondTo,
  respondToAllowlist,
  onRespondToChange,
  onAllowlistChange,
  agentAccessOwnerOnly,
  selectedRuntimeId,
  runtimeDropdownValue,
  instanceRuntimeDropdownOptions,
  onRuntimeChange,
  selectedRuntime,
  inheritHarness,
  agentCommand,
  onAgentCommandChange,
  llmProviderFieldVisible,
  providerSelectValue,
  providerDropdownOptions,
  onProviderChange,
  isCustomProviderEditing,
  provider,
  onProviderTextChange,
  providerRequired,
  topLevelSecretEnvVar,
  apiKeyValue,
  apiKeyIsInherited,
  apiKeyInheritedLabel,
  apiKeyIsRequired,
  effectiveProvider,
  onApiKeyChange,
  modelSelectValue,
  modelDropdownOptions,
  onModelChange,
  modelDiscoveryLoading,
  showCustomModelInput,
  model,
  onModelTextChange,
  modelStatusMessage,
  modelRequired,
  inheritedSubmission,
  inheritedModelDefault,
  inheritedProviderDefault,
  aiDefaultsOpen,
  onAiDefaultsOpenChange,
  aiDefaultsTriggerRef,
  showAdvancedFields,
  onShowAdvancedFieldsChange,
  acpCommand,
  agentArgs,
  autoRestartOnConfigChange,
  startOnAppLaunch,
  instanceEnvVars,
  fileSatisfiedEnvKeys,
  advancedRequiredEnvKeys,
  initialFocus,
  inheritedEnvVarsForAdvanced,
  linkedPersona,
  prospectiveRuntimeId,
  prospectiveRuntime,
  parallelism,
  onAcpCommandChange,
  onAgentArgsChange,
  onAutoRestartChange,
  onStartOnAppLaunchChange,
  onEnvVarsChange,
  onInheritHarnessChange,
  onParallelismChange,
  systemPrompt,
  onSystemPromptChange,
  effortConfig,
}: AgentEditMergedISectionProps) {
  const shouldReduceMotion = useReducedMotion();
  const advancedFieldsTransition = shouldReduceMotion
    ? { duration: 0 }
    : ADVANCED_FIELDS_MOTION_TRANSITION;

  return (
    <>
      {/* Instance name */}
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
            disabled={isSaving}
            id="edit-agent-name"
            onChange={(e) => onInstanceNameChange(e.target.value)}
            placeholder="Agent name"
            value={instanceName}
          />
        </div>
      </div>

      {/* Respond-to / access (I-owned field — editable even when definition is team-managed) */}
      <OwnerOnlyAccessField
        accessLocked={agentAccessOwnerOnly === true}
        allowlist={respondToAllowlist}
        disabled={isSaving}
        mode={respondTo}
        onAllowlistChange={onAllowlistChange}
        onModeChange={onRespondToChange}
      />

      {/* Run-on summary */}
      <RunOnSummarySection backend={inst.backend} />

      {/* Runtime (harness) — I-field pin */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-runtime"
        >
          Provider
        </label>
        <PersonaDropdownField
          disabled={isSaving}
          id="edit-agent-runtime"
          onValueChange={onRuntimeChange}
          options={instanceRuntimeDropdownOptions}
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

      {selectedRuntimeId === "custom" && !inheritHarness ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-command"
          >
            Agent command
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
              disabled={isSaving}
              id="edit-agent-command"
              onChange={(e) => onAgentCommandChange(e.target.value)}
              placeholder="Full path or shell command"
              value={agentCommand}
            />
          </div>
        </div>
      ) : null}

      {/* LLM provider — I-field only when unlinked (no definition). When
          definition is present, model/provider are D-fields shown in D-section. */}
      {llmProviderFieldVisible && !showDef ? (
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
              <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
            )}
          </label>
          <PersonaDropdownField
            disabled={isSaving}
            id="edit-agent-llm-provider"
            onValueChange={onProviderChange}
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
                disabled={isSaving}
                id="edit-agent-custom-provider"
                onChange={(e) => onProviderTextChange(e.target.value)}
                placeholder="Custom provider ID"
                value={provider}
              />
            </div>
          ) : null}
        </div>
      ) : null}

      {llmProviderFieldVisible && topLevelSecretEnvVar ? (
        <PersonaProviderApiKeyField
          disabled={isSaving}
          envVarName={topLevelSecretEnvVar}
          isInherited={apiKeyIsInherited}
          inheritedLabel={apiKeyInheritedLabel}
          isRequired={apiKeyIsRequired}
          label={getProviderApiKeyLabel(effectiveProvider) ?? "API Key"}
          onValueChange={(next) => {
            onApiKeyChange(topLevelSecretEnvVar, next);
          }}
          value={apiKeyValue}
        />
      ) : null}

      {/* Model — I-field only when unlinked (no definition). When
          definition is present, model is a D-field shown in D-section. */}
      {!showDef ? (
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
          <PersonaModelCombobox
            disabled={isSaving || modelDiscoveryLoading}
            id="edit-agent-model"
            onValueChange={onModelChange}
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
                disabled={isSaving}
                id="edit-agent-custom-model"
                onChange={(e) => onModelTextChange(e.target.value)}
                placeholder="Custom model ID"
                value={model}
              />
            </div>
          ) : null}
          {modelStatusMessage ? (
            <p className="text-xs text-muted-foreground">
              {modelStatusMessage}
            </p>
          ) : null}
        </div>
      ) : null}

      <EffortPickerField agent={inst} config={effortConfig} />

      <AgentAiDefaultsNotice
        onEditDefaults={() => onAiDefaultsOpenChange(true)}
        triggerRef={aiDefaultsTriggerRef}
        explicitModel={inheritedSubmission.model ?? ""}
        explicitProvider={inheritedSubmission.provider ?? ""}
        inheritedModel={inheritedModelDefault}
        inheritedProvider={inheritedProviderDefault}
      />
      <AgentDefaultsDialog
        onOpenChange={onAiDefaultsOpenChange}
        open={aiDefaultsOpen}
        returnFocusRef={aiDefaultsTriggerRef}
      />

      {/* Advanced fields */}
      <div className="space-y-3">
        <button
          aria-expanded={showAdvancedFields}
          className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onShowAdvancedFieldsChange(!showAdvancedFields)}
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
                startOnAppLaunch={startOnAppLaunch}
                disabled={isSaving}
                envVars={instanceEnvVars}
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
                catalogStatus={runtimeCatalogStatus}
                selectedRuntime={prospectiveRuntime}
                systemPrompt={
                  showDef ? systemPrompt : (inst.systemPrompt ?? "")
                }
                onAcpCommandChange={onAcpCommandChange}
                onAgentArgsChange={onAgentArgsChange}
                onAutoRestartChange={onAutoRestartChange}
                onStartOnAppLaunchChange={onStartOnAppLaunchChange}
                onEnvVarsChange={onEnvVarsChange}
                onInheritHarnessChange={onInheritHarnessChange}
                onParallelismChange={onParallelismChange}
                onSystemPromptChange={(v) => {
                  if (!showDef) onSystemPromptChange(v);
                }}
              />
            </motion.div>
          ) : null}
        </AnimatePresence>
      </div>
    </>
  );
}
