import type { SlashCommandDefinition } from "./types";

export type SlashCommandMatchable = Pick<
  SlashCommandDefinition,
  "name" | "aliases"
>;

export type SlashInvocation<T extends SlashCommandMatchable> = {
  command: T;
  args: string;
};

export function matchesSlashCommandName(
  command: SlashCommandMatchable,
  name: string,
): boolean {
  const lowered = name.toLowerCase();
  return (
    command.name.toLowerCase() === lowered ||
    (command.aliases ?? []).some((alias) => alias.toLowerCase() === lowered)
  );
}

/**
 * A draft is a command when, after leading whitespace, its first token is `/`
 * plus a registered name or alias and ends at whitespace or the end of the
 * draft. A draft that starts with a mention, or with any other token, is never
 * a command, so agents keep their own `/commands`.
 */
export function recognizeSlashCommand<T extends SlashCommandMatchable>(
  draft: string,
  commands: readonly T[],
): SlashInvocation<T> | null {
  const text = draft.trimStart();
  if (!text.startsWith("/")) return null;
  const tokenEnd = text.search(/\s/);
  const token = tokenEnd === -1 ? text : text.slice(0, tokenEnd);
  const name = token.slice(1);
  if (!name) return null;
  const command = commands.find((candidate) =>
    matchesSlashCommandName(candidate, name),
  );
  if (!command) return null;
  return {
    command,
    args: tokenEnd === -1 ? "" : text.slice(tokenEnd).trim(),
  };
}
