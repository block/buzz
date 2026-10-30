import type { SlashCommandDefinition } from "./types";

/**
 * Flattens the group hooks' definitions. A name or alias already claimed by
 * an earlier command is ignored, so one group cannot shadow another. `channel`
 * commands stay recognized in a DM so the executor can refuse them visibly.
 */
export function composeSlashCommands(
  groups: readonly (readonly SlashCommandDefinition[])[],
  options: { enabled: boolean },
): SlashCommandDefinition[] {
  if (!options.enabled) return [];
  const claimed = new Set<string>();
  const commands: SlashCommandDefinition[] = [];
  for (const command of groups.flat()) {
    const names = [command.name, ...(command.aliases ?? [])].map((name) =>
      name.toLowerCase(),
    );
    if (names.some((name) => claimed.has(name))) continue;
    for (const name of names) claimed.add(name);
    commands.push(command);
  }
  return commands;
}
