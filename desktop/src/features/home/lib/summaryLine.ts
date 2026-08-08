import { parseDeclaredAsks } from "@/features/attention/lib/declaredAsks";
import { stripMessageNoise } from "@/features/attention/lib/taskExtraction";

const MAX_SUMMARY_LENGTH = 120;

/**
 * Bold or plain TL;DR / TLDR lead marker, optionally inside a blockquote,
 * with an optional colon inside or outside the bold span. Capture group 1
 * is the remainder of the line.
 */
const TLDR_MARKER =
  /^(?:>\s*)*(?:\*\*)?\s*tl;?dr\s*:?\s*(?:\*\*)?\s*:?\s*(.*)$/i;

function truncateOnWordBoundary(text: string): string {
  if (text.length <= MAX_SUMMARY_LENGTH) {
    return text;
  }
  const cut = text.slice(0, MAX_SUMMARY_LENGTH);
  const lastSpace = cut.lastIndexOf(" ");
  return `${cut
    .slice(0, lastSpace > 60 ? lastSpace : MAX_SUMMARY_LENGTH)
    .trimEnd()}…`;
}

/** A code-block-first message summarises from the prose after the block. */
function withoutCodeBlocks(content: string): string {
  return content.replace(/```[\s\S]*?```/g, "\n");
}

function nonEmptyLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/**
 * One line of "what this message said", for the Catch up reading.
 *
 * Tier 0: a declared ask ("Needs <Name>, <type>: …") is the author's own
 * statement of the point — use the first declaration's ask.
 * Tier 1: an authored TL;DR lead line wins over anything derived.
 * Tier 2: first sentence of the first line, else the first line, truncated
 * ~120 chars on a word boundary. Never emits raw markdown syntax; an empty
 * body returns "" and the caller falls back to its existing preview.
 */
export function summaryLineFor(content: string): string {
  const declared = parseDeclaredAsks(content);
  if (declared.length > 0) {
    return truncateOnWordBoundary(declared[0].ask);
  }

  const lines = nonEmptyLines(withoutCodeBlocks(content));
  if (lines.length === 0) {
    return "";
  }

  let lead = lines[0];
  const marker = lead.match(TLDR_MARKER);
  if (marker) {
    const rest = stripMessageNoise(marker[1]);
    if (rest) {
      return truncateOnWordBoundary(rest);
    }
    // Bare "TL;DR" line with the content below it: summarise what follows.
    lead = lines[1] ?? "";
  }

  const cleaned = stripMessageNoise(lead);
  if (!cleaned) {
    return "";
  }
  const sentence = cleaned.split(/(?<=[.?!])\s+/)[0] ?? cleaned;
  return truncateOnWordBoundary(sentence.trim());
}
