import type { AgentCommandCatalog } from "@/features/agents/agentCommandCatalog";

export type SlashCommandProvider = {
  pubkey: string;
  displayName: string;
};

export type AgentMentionCandidate = {
  displayName: string;
  pubkey: string;
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
  candidates: readonly AgentMentionCandidate[],
): string[] {
  let remaining = leadingText;
  const pubkeys: string[] = [];
  const seen = new Set<string>();
  const names = [
    ...new Set(candidates.map((candidate) => candidate.displayName)),
  ]
    .filter(Boolean)
    .sort((left, right) => right.length - left.length);

  while (remaining.startsWith("@")) {
    const lowerRemaining = remaining.toLowerCase();
    const name = names.find((candidate) => {
      const token = `@${candidate.toLowerCase()}`;
      return (
        lowerRemaining.startsWith(token) &&
        (remaining.length === token.length ||
          /\s/u.test(remaining.charAt(token.length)))
      );
    });
    if (!name) return [];

    for (const candidate of candidates) {
      if (candidate.displayName.toLowerCase() !== name.toLowerCase()) continue;
      const normalized = candidate.pubkey.toLowerCase();
      if (!seen.has(normalized)) {
        seen.add(normalized);
        pubkeys.push(normalized);
      }
    }

    remaining = remaining.slice(name.length + 1);
    const whitespace = remaining.match(/^\s+/u)?.[0] ?? "";
    if (!whitespace) return [];
    remaining = remaining.slice(whitespace.length);
  }

  return remaining.length === 0 ? pubkeys : [];
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
  const lowerQuery = query.toLowerCase();
  if (lowerName.startsWith(lowerQuery)) return 0;
  if (lowerName.includes(lowerQuery)) return 1;
  if (description?.toLowerCase().includes(lowerQuery)) return 2;
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
          rank: commandRank(command.name, command.description, query),
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
