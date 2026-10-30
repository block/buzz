import { buildShrugMessage } from "../lib/shrug";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
} from "../lib/types";

const TEXT_COMMANDS: readonly SlashCommandDefinition[] = [
  {
    name: "shrug",
    argumentHint: "[text]",
    description: "Send a message with ¯\\_(ツ)_/¯ appended",
    usage: "/shrug [text]",
    context: "any",
    run: ({ args, sendMessage }) => sendMessage(buildShrugMessage(args)),
  },
];

export function useTextCommands(
  _environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  return TEXT_COMMANDS;
}
