import * as React from "react";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  CARD_MINT_KEY_ANNOTATIONS,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";
import type { AgentPersona } from "@/shared/api/types";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  BuzzAgentModelTuningFields,
  NumericTuningFields,
} from "./buzzAgentModelTuningFields";
import {
  isBuzzAgentRuntime,
  BUZZ_AGENT_THINKING_EFFORT,
} from "./buzzAgentConfig";
import {
  EDIT_AGENT_PARALLELISM_HELP,
  parallelismCapHint,
} from "../lib/agentParallelism";
import {
  deriveNumericDescriptors,
  structuredEnvKeys,
  type RuntimeCatalogStatus,
} from "../lib/agentConfigCore";

export function EditAgentAdvancedFields({
  acpCommand,
  agentArgs,
  autoRestartOnConfigChange,
  disabled,
  envVars,
  fileSatisfiedEnvKeys,
  hiddenEnvKeys = [],
  focusKey,
  inheritedEnvVars,
  inheritHarness,
  linkedPersona,
  model,
  modelTuningRuntimeId,
  parallelism,
  provider,
  requiredEnvKeys,
  catalogStatus = "ready",
  selectedRuntime,
  systemPrompt,
  onAcpCommandChange,
  onAgentArgsChange,
  onEnvVarsChange,
  onInheritHarnessChange,
  onParallelismChange,
  onAutoRestartChange,
  onSystemPromptChange,
}: {
  acpCommand: string;
  agentArgs: string;
  autoRestartOnConfigChange: boolean;
  disabled: boolean;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys: readonly string[];
  hiddenEnvKeys?: readonly string[];
  /** When set, EnvVarsEditor scrolls and focuses this key's input on mount. */
  focusKey?: string;
  inheritedEnvVars: Record<string, string>;
  inheritHarness: boolean;
  linkedPersona: AgentPersona | null;
  /** Active LLM model — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  model?: string;
  /**
   * The actual/prospective runtime id used to decide whether to show the
   * buzz-agent effort-tuning field. Uses `prospectiveRuntimeId` from
   * EditAgentDialog — the resolved runtime, not the "inherit"/"custom" sentinel.
   */
  modelTuningRuntimeId: string;
  parallelism: string;
  /** Active LLM provider id — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  provider?: string;
  requiredEnvKeys: readonly string[];
  /**
   * Lifecycle status of the runtime catalog query. Controls the numeric-tuning
   * gate and hidden-key behaviour:
   * - `loading` or `error`: no structured controls; keys not hidden — saved
   *   values stay visible as generic rows.
   * - `ready`: descriptors derived from `selectedRuntime` (empty when the
   *   runtime has no numeric env-var fields).
   *
   * Defaults to `"ready"` so existing callers without the catalog query do not
   * need to change.
   */
  catalogStatus?: RuntimeCatalogStatus;
  /**
   * The catalog entry for the prospective runtime. Drives descriptor-based
   * numeric tuning fields (max output tokens / context limit / max rounds).
   * When undefined after the catalog has settled, no numeric controls render.
   */
  selectedRuntime?: AcpRuntimeCatalogEntry;
  systemPrompt: string;
  onAcpCommandChange: (value: string) => void;
  onAgentArgsChange: (value: string) => void;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onInheritHarnessChange: (value: boolean) => void;
  onParallelismChange: (value: string) => void;
  onAutoRestartChange: (value: boolean) => void;
  onSystemPromptChange: (value: string) => void;
}) {
  // Numeric tuning descriptors — gate on catalog status so that loading/error
  // never collapses to "no controls": keys stay visible as generic rows.
  const numericDescriptors = React.useMemo(
    () =>
      catalogStatus === "ready"
        ? deriveNumericDescriptors(selectedRuntime)
        : [],
    [catalogStatus, selectedRuntime],
  );

  // Build the effective hidden-key list: caller's secrets + effort key (when
  // rendered by BuzzAgentModelTuningFields) + numeric keys via structuredEnvKeys.
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

  // Harness cap hint: show only when the selected runtime has a cap and the
  // current parallelism value exceeds it. Cap and label come from the catalog
  // entry — no hardcoded constant in TS.
  const parallelismHint = React.useMemo(() => {
    if (selectedRuntime?.maxParallelism === undefined || parallelism === "") {
      return null;
    }
    const requested = parseInt(parallelism, 10);
    if (Number.isNaN(requested)) return null;
    return parallelismCapHint(
      selectedRuntime.label,
      selectedRuntime.maxParallelism,
      requested,
    );
  }, [selectedRuntime, parallelism]);

  return (
    <div className="space-y-5 pt-2">
      {/* Inherit runtime from template */}
      {linkedPersona ? (
        <div className="space-y-1.5">
          <label
            className="flex items-center gap-2 text-sm font-medium"
            htmlFor="edit-agent-inherit-harness"
          >
            <input
              checked={inheritHarness}
              id="edit-agent-inherit-harness"
              onChange={(event) => onInheritHarnessChange(event.target.checked)}
              type="checkbox"
            />
            Inherit runtime from template
          </label>
          <p className="text-xs text-muted-foreground">
            {inheritHarness
              ? `Uses the ${linkedPersona.displayName} template's runtime${
                  linkedPersona.runtime ? ` (${linkedPersona.runtime})` : ""
                }. Editing the template and respawning propagates the new runtime.`
              : "Pins this agent to a specific runtime command, overriding the template's runtime."}
          </p>
        </div>
      ) : null}

      {/* Auto-restart on config change (Chunk F) */}
      <div className="space-y-1.5">
        <label
          className="flex items-center gap-2 text-sm font-medium"
          htmlFor="edit-agent-auto-restart"
        >
          <input
            checked={autoRestartOnConfigChange}
            id="edit-agent-auto-restart"
            onChange={(event) => onAutoRestartChange(event.target.checked)}
            type="checkbox"
          />
          Auto-restart on config change
        </label>
        <p className="text-xs text-muted-foreground">
          {autoRestartOnConfigChange
            ? "Restarts this agent automatically when its configuration changes, once it is idle and connected."
            : "Configuration changes only show the restart badge; restart manually to apply them."}
        </p>
      </div>

      {/* Agent runtime args */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-args"
        >
          Agent runtime args
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
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
            disabled={disabled}
            id="edit-agent-args"
            onChange={(event) => onAgentArgsChange(event.target.value)}
            placeholder="Comma-separated"
            value={agentArgs}
          />
        </div>
      </div>

      {/* Parallelism */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-parallelism"
        >
          Parallelism
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
            disabled={disabled}
            id="edit-agent-parallelism"
            inputMode="numeric"
            onChange={(event) => onParallelismChange(event.target.value)}
            placeholder="Current value"
            type="text"
            value={parallelism}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          {EDIT_AGENT_PARALLELISM_HELP}
        </p>
        {parallelismHint !== null ? (
          <p className="text-xs text-amber-600 dark:text-amber-400">
            {parallelismHint}
          </p>
        ) : null}
      </div>

      {/* Relay URL: intentionally no editor. The legacy per-record relay pin
          is ignored (#2122 agents-everywhere) — agents always run on the
          active community relay — so offering a knob here would advertise a
          setting with no effect. The stored field is preserved untouched. */}

      {/* ACP command */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-acp-command"
        >
          ACP command
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
            disabled={disabled}
            id="edit-agent-acp-command"
            onChange={(event) => onAcpCommandChange(event.target.value)}
            value={acpCommand}
          />
        </div>
      </div>

      {/* System prompt override — hidden for linked instances; the persona
          definition is authoritative and the backend will reject any override. */}
      {linkedPersona == null && (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-system-prompt"
          >
            System prompt override
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <div className={PERSONA_FIELD_SHELL_CLASS}>
            <Textarea
              className={cn(
                "min-h-24 resize-y px-3 py-3 leading-5",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="edit-agent-system-prompt"
              onChange={(event) => onSystemPromptChange(event.target.value)}
              placeholder="Leave blank to send no ACP system prompt"
              value={systemPrompt}
            />
          </div>
        </div>
      )}

      {/* Env vars */}
      <EnvVarsEditor
        disabled={disabled}
        fileSatisfiedKeys={fileSatisfiedEnvKeys}
        hiddenKeys={effectiveHiddenKeys}
        focusKey={focusKey}
        helperText="Per-agent env vars. Override the template's vars on collision."
        inheritedFrom={inheritedEnvVars}
        inheritedLabel="template / global defaults"
        keyAnnotations={CARD_MINT_KEY_ANNOTATIONS}
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />

      {/* Descriptor-driven numeric tuning knobs — shown when the catalog has settled
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
