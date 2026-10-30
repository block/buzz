import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

const NO_COMMANDS: readonly SlashCommandDefinition[] = [];

export function useMembershipCommands(
  _environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  return NO_COMMANDS;
}
