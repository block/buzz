import type { ChannelType } from "@/shared/api/types";
import { channelNamesMatch } from "@/features/channels/lib/canonicalChannelName";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

export type CanvasBoardCardKind =
  | "artifact"
  | "invitation"
  | "note"
  | "now"
  | "people"
  | "welcome";

export type CanvasBoardCardType =
  | "agent"
  | "artifact"
  | "conversation"
  | "decision"
  | "note"
  | "person"
  | "project"
  | "task";

export type CanvasBoardCardStatus = "backlog" | "doing" | "done";

export type CanvasBoardCard = {
  author: string | null;
  body: string;
  hasExplicitMetadata: boolean;
  id: string;
  kind: CanvasBoardCardKind;
  status: CanvasBoardCardStatus;
  threadId: string | null;
  title: string;
  type: CanvasBoardCardType;
};

export type CanvasBoard = {
  cards: CanvasBoardCard[];
  introduction: string;
  title: string | null;
};

export type CanvasBoardCardDraft = {
  author?: string | null;
  body: string;
  id?: string;
  status?: CanvasBoardCardStatus;
  threadId?: string | null;
  title: string;
  type?: CanvasBoardCardType;
};

export type ChannelViewMode = "board" | "stream";

const CHANNEL_VIEW_MODE_STORAGE_PREFIX = "buzz.channelViewMode.v1";

const H1_PATTERN = /^#\s+(.+?)\s*#*\s*$/u;
const H2_PATTERN = /^##\s+(.+?)\s*#*\s*$/u;
const FENCE_OPEN_PATTERN = /^ {0,3}(`{3,}|~{3,})/u;
const FENCE_CLOSE_PATTERN = /^ {0,3}(`{3,}|~{3,})[ \t]*$/u;
const CARD_METADATA_PATTERN =
  /^\s*<!--\s*buzz-board-card\s+(\{.*\})\s*-->\s*$/u;
const CARD_ID_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/u;
const EVENT_ID_PATTERN = /^[0-9a-f]{64}$/u;

const CARD_TYPES = new Set<CanvasBoardCardType>([
  "agent",
  "artifact",
  "conversation",
  "decision",
  "note",
  "person",
  "project",
  "task",
]);
const CARD_STATUSES = new Set<CanvasBoardCardStatus>([
  "backlog",
  "doing",
  "done",
]);

type CanvasBoardCardMetadata = {
  author: string | null;
  id: string | null;
  status: CanvasBoardCardStatus | null;
  threadId: string | null;
  type: CanvasBoardCardType | null;
};

type ParsedCanvasBoardSection = {
  body: string;
  hasExplicitMetadata: boolean;
  metadata: CanvasBoardCardMetadata;
};

type CanvasBoardSourceSection = {
  bodyLines: string[];
  headingLine: string;
  title: string;
};

type CanvasBoardSource = {
  introductionLines: string[];
  preambleLines: string[];
  sections: CanvasBoardSourceSection[];
  title: string | null;
};

function cardId(title: string, index: number): string {
  const slug = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, "-")
    .replace(/^-|-$/gu, "");
  return `${slug || "card"}-${index + 1}`;
}

function isStringRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0
    ? value.trim()
    : null;
}

function parseCardMetadataLine(line: string): CanvasBoardCardMetadata | null {
  const match = line.match(CARD_METADATA_PATTERN);
  if (!match) {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(match[1]);
    if (!isStringRecord(parsed)) {
      return null;
    }
    const id = optionalString(parsed.id);
    const type = optionalString(parsed.type);
    const status = optionalString(parsed.status);
    const threadId = optionalString(parsed.thread);
    const author = optionalString(parsed.author);
    return {
      author,
      id: id && CARD_ID_PATTERN.test(id) ? id : null,
      status:
        status && CARD_STATUSES.has(status as CanvasBoardCardStatus)
          ? (status as CanvasBoardCardStatus)
          : null,
      threadId:
        threadId && EVENT_ID_PATTERN.test(threadId.toLowerCase())
          ? threadId.toLowerCase()
          : null,
      type:
        type && CARD_TYPES.has(type as CanvasBoardCardType)
          ? (type as CanvasBoardCardType)
          : null,
    };
  } catch {
    return null;
  }
}

function parseCanvasBoardSection(
  section: CanvasBoardSourceSection,
): ParsedCanvasBoardSection {
  const bodyLines = [...section.bodyLines];
  const metadataLineIndex = bodyLines.findIndex(
    (line) => line.trim().length > 0,
  );
  if (metadataLineIndex === -1) {
    return {
      body: "",
      hasExplicitMetadata: false,
      metadata: {
        author: null,
        id: null,
        status: null,
        threadId: null,
        type: null,
      },
    };
  }

  const metadata = parseCardMetadataLine(bodyLines[metadataLineIndex]);
  if (!metadata) {
    return {
      body: bodyLines.join("\n").trim(),
      hasExplicitMetadata: false,
      metadata: {
        author: null,
        id: null,
        status: null,
        threadId: null,
        type: null,
      },
    };
  }

  bodyLines.splice(metadataLineIndex, 1);
  return {
    body: bodyLines.join("\n").trim(),
    hasExplicitMetadata: true,
    metadata,
  };
}

function serializeCardMetadata(metadata: {
  author?: string | null;
  id: string;
  status: CanvasBoardCardStatus;
  threadId?: string | null;
  type: CanvasBoardCardType;
}): string {
  return `<!-- buzz-board-card ${JSON.stringify({
    id: metadata.id,
    type: metadata.type,
    status: metadata.status,
    ...(metadata.threadId ? { thread: metadata.threadId } : {}),
    ...(metadata.author ? { author: metadata.author } : {}),
  })} -->`;
}

export function classifyCanvasBoardCard(title: string): CanvasBoardCardKind {
  const normalizedTitle = title.toLowerCase();

  if (/\b(finished|made|shipped|artifact|showcase)\b/u.test(normalizedTitle)) {
    return "artifact";
  }
  if (
    /\b(help|join|invitation|next|action|participate)\b/u.test(normalizedTitle)
  ) {
    return "invitation";
  }
  if (/\b(people|member|agent|steward|contributor)\b/u.test(normalizedTitle)) {
    return "people";
  }
  if (/\b(now|today|week|current|happening|active)\b/u.test(normalizedTitle)) {
    return "now";
  }
  if (/\b(welcome|start|about|orientation)\b/u.test(normalizedTitle)) {
    return "welcome";
  }
  return "note";
}

export function classifyCanvasBoardCardType(
  title: string,
): CanvasBoardCardType {
  const normalizedTitle = title.toLowerCase();
  if (/\b(agent|bot|berd)\b/u.test(normalizedTitle)) return "agent";
  if (/\b(person|people|member|steward|contributor)\b/u.test(normalizedTitle)) {
    return "person";
  }
  if (/\b(decision|decide|approved|verdict)\b/u.test(normalizedTitle)) {
    return "decision";
  }
  if (/\b(conversation|discussion|thread|work room)\b/u.test(normalizedTitle)) {
    return "conversation";
  }
  if (/\b(project|initiative|program|campaign)\b/u.test(normalizedTitle)) {
    return "project";
  }
  if (/\b(finished|made|shipped|artifact|showcase)\b/u.test(normalizedTitle)) {
    return "artifact";
  }
  if (
    /\b(task|todo|to-do|help|join|next|action|now|today|week|current|active)\b/u.test(
      normalizedTitle,
    )
  ) {
    return "task";
  }
  return "note";
}

function inferredCardStatus(kind: CanvasBoardCardKind): CanvasBoardCardStatus {
  if (kind === "artifact") return "done";
  if (kind === "now") return "doing";
  return "backlog";
}

function parseCanvasBoardSource(content: string): CanvasBoardSource {
  const introductionLines: string[] = [];
  const preambleLines: string[] = [];
  const sections: CanvasBoardSourceSection[] = [];
  let title: string | null = null;
  let activeSection: CanvasBoardSourceSection | null = null;
  let activeFence: { character: "`" | "~"; length: number } | null = null;

  for (const line of content.replace(/\r\n?/gu, "\n").split("\n")) {
    const wasInsideFence = activeFence !== null;
    if (activeFence) {
      const closingFenceMatch = line.match(FENCE_CLOSE_PATTERN);
      if (
        closingFenceMatch &&
        closingFenceMatch[1][0] === activeFence.character &&
        closingFenceMatch[1].length >= activeFence.length
      ) {
        activeFence = null;
      }
    } else {
      const openingFenceMatch = line.match(FENCE_OPEN_PATTERN);
      if (openingFenceMatch) {
        activeFence = {
          character: openingFenceMatch[1][0] as "`" | "~",
          length: openingFenceMatch[1].length,
        };
      }
    }

    const isFenceBoundary = wasInsideFence || activeFence !== null;

    if (!isFenceBoundary && !activeSection) {
      const h1Match = line.match(H1_PATTERN);
      if (h1Match && title === null) {
        title = h1Match[1].trim();
        preambleLines.push(line);
        continue;
      }
    }

    if (!isFenceBoundary) {
      const h2Match = line.match(H2_PATTERN);
      if (h2Match) {
        activeSection = {
          bodyLines: [],
          headingLine: line,
          title: h2Match[1].trim(),
        };
        sections.push(activeSection);
        continue;
      }
    }

    if (activeSection) {
      activeSection.bodyLines.push(line);
    } else {
      preambleLines.push(line);
      introductionLines.push(line);
    }
  }

  return { introductionLines, preambleLines, sections, title };
}

function serializeCanvasBoardSource(source: CanvasBoardSource): string {
  const blocks = [
    source.preambleLines.join("\n").trim(),
    ...source.sections.map((section) => {
      const body = section.bodyLines.join("\n").trim();
      return body ? `${section.headingLine}\n\n${body}` : section.headingLine;
    }),
  ].filter((block) => block.length > 0);

  return blocks.length > 0 ? `${blocks.join("\n\n")}\n` : "";
}

function canvasBoardCardsFromSource(
  source: CanvasBoardSource,
): CanvasBoardCard[] {
  return source.sections.map((section, index) => {
    const parsedSection = parseCanvasBoardSection(section);
    const kind = classifyCanvasBoardCard(section.title);
    return {
      author: parsedSection.metadata.author,
      body: parsedSection.body,
      hasExplicitMetadata: parsedSection.hasExplicitMetadata,
      id: parsedSection.metadata.id ?? cardId(section.title, index),
      kind,
      status: parsedSection.metadata.status ?? inferredCardStatus(kind),
      threadId: parsedSection.metadata.threadId,
      title: section.title,
      type:
        parsedSection.metadata.type ??
        classifyCanvasBoardCardType(section.title),
    };
  });
}

/**
 * Converts a shared Markdown canvas into a title/introduction plus `##` cards.
 * Headings inside fenced code blocks remain body content.
 */
export function parseCanvasBoard(content: string): CanvasBoard {
  const source = parseCanvasBoardSource(content);
  const introduction = source.introductionLines.join("\n").trim();
  const cards = canvasBoardCardsFromSource(source);

  if (cards.length === 0 && introduction.length > 0) {
    cards.push({
      author: null,
      body: introduction,
      hasExplicitMetadata: false,
      id: "overview-1",
      kind: "welcome",
      status: "backlog",
      threadId: null,
      title: "Overview",
      type: "note",
    });
  }

  return {
    cards,
    introduction: source.sections.length > 0 ? introduction : "",
    title: source.title,
  };
}

export function validateCanvasBoardCardDraft(
  draft: CanvasBoardCardDraft,
): string | null {
  const title = draft.title.trim();
  if (!title) {
    return "Add a card title.";
  }
  if (/\r|\n/u.test(title)) {
    return "Keep the card title on one line.";
  }
  if (title.length > 120) {
    return "Keep the card title to 120 characters or fewer.";
  }

  const preview = parseCanvasBoard(`## ${title}\n\n${draft.body}`);
  if (preview.cards.length !== 1) {
    return "Use level-three headings inside a card. Level-two headings create separate cards.";
  }

  return null;
}

export function appendCanvasBoardCard(
  content: string,
  draft: CanvasBoardCardDraft,
): string {
  const source = parseCanvasBoardSource(content);
  const type = draft.type ?? classifyCanvasBoardCardType(draft.title);
  const status = draft.status ?? "backlog";
  const id = draft.id ?? cardId(draft.title, source.sections.length);
  source.sections.push({
    bodyLines: [
      serializeCardMetadata({
        author: draft.author,
        id,
        status,
        threadId: draft.threadId,
        type,
      }),
      "",
      ...draft.body.trim().split("\n"),
    ],
    headingLine: `## ${draft.title.trim()}`,
    title: draft.title.trim(),
  });
  return serializeCanvasBoardSource(source);
}

export function updateCanvasBoardCard(
  content: string,
  cardIdToUpdate: string,
  draft: CanvasBoardCardDraft,
): string | null {
  const source = parseCanvasBoardSource(content);
  if (
    source.sections.length === 0 &&
    cardIdToUpdate === "overview-1" &&
    source.introductionLines.join("\n").trim().length > 0
  ) {
    source.preambleLines = source.preambleLines.filter((line) =>
      H1_PATTERN.test(line),
    );
    source.introductionLines = [];
    source.sections.push({
      bodyLines: [
        serializeCardMetadata({
          author: draft.author,
          id: draft.id ?? cardId(draft.title, 0),
          status: draft.status ?? "backlog",
          threadId: draft.threadId,
          type: draft.type ?? classifyCanvasBoardCardType(draft.title),
        }),
        "",
        ...draft.body.trim().split("\n"),
      ],
      headingLine: `## ${draft.title.trim()}`,
      title: draft.title.trim(),
    });
    return serializeCanvasBoardSource(source);
  }

  const cards = canvasBoardCardsFromSource(source);
  const sectionIndex = cards.findIndex((card) => card.id === cardIdToUpdate);
  const section = source.sections[sectionIndex];
  if (!section) {
    return null;
  }

  const currentCard = cards[sectionIndex];
  section.bodyLines = [
    serializeCardMetadata({
      author: draft.author ?? currentCard.author,
      id: draft.id ?? currentCard.id,
      status: draft.status ?? currentCard.status,
      threadId:
        draft.threadId === undefined ? currentCard.threadId : draft.threadId,
      type: draft.type ?? currentCard.type,
    }),
    "",
    ...draft.body.trim().split("\n"),
  ];
  section.headingLine = `## ${draft.title.trim()}`;
  section.title = draft.title.trim();
  return serializeCanvasBoardSource(source);
}

export function updateCanvasBoardCardMetadata(
  content: string,
  cardIdToUpdate: string,
  patch: Partial<{
    author: string | null;
    status: CanvasBoardCardStatus;
    threadId: string | null;
    type: CanvasBoardCardType;
  }>,
): string | null {
  const source = parseCanvasBoardSource(content);
  const cards = canvasBoardCardsFromSource(source);
  const sectionIndex = cards.findIndex((card) => card.id === cardIdToUpdate);
  const section = source.sections[sectionIndex];
  const card = cards[sectionIndex];
  if (!section || !card) {
    return null;
  }

  section.bodyLines = [
    serializeCardMetadata({
      author: patch.author === undefined ? card.author : patch.author,
      id: card.id,
      status: patch.status ?? card.status,
      threadId: patch.threadId === undefined ? card.threadId : patch.threadId,
      type: patch.type ?? card.type,
    }),
    "",
    ...card.body.split("\n"),
  ];
  return serializeCanvasBoardSource(source);
}

export function canvasBoardCardConversationMarker(cardId: string): string {
  return `magic-board-card:${cardId}`;
}

export function buildCanvasBoardCardConversationOpener(
  card: CanvasBoardCard,
  channelName: string,
): string {
  const body = card.body.trim();
  return [
    `## ${card.title}`,
    body,
    `_Conversation attached to the ${channelName} board._`,
  ]
    .filter((part) => part.length > 0)
    .join("\n\n");
}

export function reorderCanvasBoardCard(
  content: string,
  activeCardId: string,
  overCardId: string,
): string | null {
  if (activeCardId === overCardId) {
    return content;
  }

  const source = parseCanvasBoardSource(content);
  const cards = canvasBoardCardsFromSource(source);
  const activeIndex = cards.findIndex((card) => card.id === activeCardId);
  const overIndex = cards.findIndex((card) => card.id === overCardId);
  if (activeIndex === -1 || overIndex === -1) {
    return null;
  }

  const [movedSection] = source.sections.splice(activeIndex, 1);
  source.sections.splice(overIndex, 0, movedSection);
  return serializeCanvasBoardSource(source);
}

export function resolveChannelViewMode(input: {
  channelName: string | null;
  channelType: ChannelType | null;
  explicitView: ChannelViewMode | null;
  hasCanvas: boolean;
  hasRouteTarget: boolean;
}): { boardAvailable: boolean; mode: ChannelViewMode } {
  const isDispatch =
    input.channelName !== null &&
    channelNamesMatch(input.channelName, "Dispatch");
  const boardAvailable =
    input.channelType !== null &&
    input.channelType !== "dm" &&
    (input.hasCanvas || isDispatch);

  if (input.hasRouteTarget || !boardAvailable) {
    return { boardAvailable, mode: "stream" };
  }

  return {
    boardAvailable,
    mode: input.explicitView ?? (isDispatch ? "board" : "stream"),
  };
}

export function channelViewModeStorageKey(channelId: string): string {
  return `${CHANNEL_VIEW_MODE_STORAGE_PREFIX}:${channelId}`;
}

export function readStoredChannelViewMode(
  channelId: string,
): ChannelViewMode | null {
  const value = getStorageItem(channelViewModeStorageKey(channelId));
  return value === "board" || value === "stream" ? value : null;
}

export function writeStoredChannelViewMode(
  channelId: string,
  mode: ChannelViewMode,
): boolean {
  return setStorageItem(channelViewModeStorageKey(channelId), mode);
}
