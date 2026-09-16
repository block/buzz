export const SENT_FROM_THREAD_TAG = "buzz:sent-from-thread";
const THREAD_ROOT_EXCERPT_MAX_LENGTH = 64;

export type SentFromThreadReference = {
  rootEventId: string;
  rootExcerpt: string | null;
};

export function summarizeThreadRoot(content: string): string | null {
  const withoutControls = Array.from(content, (character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f)
      ? " "
      : character;
  }).join("");
  const normalized = withoutControls
    .replace(/\|\|[^|]*(?:\|(?!\|)[^|]*)*\|\|/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/<?https?:\/\/\S+>?/g, " ")
    .replace(/[`*_~>#|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const characters = Array.from(normalized);
  if (!normalized) return null;
  if (characters.length <= THREAD_ROOT_EXCERPT_MAX_LENGTH) return normalized;
  const clipped = characters
    .slice(0, THREAD_ROOT_EXCERPT_MAX_LENGTH - 1)
    .join("");
  const lastSpace = clipped.lastIndexOf(" ");
  const excerpt = lastSpace > 32 ? clipped.slice(0, lastSpace) : clipped;
  return `${excerpt.trimEnd()}…`;
}

export function buildSentFromThreadTag(
  rootEventId: string,
  rootExcerpt?: string | null,
): string[] {
  const normalizedRootEventId = rootEventId.trim();
  if (!normalizedRootEventId) {
    throw new Error("A thread root event ID is required.");
  }

  const normalizedExcerpt = rootExcerpt?.trim();
  return normalizedExcerpt
    ? [SENT_FROM_THREAD_TAG, normalizedRootEventId, normalizedExcerpt]
    : [SENT_FROM_THREAD_TAG, normalizedRootEventId];
}

export function getSentFromThreadReference(
  tags: readonly (readonly string[])[] | null | undefined,
): SentFromThreadReference | null {
  const tag = tags?.find(
    (candidate) =>
      (candidate.length === 2 || candidate.length === 3) &&
      candidate[0] === SENT_FROM_THREAD_TAG,
  );
  const rootEventId = tag?.[1]?.trim();
  if (!rootEventId) return null;
  return {
    rootEventId,
    rootExcerpt: tag?.[2]?.trim() || null,
  };
}

export function getSentFromThreadRootId(
  tags: readonly (readonly string[])[] | null | undefined,
): string | null {
  return getSentFromThreadReference(tags)?.rootEventId ?? null;
}
