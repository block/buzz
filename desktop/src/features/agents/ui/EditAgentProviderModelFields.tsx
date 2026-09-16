import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";

import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  getProviderApiKeyLabel,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";

/**
 * LLM provider + provider API key + model block of the Edit Agent dialog.
 *
 * Extracted verbatim from `AgentInstanceEditDialog` as one cohesive unit: the
 * provider selection, the top-level API-key pseudo-field that appears for
 * secret-requiring providers, and the model picker that depends on the chosen
 * provider. Purely presentational — all state and handlers are owned by the
 * dialog and passed in; the render is byte-identical to the inlined version.
 */
export function EditAgentProviderModelFields({
  disabled,
  llmProviderFieldVisible,
  providerRequired,
  providerDropdownOptions,
  providerSelectValue,
  onProviderDropdownChange,
  isCustomProviderEditing,
  provider,
  onProviderChange,
  topLevelSecretEnvVar,
  apiKeyIsInherited,
  apiKeyInheritedLabel,
  apiKeyIsRequired,
  effectiveProvider,
  apiKeyValue,
  onApiKeyChange,
  modelRequired,
  modelDiscoveryLoading,
  modelDropdownOptions,
  modelSelectValue,
  onModelDropdownChange,
  showCustomModelInput,
  model,
  onModelChange,
  modelStatusMessage,
}: {
  disabled: boolean;
  llmProviderFieldVisible: boolean;
  providerRequired: boolean;
  providerDropdownOptions: PersonaDropdownOption[];
  providerSelectValue: string;
  onProviderDropdownChange: (value: string) => void;
  isCustomProviderEditing: boolean;
  provider: string;
  onProviderChange: (value: string) => void;
  topLevelSecretEnvVar: string | null;
  apiKeyIsInherited: boolean;
  apiKeyInheritedLabel: string;
  apiKeyIsRequired: boolean;
  effectiveProvider: string;
  apiKeyValue: string;
  onApiKeyChange: (value: string) => void;
  modelRequired: boolean;
  modelDiscoveryLoading: boolean;
  modelDropdownOptions: PersonaDropdownOption[];
  modelSelectValue: string;
  onModelDropdownChange: (value: string) => void;
  showCustomModelInput: boolean;
  model: string;
  onModelChange: (value: string) => void;
  modelStatusMessage: string | null;
}) {
  return (
    <>
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
              <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
            )}
          </label>
          <PersonaDropdownField
            disabled={disabled}
            id="edit-agent-llm-provider"
            onValueChange={onProviderDropdownChange}
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
                disabled={disabled}
                id="edit-agent-custom-provider"
                onChange={(event) => onProviderChange(event.target.value)}
                placeholder="Custom provider ID"
                value={provider}
              />
            </div>
          ) : null}
        </div>
      ) : null}

      {llmProviderFieldVisible && topLevelSecretEnvVar ? (
        <PersonaProviderApiKeyField
          disabled={disabled}
          envVarName={topLevelSecretEnvVar}
          isInherited={apiKeyIsInherited}
          inheritedLabel={apiKeyInheritedLabel}
          isRequired={apiKeyIsRequired}
          label={getProviderApiKeyLabel(effectiveProvider) ?? "API Key"}
          onValueChange={onApiKeyChange}
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
          disabled={disabled || modelDiscoveryLoading}
          id="edit-agent-model"
          onValueChange={onModelDropdownChange}
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
              disabled={disabled}
              id="edit-agent-custom-model"
              onChange={(event) => onModelChange(event.target.value)}
              placeholder="Custom model ID"
              value={model}
            />
          </div>
        ) : null}
        {modelStatusMessage ? (
          <p className="text-xs text-muted-foreground">{modelStatusMessage}</p>
        ) : null}
      </div>
    </>
  );
}
