import {
  AUTO_MODEL_DROPDOWN_VALUE,
  buildTemplateModelDropdownOptions,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  getModelSelectValue,
  hasPersonaModelOption,
  type PersonaDropdownOption,
  type PersonaModelOption,
} from "./agentConfigOptions";

function withSharedComputeAutoOption(
  options: readonly PersonaModelOption[],
): readonly PersonaModelOption[] {
  const modelOptions = options.filter((option) => option.id.trim() !== "");
  return [{ id: "", label: "Default (auto)" }, ...modelOptions];
}

export function relayMeshModelPickerState({
  discoveredOptions,
  fallbackOptions,
  knownOptions,
  isCustomEditing,
  model,
  modelFieldVisible = true,
  provider,
}: {
  discoveredOptions: readonly PersonaModelOption[] | null;
  fallbackOptions: readonly PersonaModelOption[];
  knownOptions?: readonly PersonaModelOption[];
  isCustomEditing: boolean;
  model: string;
  modelFieldVisible?: boolean;
  provider: string;
}) {
  const isRelayMesh = provider.trim() === "relay-mesh";
  const trimmedModel = model.trim();
  const options = isRelayMesh
    ? withSharedComputeAutoOption(discoveredOptions ?? [])
    : (discoveredOptions ?? fallbackOptions);
  const isKnownModel = hasPersonaModelOption(knownOptions ?? options, model);
  const isCustom = !isRelayMesh && !isKnownModel;
  const selectValue = isRelayMesh
    ? trimmedModel === "auto" || !isKnownModel
      ? AUTO_MODEL_DROPDOWN_VALUE
      : trimmedModel || AUTO_MODEL_DROPDOWN_VALUE
    : getModelSelectValue({
        isCustomModelEditing: isCustomEditing,
        isModelCustom: isCustom,
        model,
      });
  return {
    isCustom,
    isRelayMesh,
    options,
    selectValue,
    showCustomInput:
      !isRelayMesh && modelFieldVisible && (isCustomEditing || isCustom),
  };
}

export function modelDropdownOptions({
  options,
  loading,
  loadingValue,
  allowCustom,
  globalModel,
  globalModelLabel,
}: {
  options: readonly PersonaModelOption[];
  loading: boolean;
  loadingValue: string;
  allowCustom: boolean;
  globalModel?: string;
  globalModelLabel?: string;
}): PersonaDropdownOption[] {
  const modelOptions =
    globalModel === undefined
      ? options.map((option) => ({
          label: option.label,
          value: option.id || AUTO_MODEL_DROPDOWN_VALUE,
        }))
      : buildTemplateModelDropdownOptions(
          options,
          globalModel,
          globalModelLabel,
        );
  return [
    ...modelOptions,
    ...(loading
      ? [{ disabled: true, label: "Loading models...", value: loadingValue }]
      : []),
    ...(allowCustom
      ? [{ label: "Custom model...", value: CUSTOM_MODEL_DROPDOWN_VALUE }]
      : []),
  ];
}
