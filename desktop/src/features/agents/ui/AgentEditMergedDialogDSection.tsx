/**
 * AgentEditMergedDialogDSection.tsx — Definition-field section for the merged edit surface.
 *
 * Renders agent name, system prompt, and (for definition-only contexts)
 * runtime, LLM provider, and model. Team-managed fields render read-only.
 *
 * Extracted from AgentEditMergedDialog to satisfy the desktop file-size gate.
 */

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { AgentHarnessField } from "./AgentHarnessField";
import { PersonaDropdownField } from "./PersonaDropdownField";

// ── Props ─────────────────────────────────────────────────────────────────────

export type AgentEditMergedDSectionProps = {
  /** When true definition fields are team-managed and rendered read-only. */
  defReadOnly: boolean;
  isSaving: boolean;
  // Identity
  displayName: string;
  onDisplayNameChange: (value: string) => void;
  // Behavior
  systemPrompt: string;
  onSystemPromptChange: (value: string) => void;
  // Runtime (definition-only — hidden when showInst)
  runtimeCatalogStatus: "loading" | "error" | "ready";
  runtimeDropdownValue: string;
  defRuntimeDropdownOptions: PersonaDropdownOption[];
  defBlankLabel: string;
  onRuntimeChange: (value: string) => void;
  // LLM provider (definition-only — hidden when showInst)
  llmProviderFieldVisible: boolean;
  providerSelectValue: string;
  providerDropdownOptions: PersonaDropdownOption[];
  onProviderChange: (value: string) => void;
  isCustomProviderEditing: boolean;
  provider: string;
  onProviderTextChange: (value: string) => void;
  // Model (definition-only — hidden when showInst)
  modelSelectValue: string;
  modelDropdownOptions: PersonaDropdownOption[];
  onModelChange: (value: string) => void;
  modelDiscoveryLoading: boolean;
  showCustomModelInput: boolean;
  model: string;
  onModelTextChange: (value: string) => void;
  modelStatusMessage: string | null;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentEditMergedDSection({
  defReadOnly,
  isSaving,
  displayName,
  onDisplayNameChange,
  systemPrompt,
  onSystemPromptChange,
  runtimeCatalogStatus,
  runtimeDropdownValue,
  defRuntimeDropdownOptions,
  defBlankLabel,
  onRuntimeChange,
  llmProviderFieldVisible,
  providerSelectValue,
  providerDropdownOptions,
  onProviderChange,
  isCustomProviderEditing,
  provider,
  onProviderTextChange,
  modelSelectValue,
  modelDropdownOptions,
  onModelChange,
  modelDiscoveryLoading,
  showCustomModelInput,
  model,
  onModelTextChange,
  modelStatusMessage,
}: AgentEditMergedDSectionProps) {
  return (
    <>
      {/* Agent name */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-display-name"
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
            disabled={isSaving || defReadOnly}
            id="edit-agent-display-name"
            onChange={(e) => onDisplayNameChange(e.target.value)}
            placeholder="Agent name"
            value={displayName}
          />
        </div>
      </div>

      {/* System prompt */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-system-prompt"
        >
          Agent instructions
        </label>
        <div className={PERSONA_FIELD_SHELL_CLASS}>
          <Textarea
            className={cn(
              "min-h-40 resize-y px-3 py-3 leading-5",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={isSaving || defReadOnly}
            id="edit-agent-system-prompt"
            onChange={(e) => onSystemPromptChange(e.target.value)}
            placeholder="Describe what this agent should do."
            value={systemPrompt}
          />
        </div>
      </div>

      {/* Runtime (harness) — D-field, shown for all definition contexts */}
      <AgentHarnessField
        disabled={isSaving || runtimeCatalogStatus === "loading" || defReadOnly}
        onValueChange={onRuntimeChange}
        options={defRuntimeDropdownOptions}
        placeholder={defBlankLabel}
        value={runtimeDropdownValue}
        warning={null}
      />

      {/* LLM provider — D-field */}
      {llmProviderFieldVisible ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-llm-provider"
          >
            LLM provider
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <PersonaDropdownField
            disabled={isSaving || defReadOnly}
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
                disabled={isSaving || defReadOnly}
                id="edit-agent-custom-provider"
                onChange={(e) => onProviderTextChange(e.target.value)}
                placeholder="Custom provider ID"
                value={provider}
              />
            </div>
          ) : null}
        </div>
      ) : null}

      {/* Model — D-field */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-model"
        >
          Model
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        </label>
        <PersonaDropdownField
          disabled={isSaving || modelDiscoveryLoading || defReadOnly}
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
              disabled={isSaving || defReadOnly}
              id="edit-agent-custom-model"
              onChange={(e) => onModelTextChange(e.target.value)}
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
