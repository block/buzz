import * as React from "react";
import { useAgentAccessOwnerOnlyQuery } from "../useAgentAccessOwnerOnly";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  CreateAgentRespondToField,
  OWNER_ONLY_ACCESS_DISABLED_REASON,
} from "./RespondToField";
import type { PersonaBehaviorDraft } from "./personaBehaviorDraft";
import {
  isBuzzAgentRuntime,
  BUZZ_AGENT_THINKING_EFFORT,
} from "./buzzAgentConfig";
import {
  AGENT_PARALLELISM_HELP,
  AGENT_PARALLELISM_PLACEHOLDER,
  parallelismCapHint,
} from "../lib/agentParallelism";
import {
  BuzzAgentModelTuningFields,
  NumericTuningFields,
} from "./buzzAgentModelTuningFields";
import {
  CARD_MINT_KEY_ANNOTATIONS,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  deriveNumericDescriptors,
  structuredEnvKeys,
  type RuntimeCatalogStatus,
} from "../lib/agentConfigCore";

export function PersonaAdvancedFields({
  behaviorDraft,
  disabled,
  envVars,
  afterRespondTo,
  inheritedEnvVars = {},
  model,
  modelTuningRuntimeId = "",
  namePoolText,
  onBehaviorDraftChange,
  onEnvVarsChange,
  onNamePoolTextChange,
  provider,
  requiredEnvKeys = [],
  fileSatisfiedEnvKeys = [],
  hiddenEnvKeys = [],
  catalogStatus = "ready" as RuntimeCatalogStatus,
  selectedRuntime,
}: {
  behaviorDraft: PersonaBehaviorDraft;
  disabled: boolean;
  envVars: EnvVarsValue;
  /** Optional create-only field rendered after instruction permissions. */
  afterRespondTo?: React.ReactNode;
  /** Env vars to display as inherited defaults in tuning-field placeholders.
   *  For templates, pass `globalConfig.env_vars` (the fallback layer). */
  inheritedEnvVars?: EnvVarsValue;
  /** Active LLM model — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  model?: string;
  /** Runtime id for the buzz-agent effort-tuning knob visibility gate. */
  modelTuningRuntimeId?: string;
  namePoolText: string;
  onBehaviorDraftChange: (value: PersonaBehaviorDraft) => void;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onNamePoolTextChange: (value: string) => void;
  /** Active LLM provider id — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  provider?: string;
  requiredEnvKeys?: readonly string[];
  fileSatisfiedEnvKeys?: readonly string[];
  hiddenEnvKeys?: readonly string[];
  /**
   * Lifecycle status of the runtime catalog query. Controls the numeric-tuning
   * gate and hidden-key behaviour:
   * - `loading` or `error`: no structured controls; keys not hidden — saved
   *   values stay visible as generic rows.
   * - `ready`: descriptors derived from `selectedRuntime` (empty when the
   *   runtime has no numeric env-var fields).
   */
  catalogStatus?: RuntimeCatalogStatus;
  /**
   * The catalog entry for the selected runtime. Drives descriptor-based
   * numeric tuning fields. When undefined after the catalog has settled,
   * no numeric controls render.
   */
  selectedRuntime?: AcpRuntimeCatalogEntry;
}) {
  const { data: agentAccessOwnerOnly = false } = useAgentAccessOwnerOnlyQuery();
  const respondToMode = agentAccessOwnerOnly
    ? "owner-only"
    : (behaviorDraft.respondTo ?? "owner-only");

  // Numeric tuning descriptors — gate on catalog status so that loading/error
  // never collapses to "no controls": keys stay visible as generic rows.
  const numericDescriptors = React.useMemo(
    () =>
      catalogStatus === "ready"
        ? deriveNumericDescriptors(selectedRuntime)
        : [],
    [catalogStatus, selectedRuntime],
  );

  const effectiveHiddenKeys = React.useMemo(
    () => [
      ...hiddenEnvKeys,
      ...(isBuzzAgentRuntime(modelTuningRuntimeId)
        ? [BUZZ_AGENT_THINKING_EFFORT]
        : []),
      ...structuredEnvKeys(numericDescriptors),
    ],
    [hiddenEnvKeys, modelTuningRuntimeId, numericDescriptors],
  );

  // Persona hint: definitions keep a portable requested value across harnesses.
  // When the selected harness has a cap and the draft's parallelism exceeds it,
  // explain that the agent will run at the cap — without clamping the stored value.
  const personaParallelismHint = React.useMemo(() => {
    if (
      selectedRuntime?.maxParallelism === undefined ||
      behaviorDraft.parallelism === ""
    ) {
      return null;
    }
    const requested = parseInt(behaviorDraft.parallelism, 10);
    if (Number.isNaN(requested)) return null;
    return parallelismCapHint(
      selectedRuntime.label,
      selectedRuntime.maxParallelism,
      requested,
    );
  }, [selectedRuntime, behaviorDraft.parallelism]);

  return (
    <div className="space-y-5 pt-2">
      <CreateAgentRespondToField
        allowlist={agentAccessOwnerOnly ? [] : behaviorDraft.respondToAllowlist}
        disabled={disabled || agentAccessOwnerOnly}
        disabledReason={
          agentAccessOwnerOnly ? OWNER_ONLY_ACCESS_DISABLED_REASON : undefined
        }
        mode={respondToMode}
        onAllowlistChange={(allowlist) =>
          onBehaviorDraftChange({
            ...behaviorDraft,
            respondToAllowlist: allowlist,
          })
        }
        onModeChange={(mode) =>
          onBehaviorDraftChange({ ...behaviorDraft, respondTo: mode })
        }
        variant="persona"
      />

      {afterRespondTo}

      <div className="grid gap-5 sm:grid-cols-2">
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="persona-parallelism"
          >
            Parallelism
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              className={cn(
                "h-8 px-0 py-0 leading-6 [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="persona-parallelism"
              inputMode="numeric"
              max={32}
              min={1}
              onChange={(event) =>
                onBehaviorDraftChange({
                  ...behaviorDraft,
                  parallelism: event.target.value,
                })
              }
              placeholder={AGENT_PARALLELISM_PLACEHOLDER}
              type="number"
              value={behaviorDraft.parallelism}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            {AGENT_PARALLELISM_HELP}
          </p>
          {personaParallelismHint !== null ? (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              {personaParallelismHint}
            </p>
          ) : null}
        </div>
      </div>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-name-pool"
        >
          Instance name pool
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCapitalize="words"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="persona-name-pool"
            onChange={(event) => onNamePoolTextChange(event.target.value)}
            placeholder="Birch, Compass, Ridge, Thistle"
            spellCheck={false}
            value={namePoolText}
          />
        </div>
      </div>

      <EnvVarsEditor
        disabled={disabled}
        fileSatisfiedKeys={fileSatisfiedEnvKeys}
        hiddenKeys={effectiveHiddenKeys}
        keyAnnotations={CARD_MINT_KEY_ANNOTATIONS}
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />

      {/* Descriptor-driven numeric tuning knobs — shown when catalog has settled
          and the runtime exposes numeric env-var fields. */}
      {numericDescriptors.length > 0 ? (
        <NumericTuningFields
          descriptors={numericDescriptors}
          envVars={envVars}
          inheritedEnvVars={inheritedEnvVars}
          onEnvVarChange={(key, value) => {
            const next = { ...envVars };
            if (value === "") {
              delete next[key];
            } else {
              next[key] = value;
            }
            onEnvVarsChange(next);
          }}
        />
      ) : null}

      {/* Effort-tuning knob — only shown for buzz-agent. */}
      {isBuzzAgentRuntime(modelTuningRuntimeId) ? (
        <BuzzAgentModelTuningFields
          envVars={envVars}
          inheritedEnvVars={inheritedEnvVars}
          model={model}
          onEnvVarChange={(key, value) => {
            const next = { ...envVars };
            if (value === "") {
              delete next[key];
            } else {
              next[key] = value;
            }
            onEnvVarsChange(next);
          }}
          provider={provider}
        />
      ) : null}
    </div>
  );
}
