import type { PersonaDropdownOption } from "./agentConfigOptions";

/** Option label with the harness's optional description as a muted second line. */
export function OptionLabel({ option }: { option: PersonaDropdownOption }) {
  return (
    <span className="flex min-w-0 flex-col">
      <span className="truncate">{option.label}</span>
      {option.description ? (
        <span className="truncate text-xs text-muted-foreground">
          {option.description}
        </span>
      ) : null}
    </span>
  );
}
