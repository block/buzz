import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
  playgroundPin,
  type PlaygroundCard,
} from "./types.ts";
import { isAllowedPlaygroundUrl } from "./url.ts";

const PLAYGROUND_FENCE_RE = /```playground(?:[^\n]*)\n([\s\S]*?)```/g;
/** Any markdown fence; used to catch `json` / unlabeled playground payloads. */
const ANY_FENCE_RE = /```(?:[^\n]*)\n([\s\S]*?)```/g;

function asRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

/**
 * Parse one playground card object. Required fields must be present and valid.
 * Optional `stack` / `expires` are copied only when present. Fields are never
 * rewritten.
 */
export function parsePlaygroundCardValue(
  value: unknown,
): PlaygroundCard | null {
  const candidate = asRecord(value);
  if (!candidate) {
    return null;
  }
  if (candidate.hula !== PLAYGROUND_HULA) {
    return null;
  }
  if (candidate.v !== PLAYGROUND_VERSION) {
    return null;
  }
  if (
    typeof candidate.name !== "string" ||
    candidate.name.trim().length === 0
  ) {
    return null;
  }
  if (
    typeof candidate.url !== "string" ||
    !isAllowedPlaygroundUrl(candidate.url)
  ) {
    return null;
  }
  if (candidate.pin != null && typeof candidate.pin !== "string") {
    return null;
  }
  if (typeof candidate.sid !== "string" || candidate.sid.trim().length === 0) {
    return null;
  }

  const card: PlaygroundCard = {
    hula: PLAYGROUND_HULA,
    v: PLAYGROUND_VERSION,
    name: candidate.name,
    url: candidate.url,
    sid: candidate.sid,
  };
  const pin = playgroundPin(
    typeof candidate.pin === "string" ? candidate.pin : "",
  );
  if (pin) {
    card.pin = typeof candidate.pin === "string" ? candidate.pin : pin;
  }

  if (typeof candidate.stack === "string" && candidate.stack.length > 0) {
    card.stack = candidate.stack;
  }
  if (
    typeof candidate.expires === "string" ||
    typeof candidate.expires === "number"
  ) {
    card.expires = candidate.expires;
  }

  return card;
}

export function parsePlaygroundCard(raw: string): PlaygroundCard | null {
  if (typeof raw !== "string") {
    return null;
  }
  try {
    return parsePlaygroundCardValue(JSON.parse(raw.trim()));
  } catch {
    return null;
  }
}

/** First `playground` fence in a message, or a standalone JSON payload. */
export function extractPlaygroundCard(content: string): PlaygroundCard | null {
  if (typeof content !== "string") {
    return null;
  }
  const standalone = parsePlaygroundCard(content);
  if (standalone) {
    return standalone;
  }
  PLAYGROUND_FENCE_RE.lastIndex = 0;
  const playgroundFence = PLAYGROUND_FENCE_RE.exec(content);
  if (playgroundFence?.[1]) {
    const card = parsePlaygroundCard(playgroundFence[1]);
    if (card) {
      return card;
    }
  }
  ANY_FENCE_RE.lastIndex = 0;
  for (const match of content.matchAll(ANY_FENCE_RE)) {
    const card = parsePlaygroundCard(match[1] ?? "");
    if (card) {
      return card;
    }
  }
  return null;
}

export function extractPlaygroundCards(content: string): PlaygroundCard[] {
  if (typeof content !== "string") {
    return [];
  }
  const cards: PlaygroundCard[] = [];
  const seen = new Set<string>();
  const push = (card: PlaygroundCard | null) => {
    if (!card || seen.has(card.sid)) {
      return;
    }
    seen.add(card.sid);
    cards.push(card);
  };

  PLAYGROUND_FENCE_RE.lastIndex = 0;
  for (const match of content.matchAll(PLAYGROUND_FENCE_RE)) {
    push(parsePlaygroundCard(match[1] ?? ""));
  }
  ANY_FENCE_RE.lastIndex = 0;
  for (const match of content.matchAll(ANY_FENCE_RE)) {
    push(parsePlaygroundCard(match[1] ?? ""));
  }
  if (cards.length === 0) {
    push(parsePlaygroundCard(content));
  }
  return cards;
}
