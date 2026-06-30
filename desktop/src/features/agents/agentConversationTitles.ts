import type { TimelineMessage } from "@/features/messages/types";
import type {
  AgentConversation,
  AgentConversationTitleStatus,
  OpenAgentConversationInput,
} from "./agentConversations";

const MIN_CONTEXT_MESSAGES_FOR_TOPIC_TITLE = 3;
const MIN_MEANINGFUL_HUMAN_MESSAGES_FOR_TOPIC_TITLE = 2;
const CONCISE_TITLE_MAX_WORDS = 5;
const CONCISE_TITLE_MAX_CHARS = 44;
const GENERIC_REFERENCE_WORDS = new Set([
  "actually",
  "again",
  "also",
  "bit",
  "even",
  "half",
  "just",
  "kind",
  "little",
  "maybe",
  "more",
  "much",
  "really",
  "same",
  "slightly",
  "sort",
  "still",
  "thing",
  "things",
]);
const TITLE_STOP_WORDS = new Set([
  "a",
  "about",
  "an",
  "and",
  "app",
  "are",
  "as",
  "be",
  "but",
  "by",
  "can",
  "could",
  "did",
  "do",
  "does",
  "for",
  "from",
  "get",
  "had",
  "has",
  "have",
  "having",
  "help",
  "how",
  "i",
  "if",
  "in",
  "into",
  "is",
  "it",
  "its",
  "kind",
  "kinds",
  "like",
  "me",
  "mean",
  "meant",
  "of",
  "on",
  "or",
  "our",
  "please",
  "product",
  "that",
  "the",
  "their",
  "them",
  "there",
  "tell",
  "this",
  "to",
  "type",
  "types",
  "us",
  "was",
  "we",
  "what",
  "when",
  "where",
  "which",
  "with",
  "work",
  "working",
  "would",
  "you",
  "your",
]);
const TOPIC_TOKEN_PRIORITY = new Map([
  ["animation", 18],
  ["composer", 18],
  ["conversation", 18],
  ["conversations", 18],
  ["data", 50],
  ["header", 18],
  ["link", 18],
  ["message", 14],
  ["messages", 14],
  ["padding", 18],
  ["search", 18],
  ["sidebar", 18],
  ["spacing", 18],
  ["thread", 18],
  ["threads", 18],
  ["title", 22],
  ["titles", 22],
  ["user", 16],
  ["users", 16],
]);
const TOPIC_ANCHOR_SUFFIX =
  "app|product|workspace|relay|channel|thread|conversation|sidebar|composer|header|inbox|panel|title|link|button|row|animation|shimmer|screen|view";
const TOPIC_ANCHOR_PATTERN = new RegExp(
  `\\b(?:the\\s+)?([A-Z][A-Za-z0-9_-]*(?:\\s+[A-Z][A-Za-z0-9_-]+){0,2}\\s+(?:${TOPIC_ANCHOR_SUFFIX}))\\b`,
  "g",
);

function compactMessageText(message: TimelineMessage | null): string | null {
  if (
    /^\s*(?:\*\*)?Outcome from continued conversation/i.test(
      message?.body ?? "",
    ) ||
    /^\s*Please send a concise summary of this continued conversation/i.test(
      message?.body ?? "",
    ) ||
    /^\s*Please create a concise conversation recap/i.test(
      message?.body ?? "",
    ) ||
    /^\s*thinking\.{0,3}\s*$/i.test(message?.body ?? "")
  ) {
    return null;
  }

  const compact = message?.body
    .replace(/```[\s\S]*?```/g, " code ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*]\([^)]+\)/g, "media")
    .replace(/https?:\/\/\S+/g, "link")
    .replace(/@\S+/g, "")
    .replace(/^[\s,.:;-]*(ok|okay|so|also|then|and then|um|uh)[\s,.:;-]+/i, "")
    .replace(/^(i think|i guess|i wonder if|maybe|basically)[\s,.:;-]+/i, "")
    .replace(/^(can|could|would) (you|we)\s+/i, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.!?]+$/, "");

  if (!compact) {
    return null;
  }

  return compact;
}

function normalizeWorkTitleText(text: string): string {
  let normalized = text;

  for (let index = 0; index < 4; index += 1) {
    const next = normalized
      .replace(/^(?:i\s+)?(?:mean|meant),?\s+/i, "")
      .replace(/^(?:i\s+)?(?:think|guess|wonder)(?:\s+that)?(?:\s+if)?\s+/i, "")
      .replace(/^(?:can|could|would|should)\s+(?:you|we)\s+/i, "")
      .replace(/^(?:i\s+)?(?:just\s+)?(?:want|wanted)\s+(?:to\s+)?/i, "")
      .replace(/^(?:what\s+)?(?:i\s+)?(?:would\s+)?like\s+(?:is\s+)?/i, "")
      .replace(/^(?:also|okay|ok|so|then|and then|actually)\s+/i, "")
      .replace(/^(?:just|maybe)\s+/i, "")
      .trim();

    if (next === normalized) {
      break;
    }
    normalized = next;
  }

  return normalized
    .replace(/\b(?:like|basically|kind of|sort of)\b[,\s]*/gi, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function isGenericConversationTitle(text: string): boolean {
  const normalized = text.toLowerCase();

  return (
    /^(respond|reply|answer|can respond|can reply)$/.test(normalized) ||
    /^what can i help you with\b/.test(normalized) ||
    /^of course\b/.test(normalized) ||
    /^(thanks|thank you|got it|sounds good)$/.test(normalized)
  );
}

function formatConversationTitle(text: string): string {
  const sentenceEnd = text.search(/[.!?]\s/);
  const candidate = sentenceEnd > 12 ? text.slice(0, sentenceEnd).trim() : text;
  const words = candidate.split(" ");
  const title =
    words.length > CONCISE_TITLE_MAX_WORDS
      ? words.slice(0, CONCISE_TITLE_MAX_WORDS).join(" ")
      : candidate;

  return title.length > CONCISE_TITLE_MAX_CHARS
    ? `${title.slice(0, CONCISE_TITLE_MAX_CHARS - 3).trimEnd()}...`
    : title;
}

export function sentenceCaseTitle(text: string): string {
  if (!text) {
    return text;
  }

  return `${text.charAt(0).toLocaleUpperCase()}${text.slice(1)}`;
}

function titleCaseToken(token: string): string {
  if (token.toUpperCase() === token && token.length <= 4) {
    return token;
  }

  return `${token.charAt(0).toLocaleUpperCase()}${token.slice(1).toLocaleLowerCase()}`;
}

export function normalizeTitleToken(token: string): string {
  const normalized = token
    .toLocaleLowerCase()
    .replace(/'s$/, "")
    .replace(/[^a-z0-9_-]/g, "");

  if (normalized.endsWith("ies") && normalized.length > 4) {
    return `${normalized.slice(0, -3)}y`;
  }
  if (normalized.endsWith("s") && normalized.length > 4) {
    return normalized.slice(0, -1);
  }

  return normalized;
}

function extractConciseTopicPhrase(text: string): string | null {
  const normalized = normalizeWorkTitleText(text)
    .replace(
      /^(?:tell me about|talk about|explain|describe|summarize|look into|look at|check|review|investigate)\s+/i,
      "",
    )
    .replace(
      /^what\s+(?:kind|types?)\s+of\s+(.+?)(?:\s+(?:do|does|did|is|are|we|you)\b|$).*/i,
      "$1",
    )
    .replace(
      /^what\s+(.+?)\s+(?:do|does|did|can|could|would|should|is|are)\b.*$/i,
      "$1",
    )
    .replace(
      /\b(?:do\s+)?(?:we|you|i)\s+(?:have|store|collect|track|use|show|need|want)\b/gi,
      "",
    )
    .replace(
      /\b(?:about|around|for|of)\s+(?:how|what|why|when|where|whether)\b.*$/i,
      "",
    )
    .replace(/\b(?:so that|because|when|if|whether)\b.*$/i, "")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.!?]+$/, "");

  if (!normalized) {
    return null;
  }

  return formatConversationTitle(normalized);
}

function titleFromMessage(
  message: TimelineMessage | null,
  options?: { allowGeneric?: boolean; workTitle?: boolean },
): string | null {
  const compact = compactMessageText(message);
  if (!compact) {
    return null;
  }

  const title = sentenceCaseTitle(
    formatConversationTitle(
      options?.workTitle
        ? (extractConciseTopicPhrase(compact) ??
            normalizeWorkTitleText(compact))
        : compact,
    ),
  );
  if (!title) {
    return null;
  }

  if (!options?.allowGeneric && isGenericConversationTitle(title)) {
    return null;
  }

  return title;
}

function countSpecificTitleTokens(title: string): number {
  return title
    .toLowerCase()
    .split(/[^a-z0-9_-]+/)
    .filter((token) => {
      if (token.length <= 2) {
        return false;
      }
      if (TITLE_STOP_WORDS.has(token)) {
        return false;
      }
      if (GENERIC_REFERENCE_WORDS.has(token)) {
        return false;
      }

      return true;
    }).length;
}

function isReferentialTitle(title: string): boolean {
  const normalized = title.toLowerCase();
  if (!/\b(?:it|that|this|those|these|them|one)\b/.test(normalized)) {
    return false;
  }

  return countSpecificTitleTokens(title) < 3;
}

function extractTopicAnchors(text: string): string[] {
  TOPIC_ANCHOR_PATTERN.lastIndex = 0;

  return [...text.matchAll(TOPIC_ANCHOR_PATTERN)]
    .map((match) => match[1]?.trim())
    .filter((anchor): anchor is string => Boolean(anchor));
}

function pickTopicAnchor(texts: readonly string[]): string | null {
  const anchors = new Map<string, { display: string; score: number }>();

  texts.forEach((text, index) => {
    for (const anchor of extractTopicAnchors(text)) {
      const key = anchor.toLocaleLowerCase();
      const current = anchors.get(key);
      const score = 24 + index * 5 + anchor.split(/\s+/).length * 2;
      anchors.set(key, {
        display: current?.display ?? anchor,
        score: (current?.score ?? 0) + score,
      });
    }
  });

  return (
    [...anchors.values()].sort((left, right) => right.score - left.score)[0]
      ?.display ?? null
  );
}

function pickPrimaryTopicTerm(texts: readonly string[]): {
  display: string;
  normalized: string;
} | null {
  const terms = new Map<
    string,
    { display: string; firstSeen: number; score: number }
  >();

  texts.forEach((text, index) => {
    const phrase =
      extractConciseTopicPhrase(text) ?? normalizeWorkTitleText(text);
    for (const match of phrase.matchAll(/[A-Za-z][A-Za-z0-9_-]*/g)) {
      const rawToken = match[0];
      const normalized = normalizeTitleToken(rawToken);
      if (
        !normalized ||
        TITLE_STOP_WORDS.has(normalized) ||
        GENERIC_REFERENCE_WORDS.has(normalized)
      ) {
        continue;
      }

      const priority =
        TOPIC_TOKEN_PRIORITY.get(normalized) ??
        TOPIC_TOKEN_PRIORITY.get(rawToken.toLocaleLowerCase()) ??
        0;
      const current = terms.get(normalized);
      terms.set(normalized, {
        display: current?.display ?? titleCaseToken(rawToken),
        firstSeen: current?.firstSeen ?? index,
        score: (current?.score ?? 0) + 8 + index * 3 + priority,
      });
    }
  });

  const best = [...terms.entries()].sort(
    (left, right) =>
      right[1].score - left[1].score || left[1].firstSeen - right[1].firstSeen,
  )[0];

  if (!best || best[1].score < 10) {
    return null;
  }

  return { display: best[1].display, normalized: best[0] };
}

function deriveConciseContextTitle({
  contextMessages,
  normalizedAgentPubkey,
}: {
  contextMessages: TimelineMessage[];
  normalizedAgentPubkey: string;
}): string | null {
  const humanTexts = contextMessages
    .filter(
      (message) =>
        message.pubkey?.toLocaleLowerCase() !== normalizedAgentPubkey,
    )
    .map((message) => compactMessageText(message))
    .filter((text): text is string => Boolean(text));

  if (humanTexts.length === 0) {
    return null;
  }

  const anchor = pickTopicAnchor(humanTexts);
  const primaryTerm = pickPrimaryTopicTerm(humanTexts);
  if (anchor && primaryTerm) {
    const normalizedAnchor = anchor.toLocaleLowerCase();
    if (!normalizedAnchor.includes(primaryTerm.normalized)) {
      return `${primaryTerm.display} in ${anchor}`;
    }

    return sentenceCaseTitle(anchor);
  }

  const latestSpecificPhrase = [...humanTexts]
    .reverse()
    .map((text) => extractConciseTopicPhrase(text))
    .find(
      (title): title is string =>
        title != null &&
        !isReferentialTitle(title) &&
        countSpecificTitleTokens(title) > 0,
    );

  return latestSpecificPhrase ? sentenceCaseTitle(latestSpecificPhrase) : null;
}

export function collectConversationContextMessages(
  input: OpenAgentConversationInput,
  threadRootId: string,
): TimelineMessage[] {
  const byId = new Map<string, TimelineMessage>();
  const add = (message: TimelineMessage | null | undefined) => {
    if (message) {
      byId.set(message.id, message);
    }
  };

  add(input.threadRootMessage);
  add(input.parentMessage);
  add(input.agentReply);

  for (const message of input.contextMessages ?? []) {
    if (
      message.id === threadRootId ||
      message.id === input.agentReply.id ||
      message.rootId === threadRootId ||
      message.parentId === threadRootId
    ) {
      add(message);
    }
  }

  return [...byId.values()].sort(
    (left, right) => left.createdAt - right.createdAt,
  );
}

export function deriveTitleFromContext({
  agentPubkey,
  agentReply,
  contextMessages,
  parentMessage,
  threadRootId,
  threadRootMessage,
}: {
  agentPubkey: string;
  agentReply: TimelineMessage;
  contextMessages: TimelineMessage[];
  parentMessage: TimelineMessage | null;
  threadRootId: string;
  threadRootMessage: TimelineMessage | null;
}): { status: AgentConversationTitleStatus; title: string } {
  const normalizedAgentPubkey = agentPubkey.toLowerCase();
  const titleCandidates = contextMessages.flatMap((message, index) => {
    const isAgentMessage =
      message.pubkey?.toLowerCase() === normalizedAgentPubkey;
    const title = titleFromMessage(message, { workTitle: !isAgentMessage });
    if (!title) {
      return [];
    }

    let score = Math.min(title.length, 80) + index * 10;
    if (!isAgentMessage) score += 120;
    if (message.id === threadRootId) score -= 20;
    if (message.id === parentMessage?.id) score += 10;
    if (message.id === agentReply.id) score += isAgentMessage ? -30 : 10;
    score += countSpecificTitleTokens(title) * 12;
    if (isReferentialTitle(title)) score -= 80;

    return [
      {
        isAgentMessage,
        isReferential: isReferentialTitle(title),
        score,
        title,
      },
    ];
  });
  const humanTitleCandidates = titleCandidates.filter(
    (candidate) => !candidate.isAgentMessage,
  );
  const meaningfulHumanCount = humanTitleCandidates.length;
  const hasEnoughContext =
    contextMessages.length >= MIN_CONTEXT_MESSAGES_FOR_TOPIC_TITLE ||
    meaningfulHumanCount >= MIN_MEANINGFUL_HUMAN_MESSAGES_FOR_TOPIC_TITLE;

  if (!hasEnoughContext) {
    return { status: "provisional", title: "New conversation" };
  }

  const conciseContextTitle = deriveConciseContextTitle({
    contextMessages,
    normalizedAgentPubkey,
  });
  if (conciseContextTitle) {
    return { status: "resolved", title: conciseContextTitle };
  }

  const latestSpecificHumanTitle = [...humanTitleCandidates]
    .reverse()
    .find((candidate) => !candidate.isReferential)?.title;
  const latestHumanTitle =
    latestSpecificHumanTitle ?? [...humanTitleCandidates].reverse()[0]?.title;
  const bestTitle =
    latestHumanTitle ??
    titleCandidates.sort((left, right) => right.score - left.score)[0]?.title;

  return {
    status: bestTitle ? "resolved" : "provisional",
    title:
      bestTitle ??
      titleFromMessage(threadRootMessage, { allowGeneric: true }) ??
      titleFromMessage(parentMessage, { allowGeneric: true }) ??
      titleFromMessage(agentReply, { allowGeneric: true }) ??
      "New conversation",
  };
}

export function deriveAgentConversationTitle(
  conversation: Pick<
    AgentConversation,
    | "agentPubkey"
    | "agentReply"
    | "contextMessages"
    | "parentMessage"
    | "threadRootId"
    | "threadRootMessage"
  >,
): { status: AgentConversationTitleStatus; title: string } {
  return deriveTitleFromContext(conversation);
}
