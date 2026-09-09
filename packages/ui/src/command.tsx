import { useState } from "react";
import { Combobox } from "./primitives/combobox";
/** A command is an explicit user action, identified independently of its label. */
export interface CommandItem {
  id: string;
  label: string;
}
/** Searchable actions with arrow-key selection. Opening or filtering never invokes an action. */
export function Command({
  label,
  items,
  onAction,
}: {
  label: string;
  items: readonly CommandItem[];
  onAction: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  return (
    <Combobox.Root<CommandItem>
      items={items}
      itemToStringLabel={(item) => item.label}
      itemToStringValue={(item) => item.id}
      inputValue={query}
      onInputValueChange={setQuery}
      value={null}
      onValueChange={(item) => {
        if (item) {
          onAction(item.id);
          setQuery("");
        }
      }}
    >
      <Combobox.Input aria-label={label} placeholder="Search commands…" />
      <Combobox.Portal>
        <Combobox.Positioner sideOffset={8}>
          <Combobox.Popup>
            <Combobox.Empty>No matching commands</Combobox.Empty>
            <Combobox.List>
              {(item: CommandItem) => (
                <Combobox.Item key={item.id} value={item}>
                  {item.label}
                </Combobox.Item>
              )}
            </Combobox.List>
          </Combobox.Popup>
        </Combobox.Positioner>
      </Combobox.Portal>
    </Combobox.Root>
  );
}
