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

export type ChannelViewMode = "board" | "stream";

const H1_PATTERN = /^#\s+(.+?)\s*#*\s*$/u;
const H2_PATTERN = /^##\s+(.+?)\s*#*\s*$/u;
const FENCE_OPEN_PATTERN = /^ {0,3}(`{3,}|~{3,})/u;
const FENCE_CLOSE_PATTERN = /^ {0,3}(`{3,}|~{3,})[ \t]*$/u;

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

/**
 * Converts a shared Markdown canvas into a title/introduction plus `##` cards.
 * Headings inside fenced code blocks remain body content.
 */
export function parseCanvasBoard(content: string): CanvasBoard {
  const introductionLines: string[] = [];
  const sections: Array<{ title: string; bodyLines: string[] }> = [];
  let title: string | null = null;
  let activeSection: { title: string; bodyLines: string[] } | null = null;
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
        continue;
      }
    }

    if (!isFenceBoundary) {
      const h2Match = line.match(H2_PATTERN);
      if (h2Match) {
        activeSection = { title: h2Match[1].trim(), bodyLines: [] };
        sections.push(activeSection);
        continue;
      }
    }

    if (activeSection) {
      activeSection.bodyLines.push(line);
    } else {
      introductionLines.push(line);
    }
  }

  const introduction = introductionLines.join("\n").trim();
  const cards = sections.map((section, index) => ({
    body: section.bodyLines.join("\n").trim(),
    id: cardId(section.title, index),
    kind: classifyCanvasBoardCard(section.title),
    title: section.title,
  }));

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
    introduction: sections.length > 0 ? introduction : "",
    title,
  };
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
