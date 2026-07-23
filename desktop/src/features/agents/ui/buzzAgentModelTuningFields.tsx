/**
 * Tier-1 buzz-agent model-tuning UI fields.
 *
 * Extracted from CreateAgentDialogSections.tsx (deleted in B5/#1667) to avoid
 * coupling tuning knobs to a legacy create-dialog.  Imported by
 * PersonaAdvancedFields and EditAgentAdvancedFields.
 */
import * as React from "react";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  AgentDropdownSelect,
  type AgentDropdownOption,
} from "./agentConfigControls";
import {
  BUZZ_AGENT_MAX_CONTEXT_TOKENS,
  BUZZ_AGENT_MAX_OUTPUT_TOKENS,
  BUZZ_AGENT_MAX_ROUNDS,
  BUZZ_AGENT_THINKING_EFFORT,
  BUZZ_AGENT_THINKING_EFFORT_VALUES,
  getProviderEffortConfig,
} from "./buzzAgentConfig";

/**
 * Shared effort-select dropdown for the `BUZZ_AGENT_THINKING_EFFORT` env var.
 *
 * Used by both `BuzzAgentModelTuningFields` (per-agent/persona dialogs) and
 * `AgentDefaultsSettingsCard` (global defaults settings card) to ensure a
 * single rendering surface for this control.
 *
 * The caller is responsible for the auto-clear `useEffect` that resets the
 * value when the provider/model changes — `useEffortAutoClear` is provided for
 * that purpose. Keeping the effect in the parent avoids coupling the dropdown
 * render to its parent's state update mechanism (which differs between the
 * env-vars map pattern and the `setConfig` pattern).
 */
export function EffortSelectField({
  currentEffort,
  disabled = false,
  emptyOptionLabel,
  effortDefault,
  effortValid,
  fieldClassName,
  htmlFor,
  inheritedEffort,
  inheritFallbackLabel,
  label,
  labelClassName,
  onChange,
  placeholderClassName,
  selectClassName,
  showUnavailableOptions = true,
  testId,
  useCustomSelect = false,
}: {
  /** Current effort value from env vars ("" = inherit). */
  currentEffort: string;
  /** Disable the dropdown. */
  disabled?: boolean;
  /** Optional replacement label for the empty/inherit option. */
  emptyOptionLabel?: string;
  /** Semantic default for this provider/model combination, or null for manual-budget. */
  effortDefault: string | null;
  /** Valid effort values for this provider/model. */
  effortValid: ReadonlyArray<string>;
  /** Optional class override for the field wrapper. */
  fieldClassName?: string;
  /** `htmlFor` attribute for the label element. */
  htmlFor: string;
  /** Inherited effort from a higher-precedence layer (shown in the Inherit option label). */
  inheritedEffort?: string;
  /**
   * Label for the "Inherit" option when no inherited effort is set and the model
   * has a semantic default (i.e. `effortDefault !== null`).
   *
   * Defaults to `"Inherit"`.
   *
   * Per-agent callers (BuzzAgentModelTuningFields) pass `"Inherit (agent default)"`
   * to preserve the label that appeared before this component was extracted.
   *
   * The global-defaults card (AgentDefaultsSettingsCard) passes
   * `"Default (<effort>)"` when a semantic default exists and nothing is baked
   * in — so OSS users see "Default (medium)" rather than bare "Inherit".
   */
  inheritFallbackLabel?: string;
  /** Label text for the dropdown. */
  label: string;
  /** Optional class override for the label. */
  labelClassName?: string;
  /** Called when the user selects a new value. */
  onChange: (value: string) => void;
  /** Optional class override for placeholder text. */
  placeholderClassName?: string;
  /** Optional class override for the select/trigger. */
  selectClassName?: string;
  /** When false, hide effort values that are not valid for the provider/model. */
  showUnavailableOptions?: boolean;
  /** data-testid attribute for the select element. */
  testId: string;
  /** Render the polished app dropdown instead of the native select. */
  useCustomSelect?: boolean;
}) {
  const inheritLabel = inheritedEffort
    ? `Inherit (${inheritedEffort})`
    : effortDefault === null
      ? "Inherit (default)"
      : (inheritFallbackLabel ?? "Inherit");
  const effortOptions: AgentDropdownOption[] = [
    { label: emptyOptionLabel ?? inheritLabel, value: "" },
    ...BUZZ_AGENT_THINKING_EFFORT_VALUES.flatMap((v) => {
      const isValid = (effortValid as readonly string[]).includes(v);
      if (!showUnavailableOptions && !isValid) return [];
      const isDefault = v === effortDefault;
      return [
        {
          disabled: !isValid,
          label: isDefault ? `${v} (default)` : v,
          value: v,
        },
      ];
    }),
  ];

  return (
    <div className={cn("space-y-1.5", fieldClassName)}>
      <label
        className={cn("text-sm font-medium", labelClassName)}
        htmlFor={htmlFor}
      >
        {label}
      </label>
      {useCustomSelect ? (
        <AgentDropdownSelect
          className={selectClassName}
          disabled={disabled}
          id={htmlFor}
          onValueChange={onChange}
          options={effortOptions}
          placeholderClassName={placeholderClassName}
          placeholderValue={emptyOptionLabel ? "" : undefined}
          testId={testId}
          value={currentEffort}
        />
      ) : (
        <select
          className={cn(
            "flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs disabled:cursor-not-allowed disabled:opacity-60",
            selectClassName,
          )}
          data-testid={testId}
          disabled={disabled}
          id={htmlFor}
          onChange={(event) => onChange(event.target.value)}
          value={currentEffort}
        >
          {effortOptions.map((option) => (
            <option
              disabled={option.disabled}
              key={option.value}
              value={option.value}
            >
              {option.label}
            </option>
          ))}
        </select>
      )}
    </div>
  );
}

/**
 * Auto-clear hook: resets `BUZZ_AGENT_THINKING_EFFORT` to "" (Inherit) when
 * the current value is no longer valid for the new provider/model.
 *
 * Call this in any component that renders `EffortSelectField` and owns the
 * effort env var. `onClear` is called with `""` when the current value is
 * invalid; it should delete the key from the env-vars map.
 *
 * `onClear` is intentionally excluded from deps — it is recreated each render
 * and adding it would cause infinite loops; the effect fires on valid-set
 * changes only.
 */
export function useEffortAutoClear({
  currentEffort,
  effortValid,
  onClear,
}: {
  currentEffort: string;
  effortValid: ReadonlyArray<string>;
  onClear: () => void;
}): void {
  // biome-ignore lint/correctness/useExhaustiveDependencies: onClear excluded intentionally — see comment above
  React.useEffect(() => {
    if (
      currentEffort !== "" &&
      !(effortValid as readonly string[]).includes(currentEffort)
    ) {
      onClear();
    }
  }, [effortValid, currentEffort]);
}

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

  const currentEffort = envVars[BUZZ_AGENT_THINKING_EFFORT] ?? "";
  useEffortAutoClear({
    currentEffort,
    effortValid,
    onClear: () => onEnvVarChange(BUZZ_AGENT_THINKING_EFFORT, ""),
  });

  return (
    <div className="space-y-4">
      <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        buzz-agent model tuning
      </p>

      <div className="grid gap-4 md:grid-cols-2">
        {/* Thinking / Effort */}
        <div className="space-y-1.5">
          <EffortSelectField
            currentEffort={currentEffort}
            effortDefault={effortDefault}
            effortValid={effortValid}
            htmlFor="ba-thinking-effort"
            inheritedEffort={inheritedEnvVars[BUZZ_AGENT_THINKING_EFFORT]}
            inheritFallbackLabel="Inherit (agent default)"
            label="Thinking / Effort"
            onChange={(value) =>
              onEnvVarChange(BUZZ_AGENT_THINKING_EFFORT, value)
            }
            testId="ba-thinking-effort-select"
          />
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
