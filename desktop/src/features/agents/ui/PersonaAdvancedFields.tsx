import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import { CreateAgentRespondToField } from "./RespondToField";
import type { PersonaBehaviorDraft } from "./personaBehaviorDraft";
import { isBuzzAgentRuntime } from "./buzzAgentConfig";
import { BuzzAgentModelTuningFields } from "./buzzAgentModelTuningFields";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

export function PersonaAdvancedFields({
  behaviorDraft,
  disabled,
  envVars,
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
}: {
  behaviorDraft: PersonaBehaviorDraft;
  disabled: boolean;
  envVars: EnvVarsValue;
  /** Env vars to display as inherited defaults in tuning-field placeholders.
   *  For templates, pass `globalConfig.env_vars` (the fallback layer). */
  inheritedEnvVars?: EnvVarsValue;
  /** Active LLM model — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  model?: string;
  /** Runtime id for the buzz-agent tuning knobs visibility gate. */
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
}) {
  return (
    <div className="space-y-5 pt-2">
      <CreateAgentRespondToField
        allowlist={behaviorDraft.respondToAllowlist}
        disabled={disabled}
        mode={behaviorDraft.respondTo ?? "owner-only"}
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
                "h-8 px-0 py-0 leading-6",
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
              placeholder="1"
              type="number"
              value={behaviorDraft.parallelism}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            How many conversations each running instance handles at once (1–32).
          </p>
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
        hiddenKeys={hiddenEnvKeys}
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />

      {/* Tier-1 buzz-agent model-tuning knobs — only shown for buzz-agent. */}
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
