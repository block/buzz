/**
 * Tier-1 buzz-agent model-tuning UI fields.
 *
 * Extracted from CreateAgentDialogSections.tsx (deleted in B5/#1667) to avoid
 * coupling tuning knobs to a legacy create-dialog.  Imported by
 * PersonaAdvancedFields and EditAgentAdvancedFields.
 */
import * as React from "react";
import { Input } from "@/shared/ui/input";
import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  BUZZ_AGENT_MAX_CONTEXT_TOKENS,
  BUZZ_AGENT_MAX_OUTPUT_TOKENS,
  BUZZ_AGENT_MAX_ROUNDS,
  BUZZ_AGENT_THINKING_EFFORT,
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
  getProviderEffortConfig,
} from "./buzzAgentConfig";

export function BuzzAgentModelTuningFields({
  envVars,
  inheritedEnvVars,
  model,
  onEnvVarChange,
  provider,
}: {
  envVars: EnvVarsValue;
  inheritedEnvVars: EnvVarsValue;
  /** Active LLM model (optional) — used with `provider` for effort filtering. */
  model?: string;
  onEnvVarChange: (key: string, value: string) => void;
  /** Active LLM provider id (optional) — used for effort filtering + default labels. */
  provider?: string;
}) {
  const effortConfig = getProviderEffortConfig(provider ?? "", model);
  const { validValues: effortValid, defaultValue: effortDefault } =
    effortConfig;

  // Auto-clear effort to Inherit when provider/model change makes the current
  // value invalid. Prevents stale invalid values from being silently saved.
  // onEnvVarChange is intentionally excluded from deps — it's recreated each render
  // and adding it would cause infinite loops; the effect fires on valid-set changes.
  const currentEffort = envVars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
  // biome-ignore lint/correctness/useExhaustiveDependencies: see comment above
  React.useEffect(() => {
    if (
      currentEffort !== "" &&
      !(effortValid as readonly string[]).includes(currentEffort)
    ) {
      onEnvVarChange(BUZZ_AGENT_THINKING_EFFORT, "");
    }
  }, [effortValid, currentEffort]);
  return (
    <div className="space-y-4">
      <p className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
        buzz-agent model tuning
      </p>

      <div className="grid gap-4 md:grid-cols-2">
        {/* Thinking / Effort */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="ba-thinking-effort">
            Thinking / Effort
          </label>
          <select
            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
            data-testid="ba-thinking-effort-select"
            id="ba-thinking-effort"
            onChange={(event) =>
              onEnvVarChange(BUZZ_AGENT_THINKING_EFFORT, event.target.value)
            }
            value={envVars[BUZZ_AGENT_THINKING_EFFORT] ?? ""}
          >
            <option value="">
              {inheritedEnvVars[BUZZ_AGENT_THINKING_EFFORT]
                ? `Inherit (${inheritedEnvVars[BUZZ_AGENT_THINKING_EFFORT]})`
                : effortDefault === null
                  ? "Inherit (default)"
                  : "Inherit (agent default)"}
            </option>
            {BUZZ_AGENT_THINKING_EFFORT_VALUES.map((v) => {
              const isValid = (effortValid as readonly string[]).includes(v);
              const isDefault = v === effortDefault;
              return (
                <option disabled={!isValid} key={v} value={v}>
                  {isDefault ? `${v} (default)` : v}
                </option>
              );
            })}
          </select>
          <p
            className="text-xs text-muted-foreground"
            id="help-ba-thinking-effort"
          >
            Controls how much reasoning effort the LLM applies per turn. Leave
            blank to inherit from the global or persona default.
          </p>
        </div>

        {/* Max Rounds */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="ba-max-rounds">
            Max rounds
          </label>
          <Input
            aria-describedby="help-ba-max-rounds"
            autoComplete="off"
            data-testid="ba-max-rounds-input"
            id="ba-max-rounds"
            inputMode="numeric"
            min="0"
            onChange={(event) =>
              onEnvVarChange(BUZZ_AGENT_MAX_ROUNDS, event.target.value)
            }
            placeholder={
              inheritedEnvVars[BUZZ_AGENT_MAX_ROUNDS]
                ? `Inherit (${inheritedEnvVars[BUZZ_AGENT_MAX_ROUNDS]})`
                : "Inherit (agent default)"
            }
            step="1"
            type="number"
            value={envVars[BUZZ_AGENT_MAX_ROUNDS] ?? ""}
          />
          <p className="text-xs text-muted-foreground" id="help-ba-max-rounds">
            Maximum LLM + tool-call rounds per turn. 0 = unlimited. Leave blank
            to inherit.
          </p>
        </div>

        {/* Max Output Tokens */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="ba-max-output-tokens">
            Max output tokens
          </label>
          <Input
            aria-describedby="help-ba-max-output-tokens"
            autoComplete="off"
            data-testid="ba-max-output-tokens-input"
            id="ba-max-output-tokens"
            inputMode="numeric"
            min="1"
            onChange={(event) =>
              onEnvVarChange(BUZZ_AGENT_MAX_OUTPUT_TOKENS, event.target.value)
            }
            placeholder={
              inheritedEnvVars[BUZZ_AGENT_MAX_OUTPUT_TOKENS]
                ? `Inherit (${inheritedEnvVars[BUZZ_AGENT_MAX_OUTPUT_TOKENS]})`
                : "Inherit (agent default)"
            }
            step="1"
            type="number"
            value={envVars[BUZZ_AGENT_MAX_OUTPUT_TOKENS] ?? ""}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-ba-max-output-tokens"
          >
            Maximum tokens the LLM may generate per response. Leave blank to
            inherit.
          </p>
        </div>

        {/* Context Limit */}
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium"
            htmlFor="ba-max-context-tokens"
          >
            Context limit
          </label>
          <Input
            aria-describedby="help-ba-max-context-tokens"
            autoComplete="off"
            data-testid="ba-max-context-tokens-input"
            id="ba-max-context-tokens"
            inputMode="numeric"
            min="1"
            onChange={(event) =>
              onEnvVarChange(BUZZ_AGENT_MAX_CONTEXT_TOKENS, event.target.value)
            }
            placeholder={
              inheritedEnvVars[BUZZ_AGENT_MAX_CONTEXT_TOKENS]
                ? `Inherit (${inheritedEnvVars[BUZZ_AGENT_MAX_CONTEXT_TOKENS]})`
                : "Inherit (agent default)"
            }
            step="1"
            type="number"
            value={envVars[BUZZ_AGENT_MAX_CONTEXT_TOKENS] ?? ""}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-ba-max-context-tokens"
          >
            Maximum context window tokens buzz-agent tracks before a handoff.
            Leave blank to inherit.
          </p>
        </div>
      </div>
    </div>
  );
}
