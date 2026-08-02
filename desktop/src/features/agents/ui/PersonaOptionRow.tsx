import type { PersonaDropdownOption } from "./agentConfigOptions";

/**
 * The inside of one picker row: the option's label, and the secondary line
 * under it when the option carries one.
 *
 * Shared because the dropdown and the combobox render the SAME row and differ
 * only in the control wrapped around it — a Radix radio item vs a searchable
 * button. The secondary line is the adapter's own `description` (see
 * `personaModelOptionDescription`), which is the only thing that distinguishes
 * two models an adapter reports under one name, so the two pickers must not
 * drift into two typographies for it.
 */
export function PersonaOptionRow({
  option,
}: {
  option: Pick<PersonaDropdownOption, "description" | "label">;
}) {
  return (
    <span className="min-w-0 flex-1">
      <span className="block truncate">{option.label}</span>
      {option.description ? (
        <span className="block truncate text-xs text-muted-foreground">
          {option.description}
        </span>
      ) : null}
    </span>
  );
}
