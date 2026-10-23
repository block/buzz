import type { PersonaDropdownOption } from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { PersonaModelCombobox } from "./PersonaModelCombobox";

/**
 * LLM-provider control shared by the agent dialogs.
 *
 * `searchable` selects the combobox: harness-published inventories (goose lists
 * ~85 providers) make a plain menu unusable, and the combobox filters on both
 * the label (provider name) and the value (provider id). Runtimes still on the
 * built-in catalog keep the compact menu.
 */
export function ProviderSelectField({
  disabled,
  id,
  onValueChange,
  options,
  placeholder,
  searchable,
  value,
}: {
  disabled?: boolean;
  id: string;
  onValueChange: (value: string) => void;
  options: readonly PersonaDropdownOption[];
  placeholder: string;
  searchable: boolean;
  value: string;
}) {
  if (searchable) {
    return (
      <PersonaModelCombobox
        disabled={disabled}
        emptyLabel="No providers match"
        id={id}
        onValueChange={onValueChange}
        options={options}
        placeholder={placeholder}
        searchLabel="Search providers"
        searchPlaceholder="Search providers…"
        value={value}
      />
    );
  }
  return (
    <PersonaDropdownField
      disabled={disabled}
      id={id}
      onValueChange={onValueChange}
      options={options}
      placeholder={placeholder}
      value={value}
    />
  );
}
