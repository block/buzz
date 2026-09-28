import type { BestieScratchAddInput } from "./bestieScratchTypes";

/**
 * Natural-language intents from the *user* for Scratch notes.
 * Conservative: "park this", "save to scratch", "scratch: …".
 */
export type BestieUserScratchIntent =
  | { note: BestieScratchAddInput; op: "add" }
  | { op: "remove-match"; title: string };

const PARK_RE =
  /^(?:please\s+)?(?:park\s+(?:this|it)|save\s+(?:this\s+)?to\s+scratch|scratch)\s*[:\-–—]?\s+([\s\S]+)$/i;
const REMOVE_RE =
  /^(?:please\s+)?(?:remove|delete)\s+(?:from\s+)?scratch\s+(?:called\s+|named\s+)?["']?(.+?)["']?\.?$/i;

function trimTrailingPunctuation(value: string): string {
  return value.replace(/[.\s]+$/g, "").trim();
}

/**
 * Extract zero or one Scratch intent from a user message body.
 */
export function parseBestieUserScratchIntent(
  content: string,
): BestieUserScratchIntent | null {
  const trimmed = content.trim();
  if (!trimmed || trimmed.length > 4000) return null;

  const removeMatch = trimmed.match(REMOVE_RE);
  if (removeMatch) {
    const title = trimTrailingPunctuation(removeMatch[1] ?? "");
    if (!title) return null;
    return { op: "remove-match", title };
  }

  const parkMatch = trimmed.match(PARK_RE);
  if (parkMatch) {
    const raw = (parkMatch[1] ?? "").trim();
    if (!raw) return null;
    const newline = raw.indexOf("\n");
    if (newline >= 0) {
      const title = trimTrailingPunctuation(raw.slice(0, newline));
      const body = raw.slice(newline + 1).trimEnd();
      if (!title && !body.trim()) return null;
      return { note: { body, title }, op: "add" };
    }
    return { note: { body: raw, title: "" }, op: "add" };
  }

  return null;
}

export function messageLooksLikeBestieScratchRequest(content: string): boolean {
  return parseBestieUserScratchIntent(content) != null;
}
