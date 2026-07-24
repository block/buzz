import type { AgentCommandCatalog } from "@/features/agents/agentCommandCatalog";
import { mentionOccurrences } from "@/shared/lib/mentionOccurrences";

export type SlashCommandProvider = {
  pubkey: string;
  displayName: string;
};

export type SlashCommandSuggestion = {
  agentDisplayName: string;
  agentPubkey: string;
  description: string | null;
  name: string;
};

export type SlashCommandGroup = {
  agentDisplayName: string;
  agentPubkey: string;
  commands: readonly SlashCommandSuggestion[];
};

export type SlashCommandQuery = {
  leadingText: string;
  query: string;
  replaceFromOffset: number;
};

export function detectSlashCommandQuery(
  value: string,
  cursorPosition: number,
): SlashCommandQuery | null {
  const beforeCursor = value.slice(0, cursorPosition);
  if (beforeCursor.includes("\n")) return null;

  const slashIndex = beforeCursor.lastIndexOf("/");
  if (slashIndex < 0) return null;
  const leadingText = beforeCursor.slice(0, slashIndex);
  const query = beforeCursor.slice(slashIndex + 1);
  if (/\s|\//u.test(query)) return null;
  if (
    leadingText.length > 0 &&
    (!leadingText.startsWith("@") || !/\s$/u.test(leadingText))
  ) {
    return null;
  }

  return { leadingText, query, replaceFromOffset: slashIndex };
}

export function resolveLeadingAgentMentionPubkeys(
  leadingText: string,
  candidates: readonly SlashCommandProvider[],
): string[] {
  const pubkeys = new Set<string>();
  let offset = 0;
  for (const match of mentionOccurrences(leadingText, candidates)) {
    if (match.start !== offset) return [];
    for (const candidate of match.candidates)
      pubkeys.add(candidate.pubkey.toLowerCase());
    const whitespace = leadingText.slice(match.end).match(/^\s+/u)?.[0];
    if (!whitespace) return [];
    offset = match.end + whitespace.length;
  }
  return offset === leadingText.length ? [...pubkeys] : [];
}

export function buildSlashCommandInsertText(
  suggestion: SlashCommandSuggestion,
  hasLeadingAgentMention: boolean,
): string {
  const command = `/${suggestion.name} `;
  return hasLeadingAgentMention
    ? command
    : `@${suggestion.agentDisplayName} ${command}`;
}

function commandRank(
  name: string,
  description: string | null,
  query: string,
): number | null {
  if (!query) return 0;
  const lowerName = name.toLowerCase();
  if (lowerName.startsWith(query)) return 0;
  if (lowerName.includes(query)) return 1;
  if (description?.toLowerCase().includes(query)) return 2;
  return null;
}

export function buildSlashCommandGroups({
  catalog,
  providers,
  query,
  selectedAgentPubkeys,
}: {
  catalog: AgentCommandCatalog;
  providers: readonly SlashCommandProvider[];
  query: string;
  selectedAgentPubkeys: readonly string[] | null;
}): SlashCommandGroup[] {
  const lowerQuery = query.toLowerCase();
  const selected = selectedAgentPubkeys
    ? new Set(selectedAgentPubkeys.map((pubkey) => pubkey.toLowerCase()))
    : null;

  return providers
    .filter(
      (provider) => !selected || selected.has(provider.pubkey.toLowerCase()),
    )
    .map((provider) => {
      const commands = (
        catalog.get(provider.pubkey.toLowerCase())?.commands ?? []
      )
        .map((command) => ({
          command,
          rank: commandRank(command.name, command.description, lowerQuery),
        }))
        .filter(
          (entry): entry is typeof entry & { rank: number } =>
            entry.rank !== null,
        )
        .sort(
          (left, right) =>
            left.rank - right.rank ||
            left.command.name.localeCompare(right.command.name),
        )
        .map(({ command }) => ({
          agentDisplayName: provider.displayName,
          agentPubkey: provider.pubkey,
          description: command.description,
          name: command.name,
        }));
      return {
        agentDisplayName: provider.displayName,
        agentPubkey: provider.pubkey,
        commands,
      };
    })
    .filter((group) => group.commands.length > 0);
}
