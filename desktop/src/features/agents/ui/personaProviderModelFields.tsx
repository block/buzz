/**
 * Shared provider and model field components for agent dialogs.
 *
 * Both CreateAgentDialog (local mode) and EditAgentDialog import these
 * instead of duplicating the picker logic.
 */
import type * as React from "react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { Input } from "@/shared/ui/input";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getModelSelectValue,
  getPersonaProviderOptions,
  hasPersonaModelOption,
  type PersonaModelOption,
} from "./personaDialogPickers";
import { MODEL_DISCOVERY_LOADING_VALUE } from "./usePersonaModelDiscovery";
import type { PersonaModelDiscoveryStatus } from "./personaModelDiscoveryStatus";

export function RequiredFieldLabel({
  children,
  htmlFor,
  isRequired,
}: {
  children: React.ReactNode;
  htmlFor: string;
  isRequired: boolean;
}) {
  return (
    <label className="text-sm font-medium" htmlFor={htmlFor}>
      {children}
      {isRequired ? (
        <span className="ml-1 text-destructive" aria-hidden="true">
          *
        </span>
      ) : null}
    </label>
  );
}

export function AgentModelField({
  disabled,
  discoveredModelOptions,
  isCustomModelEditing,
  isRequired,
  model,
  modelDiscoveryLoading,
  modelDiscoveryStatus,
  onIsCustomModelEditingChange,
  onModelChange,
}: {
  disabled: boolean;
  discoveredModelOptions: readonly PersonaModelOption[] | null;
  isCustomModelEditing: boolean;
  isRequired: boolean;
  model: string;
  modelDiscoveryLoading: boolean;
  modelDiscoveryStatus: PersonaModelDiscoveryStatus | null;
  onIsCustomModelEditingChange: (value: boolean) => void;
  onModelChange: (value: string) => void;
}) {
  const trimmedModel = model.trim();

  // Mirror Persona: static options serve as the fallback when discovery hasn't
  // returned yet. Discovered options are ADDITIVE — we never disable the picker
  // or hide the custom input just because discovery returned null.
  const staticModelOptions: readonly PersonaModelOption[] = [
    { id: "", label: "Default model" },
  ];
  const effectiveModelOptions = discoveredModelOptions ?? staticModelOptions;

  // isModelCustom: true when the current model isn't in any known option set.
  // We check discovered options (when available) or runtime-static options so
  // a previously-saved custom model stays in custom mode even before discovery.
  const isModelCustom = !hasPersonaModelOption(
    effectiveModelOptions,
    trimmedModel,
  );

  const modelSelectValue = getModelSelectValue({
    isCustomModelEditing,
    isModelCustom,
    model,
  });

  // The select is only disabled for mutation pending — never for missing discovery.
  // Default/custom options remain usable regardless of discovery state.
  const selectDisabled = disabled || modelDiscoveryLoading;

  // Show the custom model input whenever custom mode is active or the current
  // model is already custom — not gated on discovery having returned.
  const showCustomModelInput = isCustomModelEditing || isModelCustom;

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor="agent-model" isRequired={isRequired}>
        Model
      </RequiredFieldLabel>
      <select
        aria-required={isRequired}
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs disabled:cursor-not-allowed disabled:opacity-60"
        disabled={selectDisabled}
        id="agent-model"
        onChange={(event) => {
          const nextValue = event.target.value;
          if (nextValue === AUTO_MODEL_DROPDOWN_VALUE) {
            onIsCustomModelEditingChange(false);
            onModelChange("");
            return;
          }
          if (nextValue === CUSTOM_MODEL_DROPDOWN_VALUE) {
            onIsCustomModelEditingChange(true);
            return;
          }
          onIsCustomModelEditingChange(false);
          onModelChange(nextValue);
        }}
        value={modelSelectValue}
      >
        {effectiveModelOptions.map((option) => (
          <option
            key={option.id}
            value={option.id || AUTO_MODEL_DROPDOWN_VALUE}
          >
            {option.label}
          </option>
        ))}
        {modelDiscoveryLoading && discoveredModelOptions === null ? (
          <option disabled value={MODEL_DISCOVERY_LOADING_VALUE}>
            Loading models...
          </option>
        ) : null}
        <option value={CUSTOM_MODEL_DROPDOWN_VALUE}>Custom model...</option>
      </select>
      {showCustomModelInput ? (
        <Input
          aria-label="Custom model ID"
          autoCorrect="off"
          disabled={disabled}
          onChange={(event) => onModelChange(event.target.value)}
          placeholder="Custom model ID"
          value={model}
        />
      ) : null}
      <p className="text-xs text-muted-foreground">
        {modelDiscoveryLoading
          ? "Loading models..."
          : modelDiscoveryStatus !== null
            ? modelDiscoveryStatus.message
            : discoveredModelOptions !== null
              ? "Saved changes take effect on the next start."
              : "Select a provider above to see available models."}
      </p>
    </div>
  );
}

export function AgentProviderField({
  disabled,
  isCustomProviderEditing,
  isRequired,
  onProviderChange,
  provider,
  selectedRuntime,
}: {
  disabled: boolean;
  isCustomProviderEditing: boolean;
  isRequired: boolean;
  onProviderChange: (value: string) => void;
  provider: string;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
}) {
  const trimmedProvider = provider.trim();
  const providerOptions = getPersonaProviderOptions(
    trimmedProvider,
    selectedRuntime?.id ?? "",
  );
  const providerSelectValue = isCustomProviderEditing
    ? CUSTOM_PROVIDER_DROPDOWN_VALUE
    : trimmedProvider || AUTO_PROVIDER_DROPDOWN_VALUE;

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor="agent-provider" isRequired={isRequired}>
        LLM provider
      </RequiredFieldLabel>
      <select
        aria-required={isRequired}
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs disabled:cursor-not-allowed disabled:opacity-60"
        disabled={disabled}
        id="agent-provider"
        onChange={(event) => onProviderChange(event.target.value)}
        value={providerSelectValue}
      >
        {providerOptions.map((option) => (
          <option
            key={option.id}
            value={option.id || AUTO_PROVIDER_DROPDOWN_VALUE}
          >
            {option.label}
          </option>
        ))}
        <option value={CUSTOM_PROVIDER_DROPDOWN_VALUE}>
          Custom provider...
        </option>
      </select>
      {isCustomProviderEditing ? (
        <Input
          aria-label="Custom provider ID"
          autoCorrect="off"
          disabled={disabled}
          onChange={(event) => onProviderChange(event.target.value)}
          placeholder="Custom provider ID"
          value={provider}
        />
      ) : null}
      <p className="text-xs text-muted-foreground">
        Changing the provider updates the available model list immediately.
      </p>
    </div>
  );
}
