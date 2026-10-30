import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

const NO_COMMANDS: readonly SlashCommandDefinition[] = [];

export function useRemindCommands(
  _environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  return NO_COMMANDS;
}
