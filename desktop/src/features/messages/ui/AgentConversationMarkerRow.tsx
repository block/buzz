import { MessagesSquare } from "lucide-react";

import type { AgentConversationMarker } from "@/features/agents/agentConversations";
import type { TimelineMessage } from "@/features/messages/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type AgentConversationMarkerRowProps = {
  className?: string;
  currentPubkey?: string;
  marker: AgentConversationMarker;
  message: TimelineMessage;
  onOpenAgentConversation?: (
    message: TimelineMessage,
    options?: { publishMarker?: boolean },
  ) => void;
  profiles?: UserProfileLookup;
};

const RECAP_SECTION_PATTERN =
  /\*\*(Original request|Findings|Outcome|Next steps):\*\*/g;
const RECAP_DETAIL_TAIL_PATTERN =
  /\b(?:Nothing else needed|Nice work|Kenny asked|The user asked|Button system\s*(?:\(|[—-])|Composer primitives\s*(?:\(|[—-])|Sidebar navigation\s*(?:\(|[—-])|Key gotcha\s*:|Decisions?\s*:|Team agreed\b)[\s\S]*$/i;

function parseRecapSections(value: string): Map<string, string> {
  const matches = [...value.matchAll(RECAP_SECTION_PATTERN)];
  const sections = new Map<string, string>();
  if (matches.length === 0) {
    return sections;
  }

  matches.forEach((match, index) => {
    const label = match[1];
    const start = (match.index ?? 0) + match[0].length;
    const end =
      index + 1 < matches.length
        ? (matches[index + 1].index ?? value.length)
        : value.length;
    const content = value.slice(start, end).trim();
    if (content) {
      sections.set(label, content);
    }
  });

  return sections;
}

function stripRecapMarkdown(value: string): string {
  return value
    .replace(RECAP_SECTION_PATTERN, "")
    .replace(/\bConversation recap:\s*/gi, "")
    .replace(/^\s*[\w .'-]{1,40}:\s+(?=\S)/gm, "")
    .replace(/\[([^\]]+)]\([^)]+\)/g, "$1")
    .replace(/[`*_~>#]/g, "")
    .replace(/^\s*[-*]\s+/gm, "")
    .replace(/(?:^|\s)\d+\.\s+/g, " ")
    .replace(/\n+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function sentenceCase(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return trimmed;
  }

  return `${trimmed.charAt(0).toLocaleUpperCase()}${trimmed.slice(1)}`;
}

function ensureSentence(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return trimmed;
  }

  return /[.!?]$/.test(trimmed) ? trimmed : `${trimmed}.`;
}

function formatJoinedList(items: readonly string[]): string {
  if (items.length <= 1) {
    return items[0] ?? "";
  }
  if (items.length === 2) {
    return `${items[0]} and ${items[1]}`;
  }

  return `${items.slice(0, -1).join(", ")}, and ${items[items.length - 1]}`;
}

function cleanPreviewTopic(title: string): string | null {
  const topic = title.trim();
  if (
    !topic ||
    topic.toLocaleLowerCase() === "new conversation" ||
    /^conversation(?:\s+(?:in|about|with)\b.*)?$/i.test(topic)
  ) {
    return null;
  }

  return topic;
}

function extractCoveredAreas(value: string): string[] {
  const areas: string[] = [];
  const seen = new Set<string>();
  const areaPattern =
    /(?:^|[.!?]\s+)([A-Z][A-Za-z0-9 /&+-]{2,70})(?:\s*\([^)]*\))?\s+[—-]\s+/g;

  for (const match of value.matchAll(areaPattern)) {
    const area = match[1]?.replace(/\s+/g, " ").trim();
    if (!area) {
      continue;
    }

    const normalized = area.toLocaleLowerCase();
    if (
      seen.has(normalized) ||
      /^(key gotcha|decisions?|next steps?|conversation recap)$/i.test(area)
    ) {
      continue;
    }

    seen.add(normalized);
    areas.push(area);
    if (areas.length >= 4) {
      break;
    }
  }

  return areas;
}

function extractLabeledText(
  value: string,
  labelPattern: string,
): string | null {
  const labelRegex = new RegExp(
    `(?:^|[.\\n]\\s*)${labelPattern}\\s*:\\s*([\\s\\S]*?)(?=(?:^|[.\\n]\\s*)(?:Key gotcha|Decisions?|Next steps(?:\\s*\\([^)]*\\))?|Original request|Findings|Outcome)\\s*:|$)`,
    "i",
  );
  const match = value.match(labelRegex);
  const text = stripRecapMarkdown(match?.[1] ?? "");

  return text || null;
}

function stripRecapDetailTail(value: string): string {
  return value.replace(RECAP_DETAIL_TAIL_PATTERN, "").trim();
}

function cleanNextStepsPreviewText(value: string | null): string | null {
  const cleaned = stripRecapDetailTail(value ?? "")
    .replace(/^pending [^:]+:\s*/i, "")
    .replace(/^,\s*/, "")
    .replace(/\s+/g, " ")
    .trim();

  return cleaned || null;
}

function firstUsefulSentence(value: string): string | null {
  const sentences = value
    .split(/(?<=[.!?])\s+/)
    .map((sentence) => sentence.trim())
    .filter(Boolean);

  return (
    sentences.find(
      (sentence) =>
        !/^(kenny asked|the user asked|conversation recap)\b/i.test(sentence),
    ) ??
    sentences[0] ??
    null
  );
}

function lowerFirst(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return trimmed;
  }

  return `${trimmed.charAt(0).toLocaleLowerCase()}${trimmed.slice(1)}`;
}

function formatNextStepsText(value: string): string {
  const normalized = value.replace(/^to\s+/i, "").trim();
  const items = normalized
    .split(
      /\s+(?=(?:Publish|Link|Cross-link|Follow-up|Follow up|Add|Create|Update|Review|Share|Ship|Document)\b)/,
    )
    .map((item) => item.trim())
    .filter(Boolean);
  const formatted =
    items.length <= 1
      ? lowerFirst(normalized)
      : formatJoinedList(items.map(lowerFirst));

  if (/^(?:to|until|once)\b/i.test(formatted)) {
    return formatted;
  }

  return `to ${formatted}`;
}

function buildRecapPreview(summary: string, title: string): string {
  const sections = parseRecapSections(summary);
  const outcome = stripRecapMarkdown(sections.get("Outcome") ?? "");
  const findings = stripRecapMarkdown(sections.get("Findings") ?? "");
  const nextSteps = stripRecapMarkdown(sections.get("Next steps") ?? "");
  const source = [outcome, findings].filter(Boolean).join(" ").trim();
  const fullText = stripRecapMarkdown(summary);
  const topic = cleanPreviewTopic(title);
  const areas = extractCoveredAreas(source || fullText);
  const decision = extractLabeledText(summary, "Decisions?");
  const rawNextSteps =
    nextSteps || extractLabeledText(summary, "Next steps(?:\\s*\\([^)]*\\))?");
  const nextStepText = cleanNextStepsPreviewText(rawNextSteps);
  const fallbackSentence = firstUsefulSentence(
    stripRecapDetailTail(source || fullText),
  );
  const sentences: string[] = [];

  if (topic) {
    sentences.push(`This conversation focused on ${topic}.`);
  }

  if (areas.length > 0) {
    const areaText = formatJoinedList(topic ? areas : areas.map(lowerFirst));
    sentences.push(
      topic
        ? `The main takeaways covered ${areaText}.`
        : ensureSentence(sentenceCase(areaText)),
    );
  } else if (fallbackSentence) {
    sentences.push(ensureSentence(sentenceCase(fallbackSentence)));
  }

  if (decision) {
    sentences.push(ensureSentence(sentenceCase(decision)));
  }

  if (nextStepText) {
    sentences.push(
      `Next steps are ${ensureSentence(formatNextStepsText(nextStepText))}`,
    );
  }

  const preview = sentences.join(" ").replace(/\s+/g, " ").trim();

  return (
    preview ||
    (fallbackSentence
      ? ensureSentence(sentenceCase(fallbackSentence))
      : fullText)
  );
}

export function AgentConversationMarkerRow({
  className,
  currentPubkey,
  marker,
  message,
  onOpenAgentConversation,
  profiles,
}: AgentConversationMarkerRowProps) {
  const starterProfile = profiles?.[normalizePubkey(marker.starterPubkey)];
  const starterName = resolveUserLabel({
    currentPubkey,
    profiles,
    pubkey: marker.starterPubkey,
  });
  const recapPreview = marker.summary
    ? buildRecapPreview(marker.summary, marker.title)
    : null;

  return (
    <article
      className={cn(
        "group/message relative z-10 mx-1 flex items-start gap-2.5 rounded-2xl px-2 py-1.5",
        className,
      )}
      data-agent-conversation-id={marker.eventId}
      data-testid="agent-conversation-marker-row"
    >
      <UserAvatar
        avatarUrl={starterProfile?.avatarUrl ?? null}
        className="!h-10 !w-10 shrink-0"
        displayName={starterName}
        testId="agent-conversation-marker-avatar"
      />
      <div className="min-w-0 flex-1">
        <div className="min-w-0 overflow-hidden rounded-lg border border-border/70 bg-muted/35">
          <div className="flex min-w-0 items-center gap-3 px-3 py-2">
            <div
              aria-hidden
              className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-background p-2.5 text-muted-foreground shadow-xs ring-1 ring-border/60"
            >
              <MessagesSquare className="size-5" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium text-foreground">
                Dedicated conversation
              </p>
              <p className="mt-0.5 truncate text-xs text-muted-foreground">
                {marker.title}
              </p>
            </div>
            <Button
              className="h-8 shrink-0 rounded-lg px-3 text-xs font-medium"
              data-testid="agent-conversation-marker-open"
              disabled={!onOpenAgentConversation}
              onClick={() =>
                onOpenAgentConversation?.(message, { publishMarker: false })
              }
              type="button"
              variant="outline"
            >
              Open
            </Button>
          </div>
          {marker.summary ? (
            <div
              className="border-t border-border/70 bg-background/55 px-3 py-2.5"
              data-testid="agent-conversation-marker-summary"
            >
              <p className="mb-2 text-sm font-semibold text-foreground">
                Conversation recap
              </p>
              {recapPreview ? (
                <p
                  className="max-w-full text-sm leading-5 text-foreground"
                  data-testid="agent-conversation-marker-summary-preview"
                >
                  {recapPreview}
                </p>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    </article>
  );
}
