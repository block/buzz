import type { SlashCommandDefinition } from "./types";

type Listable = Pick<SlashCommandDefinition, "name" | "isVisible" | "context">;

/**
 * Commands whose name starts with the typed text, alphabetical by name.
 * Aliases are never listed and there is no fuzzy matching. Commands the user
 * is known to be denied are dropped, and so are `channel` commands in a DM.
 */
export function filterSlashCommands<T extends Listable>(
  commands: readonly T[],
  query: string,
  options: { isDm?: boolean } = {},
): T[] {
  const prefix = query.toLowerCase();
  return commands
    .filter(
      (command) =>
        command.name.toLowerCase().startsWith(prefix) &&
        !(options.isDm && command.context === "channel") &&
        (command.isVisible?.() ?? true),
    )
    .sort((a, b) => a.name.localeCompare(b.name, "en"));
}
