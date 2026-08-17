import type { ChannelType } from "@/shared/api/types";
import { channelNamesMatch } from "@/features/channels/lib/canonicalChannelName";

export type CanvasBoardCardKind =
  | "artifact"
  | "invitation"
  | "note"
  | "now"
  | "people"
  | "welcome";

export type CanvasBoardCard = {
  body: string;
  id: string;
  kind: CanvasBoardCardKind;
  title: string;
};

export type CanvasBoard = {
  cards: CanvasBoardCard[];
  introduction: string;
  title: string | null;
};

export type CanvasBoardCardDraft = {
  body: string;
  title: string;
};

export type ChannelViewMode = "board" | "stream";

const H1_PATTERN = /^#\s+(.+?)\s*#*\s*$/u;
const H2_PATTERN = /^##\s+(.+?)\s*#*\s*$/u;
const FENCE_OPEN_PATTERN = /^ {0,3}(`{3,}|~{3,})/u;
const FENCE_CLOSE_PATTERN = /^ {0,3}(`{3,}|~{3,})[ \t]*$/u;

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
  return source.sections.map((section, index) => ({
    body: section.bodyLines.join("\n").trim(),
    id: cardId(section.title, index),
    kind: classifyCanvasBoardCard(section.title),
    title: section.title,
  }));
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
      body: introduction,
      id: "overview-1",
      kind: "welcome",
      title: "Overview",
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
  source.sections.push({
    bodyLines: draft.body.trim().split("\n"),
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
      bodyLines: draft.body.trim().split("\n"),
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

  section.bodyLines = draft.body.trim().split("\n");
  section.headingLine = `## ${draft.title.trim()}`;
  section.title = draft.title.trim();
  return serializeCanvasBoardSource(source);
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
