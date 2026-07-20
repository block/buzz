/**
 * Shared provider and model field components for agent dialogs.
 *
 * Both CreateAgentDialog (local mode) and AgentInstanceEditDialog import these
 * instead of duplicating the picker logic.
 */
import * as React from "react";
import { Check, ChevronDown, Search } from "lucide-react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getDefaultLlmModelLabel,
  getModelSelectValue,
  getPersonaProviderOptions,
  hasPersonaModelOption,
  providerDisplayLabel,
  type PersonaModelOption,
} from "./agentConfigOptions";
import { MODEL_DISCOVERY_LOADING_VALUE } from "./usePersonaModelDiscovery";
import type { PersonaModelDiscoveryStatus } from "./personaModelDiscoveryStatus";

export type AgentDropdownOption = {
  disabled?: boolean;
  label: React.ReactNode;
  value: string;
};

function optionTestId(testId: string | undefined, value: string) {
  if (!testId) return undefined;
  return `${testId}-option-${value || "empty"}`;
}

export function AgentDropdownSelect({
  ariaRequired,
  className,
  disabled = false,
  id,
  onValueChange,
  options,
  placeholder = "Select",
  placeholderClassName,
  placeholderValue,
  searchable = false,
  selectedLabel,
  testId,
  value,
}: {
  ariaRequired?: boolean;
  className?: string;
  disabled?: boolean;
  id: string;
  onValueChange: (value: string) => void;
  options: readonly AgentDropdownOption[];
  placeholder?: string;
  placeholderClassName?: string;
  placeholderValue?: string;
  /** Show a filter input above the options when the list is long. */
  searchable?: boolean;
  selectedLabel?: React.ReactNode;
  testId?: string;
  value: string;
}) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const selectedOption = options.find((option) => option.value === value);
  const isPlaceholderSelection =
    selectedLabel === undefined &&
    (selectedOption === undefined ||
      (placeholderValue !== undefined &&
        selectedOption.value === placeholderValue));

  const showSearch = searchable && options.length > 1;
  const filteredOptions = React.useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (!showSearch || trimmed === "") return options;
    return options.filter((option) =>
      (typeof option.label === "string" ? option.label : option.value)
        .toLowerCase()
        .includes(trimmed),
    );
  }, [options, query, showSearch]);

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) setQuery("");
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <button
          aria-controls={`${id}-listbox`}
          aria-expanded={open}
          aria-haspopup="listbox"
          aria-required={ariaRequired}
          className={cn(
            "flex h-9 w-full items-center justify-between rounded-md border border-input bg-background px-3 py-2 text-left text-sm font-normal shadow-xs transition-colors hover:bg-background/90 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
            className,
          )}
          data-testid={testId}
          data-value={value}
          disabled={disabled}
          id={id}
          role="combobox"
          type="button"
        >
          <span
            className={cn(
              "min-w-0 truncate",
              isPlaceholderSelection &&
                (placeholderClassName ?? "text-foreground/45"),
            )}
          >
            {selectedLabel ?? selectedOption?.label ?? placeholder}
          </span>
          <ChevronDown
            aria-hidden="true"
            className={cn(
              "ml-3 h-4 w-4 shrink-0 transition-transform duration-150",
              disabled ? "text-foreground/30" : "text-foreground",
              open && "rotate-180",
            )}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="z-[100] max-h-72 overflow-y-auto rounded-2xl border-foreground/10 bg-white p-1.5 text-foreground opacity-100 data-[state=closed]:animate-none data-[state=open]:animate-none"
        sideOffset={8}
        style={{
          boxShadow: "0 16px 36px rgb(0 0 0 / 0.12)",
          width: "var(--radix-popover-trigger-width)",
        }}
      >
        <div
          aria-labelledby={id}
          className="space-y-1"
          id={`${id}-listbox`}
          role="listbox"
        >
          {showSearch ? (
            <div className="relative">
              <Search
                aria-hidden="true"
                className="pointer-events-none absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-foreground/45"
              />
              <Input
                aria-label="Search models"
                autoFocus
                className="h-9 rounded-xl border-foreground/10 bg-white pl-9 text-sm"
                data-testid={testId ? `${testId}-search` : undefined}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search models…"
                value={query}
              />
            </div>
          ) : null}
          {showSearch && filteredOptions.length === 0 ? (
            <p className="px-3 py-2 text-sm text-foreground/55">No matches</p>
          ) : null}
          {filteredOptions.map((option) => {
            const selected = option.value === value;
            return (
              <button
                aria-disabled={option.disabled || undefined}
                aria-selected={selected}
                className={cn(
                  "flex w-full items-center justify-between gap-3 rounded-xl px-3 py-2 text-left text-sm leading-5 text-black opacity-100 transition-colors hover:bg-black/5 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-black/30",
                  selected && "bg-[var(--buzz-welcome-chartreuse)]/35",
                  option.disabled && "cursor-not-allowed text-black/35",
                )}
                data-testid={optionTestId(testId, option.value)}
                data-value={option.value}
                disabled={option.disabled}
                key={option.value}
                onClick={() => {
                  if (option.disabled) return;
                  onValueChange(option.value);
                  setOpen(false);
                }}
                role="option"
                type="button"
              >
                <span className="min-w-0 truncate">{option.label}</span>
                <Check
                  aria-hidden="true"
                  className={cn(
                    "h-4 w-4 shrink-0 text-black transition-opacity",
                    option.disabled && "text-black/35",
                    selected ? "opacity-100" : "opacity-0",
                  )}
                  strokeWidth={2.5}
                />
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}

export function RequiredFieldLabel({
  className,
  children,
  htmlFor,
  isRequired,
}: {
  className?: string;
  children: React.ReactNode;
  htmlFor: string;
  isRequired: boolean;
}) {
  return (
    <label className={cn("text-sm font-medium", className)} htmlFor={htmlFor}>
      {children}
      {isRequired ? (
        <span className="ml-1 text-destructive" aria-hidden="true">
          *
        </span>
      ) : null}
    </label>
  );
}

export function resolveDefaultModelLabel({
  defaultModelLabel,
  discoveredModelOptions,
  globalModel,
  isSharedCompute,
}: {
  defaultModelLabel?: string;
  discoveredModelOptions: readonly PersonaModelOption[] | null;
  globalModel?: string;
  isSharedCompute: boolean;
}) {
  return (
    defaultModelLabel ??
    discoveredModelOptions?.find((option) => option.id.trim() === "")?.label ??
    (isSharedCompute ? "Default (auto)" : getDefaultLlmModelLabel(globalModel))
  );
}

export function AgentModelField({
  disabled,
  discoveredModelOptions,
  globalModel,
  allowDefaultModel = true,
  defaultModelLabel,
  disableSelectDuringDiscovery = true,
  keepSelectedModelValueLabel = false,
  id = "agent-model",
  isCustomModelEditing,
  isRequired,
  model,
  modelDiscoveryLoading,
  modelDiscoveryStatus,
  onIsCustomModelEditingChange,
  onModelChange,
  placeholder = "Select model",
  placeholderClassName,
  provider,
  fieldClassName,
  labelClassName,
  selectClassName,
  testId,
  useCustomSelect = false,
  showStatusMessage = true,
  showCustomModelOption = true,
  useChevronIcon = false,
}: {
  disabled: boolean;
  discoveredModelOptions: readonly PersonaModelOption[] | null;
  /** Known lower-precedence model used when this field is left empty. */
  globalModel?: string;
  /** Hide the empty/default option when no valid lower-precedence model exists. */
  allowDefaultModel?: boolean;
  /** Source-aware label for the empty/default option. */
  defaultModelLabel?: string;
  /** Disable the trigger while live model discovery refreshes the option list. */
  disableSelectDuringDiscovery?: boolean;
  /** Keep the closed trigger from swapping to discovered display labels. */
  keepSelectedModelValueLabel?: boolean;
  /** DOM id for the model select. Defaults to `"agent-model"`. Override in
   *  contexts where multiple instances coexist on the same page (e.g. the
   *  global-config settings card) to avoid duplicate DOM ids. */
  id?: string;
  isCustomModelEditing: boolean;
  isRequired: boolean;
  model: string;
  modelDiscoveryLoading: boolean;
  modelDiscoveryStatus: PersonaModelDiscoveryStatus | null;
  onIsCustomModelEditingChange: (value: boolean) => void;
  onModelChange: (value: string) => void;
  /** Trigger placeholder shown when there is no selected model option. */
  placeholder?: string;
  /** Optional class override for placeholder text. */
  placeholderClassName?: string;
  provider?: string;
  /** Optional class override for the field wrapper. */
  fieldClassName?: string;
  /** Optional class override for the label. */
  labelClassName?: string;
  /** Optional class override for contexts with custom visual treatments. */
  selectClassName?: string;
  /** Optional test id for custom dropdown trigger/options. */
  testId?: string;
  /** Render the polished app dropdown instead of the native select. */
  useCustomSelect?: boolean;
  /** Hide the helper/status line in compact presentations. */
  showStatusMessage?: boolean;
  /** Hide the explicit custom-model escape hatch in compact presentations. */
  showCustomModelOption?: boolean;
  /** Render a controlled chevron instead of the native select indicator. */
  useChevronIcon?: boolean;
}) {
  const trimmedModel = model.trim();
  const isSharedCompute = provider?.trim() === "relay-mesh";
  const discoveredDefaultOption = discoveredModelOptions?.find(
    (option) => option.id.trim() === "",
  );

  // Model discovery can report the harness's current/default model while the
  // persisted value remains empty ("let the harness choose"). Treat that as a
  // real, displayable option instead of a blank select state. Explicit labels
  // from a lower-precedence/baked default still win when present.
  const defaultOption: PersonaModelOption = {
    id: "",
    label: resolveDefaultModelLabel({
      defaultModelLabel,
      discoveredModelOptions,
      globalModel,
      isSharedCompute,
    }),
  };
  const discoveredWithoutDefault = (discoveredModelOptions ?? []).filter(
    (option) => option.id.trim() !== "",
  );
  const baseModelOptions = isSharedCompute
    ? [defaultOption, ...discoveredWithoutDefault]
    : [
        ...(allowDefaultModel || discoveredDefaultOption
          ? [defaultOption]
          : []),
        ...discoveredWithoutDefault,
      ];
  const shouldShowPendingModelOption =
    !isSharedCompute &&
    !isCustomModelEditing &&
    modelDiscoveryLoading &&
    discoveredModelOptions === null &&
    trimmedModel.length > 0;
  const effectiveModelOptions =
    shouldShowPendingModelOption &&
    !baseModelOptions.some((option) => option.id === trimmedModel)
      ? [...baseModelOptions, { id: trimmedModel, label: trimmedModel }]
      : baseModelOptions;

  // isModelCustom: true when the current model isn't in any known option set.
  // We check discovered options (when available) or runtime-static options so
  // a previously-saved custom model stays in custom mode even before discovery.
  const isModelCustom =
    !isSharedCompute &&
    !hasPersonaModelOption(effectiveModelOptions, trimmedModel);

  const modelSelectValue =
    isSharedCompute && (trimmedModel === "" || trimmedModel === "auto")
      ? AUTO_MODEL_DROPDOWN_VALUE
      : getModelSelectValue({
          isCustomModelEditing,
          isModelCustom,
          model,
        });

  // The select is only disabled for mutation pending — never for missing discovery.
  // Default/custom options remain usable regardless of discovery state.
  const selectDisabled =
    disabled || (disableSelectDuringDiscovery && modelDiscoveryLoading);

  // Show the custom model input whenever custom mode is active or the current
  // model is already custom — not gated on discovery having returned.
  const showCustomModelInput =
    !isSharedCompute && (isCustomModelEditing || isModelCustom);

  const handleModelSelectChange = (nextValue: string) => {
    if (nextValue === AUTO_MODEL_DROPDOWN_VALUE) {
      onIsCustomModelEditingChange(false);
      onModelChange(isSharedCompute ? "auto" : "");
      return;
    }
    if (nextValue === CUSTOM_MODEL_DROPDOWN_VALUE) {
      onIsCustomModelEditingChange(true);
      return;
    }
    onIsCustomModelEditingChange(false);
    onModelChange(nextValue);
  };

  const modelOptions: AgentDropdownOption[] = [
    ...effectiveModelOptions.map((option) => ({
      label: option.label,
      value: option.id || AUTO_MODEL_DROPDOWN_VALUE,
    })),
    ...(modelDiscoveryLoading && discoveredModelOptions === null
      ? [
          {
            disabled: true,
            label: "Loading models...",
            value: MODEL_DISCOVERY_LOADING_VALUE,
          },
        ]
      : []),
    ...(!isSharedCompute &&
    (showCustomModelOption || modelSelectValue === CUSTOM_MODEL_DROPDOWN_VALUE)
      ? [{ label: "Custom model...", value: CUSTOM_MODEL_DROPDOWN_VALUE }]
      : []),
  ];
  const stableSelectedModelLabel =
    keepSelectedModelValueLabel &&
    modelSelectValue === trimmedModel &&
    trimmedModel.length > 0
      ? trimmedModel
      : undefined;
  // While discovery is in flight with nothing selected, the closed field
  // reads "Loading models…" instead of a select-prompt — the field isn't
  // waiting on the user, it's waiting on the harness.
  const restingPlaceholder =
    modelDiscoveryLoading &&
    discoveredModelOptions === null &&
    trimmedModel.length === 0 &&
    !isCustomModelEditing
      ? "Loading models..."
      : placeholder;

  const modelSelect = useCustomSelect ? (
    <AgentDropdownSelect
      ariaRequired={isRequired}
      className={selectClassName}
      disabled={selectDisabled}
      id={id}
      onValueChange={handleModelSelectChange}
      options={modelOptions}
      placeholder={restingPlaceholder}
      placeholderClassName={placeholderClassName}
      searchable
      selectedLabel={stableSelectedModelLabel}
      testId={testId ?? id}
      value={modelSelectValue}
    />
  ) : (
    <select
      aria-required={isRequired}
      className={cn(
        "flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs disabled:cursor-not-allowed disabled:opacity-60",
        useChevronIcon && "appearance-none pr-10",
        selectClassName,
      )}
      disabled={selectDisabled}
      id={id}
      onChange={(event) => handleModelSelectChange(event.target.value)}
      value={modelSelectValue}
    >
      {modelOptions.map((option) => (
        <option
          disabled={option.disabled}
          key={option.value}
          value={option.value}
        >
          {option.label}
        </option>
      ))}
    </select>
  );

  return (
    <div className={cn("space-y-1.5", fieldClassName)}>
      <RequiredFieldLabel
        className={labelClassName}
        htmlFor={id}
        isRequired={isRequired}
      >
        Model
      </RequiredFieldLabel>
      {!useCustomSelect && useChevronIcon ? (
        <div className="relative">
          {modelSelect}
          <ChevronDown
            aria-hidden="true"
            className="pointer-events-none absolute right-4 top-1/2 h-4 w-4 -translate-y-1/2 text-foreground"
          />
        </div>
      ) : (
        modelSelect
      )}
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
      {showStatusMessage ? (
        <p className="text-xs text-muted-foreground">
          {modelDiscoveryLoading
            ? "Loading models..."
            : modelDiscoveryStatus !== null
              ? modelDiscoveryStatus.message
              : discoveredModelOptions !== null
                ? "Saved changes take effect on the next start."
                : "Select a provider above to see available models."}
        </p>
      ) : null}
    </div>
  );
}

export function AgentProviderField({
  disabled,
  globalProvider,
  isCustomProviderEditing,
  isRequired,
  onProviderChange,
  provider,
  selectedRuntime,
}: {
  disabled: boolean;
  globalProvider?: string;
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
    globalProvider,
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
            {option.id ? providerDisplayLabel(option.label) : option.label}
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
