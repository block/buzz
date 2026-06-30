import type { TimelineMessage } from "@/features/messages/types";
import type { AgentConversationRecapInput } from "./agentConversations";
import {
  normalizeTitleToken,
  sentenceCaseTitle,
} from "./agentConversationTitles";

function normalizeRecapComparisonText(text: string | null | undefined): string {
  return (text ?? "").replace(/\s+/g, " ").trim().toLocaleLowerCase();
}

function isGenericRecapText(text: string): boolean {
  const normalized = normalizeRecapComparisonText(text);

  return (
    normalized.length < 3 ||
    normalized === "thinking" ||
    normalized === "thinking..." ||
    /^what can i help you with\b/.test(normalized) ||
    /^of course\b.*\bwhat do you need help with\??$/.test(normalized) ||
    (/^(sure|okay|ok|got it|i get it|i understand)\b/.test(normalized) &&
      /\b(?:summarize|summary|recap)\b/.test(normalized) &&
      /\b(?:you want|you'd like|you're asking|you asked)\b/.test(normalized))
  );
}

function formatRecapMessageText(message: TimelineMessage): string | null {
  const body = message.body ?? "";
  if (
    /^\s*(?:\*\*)?Outcome from continued conversation/i.test(body) ||
    /^\s*Please send a concise summary of this continued conversation/i.test(
      body,
    ) ||
    /^\s*Please create a concise conversation recap/i.test(body) ||
    /^\s*thinking\.{0,3}\s*$/i.test(body)
  ) {
    return null;
  }

  const cleaned = body
    .replace(/\r\n/g, "\n")
    .replace(/```[\s\S]*?```/g, " code ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*]\([^)]+\)/g, "media")
    .replace(/https?:\/\/\S+/g, "link")
    .replace(/@\S+/g, "")
    .replace(/^[\s,.:;-]*(ok|okay|so|also|then|and then|um|uh)[\s,.:;-]+/i, "")
    .replace(/^(i think|i guess|i wonder if|maybe|basically)[\s,.:;-]+/i, "")
    .replace(/^(can|could|would) (you|we)\s+/i, "")
    .replace(/[ \t]+/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
    .replace(/[.!?]+$/, "");

  if (!cleaned || isGenericRecapText(cleaned)) {
    return null;
  }

  return sentenceCaseTitle(cleaned);
}

function isSameRecapPoint(
  left: string | null | undefined,
  right: string | null | undefined,
) {
  return (
    normalizeRecapComparisonText(left) === normalizeRecapComparisonText(right)
  );
}

function appendUniqueRecapPoint(points: string[], point: string | null) {
  if (!point) {
    return;
  }

  if (points.some((current) => isSameRecapPoint(current, point))) {
    return;
  }

  points.push(point);
}

function normalizeInlineOrderedListBreaks(value: string): string {
  const itemMatches = [...value.matchAll(/(?:^|\s)(\d+)\.\s+/g)];
  if (itemMatches.length < 2) {
    return value;
  }

  return value.replace(/\s+(?=\d+\.\s+)/g, "\n");
}

function formatRecapSection(
  label: string,
  value: string | null,
): string | null {
  if (!value) {
    return null;
  }

  const formattedValue = normalizeInlineOrderedListBreaks(value);
  const firstListIndex = formattedValue.search(/(?:^|\n)\d+\.\s/);
  if (firstListIndex < 0) {
    return `**${label}:** ${formattedValue}`;
  }

  const preface = formattedValue.slice(0, firstListIndex).trim();
  const list = formattedValue.slice(firstListIndex).trim();

  return preface
    ? `**${label}:** ${preface}\n\n${list}`
    : `**${label}:**\n\n${list}`;
}

function singleLineRecapText(value: string | null): string | null {
  if (!value) {
    return null;
  }

  return value.replace(/\s+/g, " ").trim();
}

export function buildAgentConversationRecap({
  agentPubkeys,
  messages,
}: AgentConversationRecapInput): string | null {
  const normalizedAgentPubkeys = new Set(
    [...agentPubkeys].map((pubkey) => normalizeTitleToken(pubkey)),
  );
  const usableMessages = [...messages]
    .flatMap((message, originalIndex) => {
      const text = formatRecapMessageText(message);
      if (!text) {
        return [];
      }

      return [
        {
          isAgent:
            message.pubkey != null &&
            normalizedAgentPubkeys.has(normalizeTitleToken(message.pubkey)),
          message,
          originalIndex,
          text,
        },
      ];
    })
    .sort(
      (left, right) =>
        left.message.createdAt - right.message.createdAt ||
        left.originalIndex - right.originalIndex,
    );

  if (usableMessages.length === 0) {
    return null;
  }

  const humanMessages = usableMessages.filter((entry) => !entry.isAgent);
  const agentMessages = usableMessages.filter((entry) => entry.isAgent);
  const firstHumanText = humanMessages[0]?.text ?? null;
  const latestHumanText = humanMessages[humanMessages.length - 1]?.text ?? null;
  const originalRequest =
    firstHumanText &&
    latestHumanText &&
    !isSameRecapPoint(firstHumanText, latestHumanText)
      ? `${singleLineRecapText(firstHumanText)} Later clarified: ${singleLineRecapText(latestHumanText)}`
      : firstHumanText;

  const outcomeMessage = [...agentMessages].reverse()[0] ?? null;
  const latestAgentByPubkey = new Map<string, (typeof agentMessages)[number]>();
  for (const entry of agentMessages) {
    if (entry.message.id === outcomeMessage?.message.id) {
      continue;
    }

    latestAgentByPubkey.set(
      normalizeTitleToken(entry.message.pubkey ?? entry.message.author),
      entry,
    );
  }
  const findingPoints: string[] = [];
  for (const entry of [...latestAgentByPubkey.values()].slice(-3)) {
    const prefix =
      latestAgentByPubkey.size > 1 ? `${entry.message.author}: ` : "";
    appendUniqueRecapPoint(findingPoints, `${prefix}${entry.text}`);
  }
  const findings = findingPoints.join(" ") || null;
  const outcome = outcomeMessage?.text ?? null;

  const latestMessage = usableMessages[usableMessages.length - 1];
  const nextSteps =
    !latestMessage.isAgent && !isSameRecapPoint(latestHumanText, firstHumanText)
      ? `Follow up on the latest question: ${latestMessage.text}`
      : null;
  const sections = [
    formatRecapSection("Original request", originalRequest),
    formatRecapSection("Findings", findings),
    formatRecapSection("Outcome", outcome),
    formatRecapSection("Next steps", nextSteps),
  ].filter((section): section is string => section !== null);

  return sections.length > 0 ? sections.join("\n\n") : null;
}
