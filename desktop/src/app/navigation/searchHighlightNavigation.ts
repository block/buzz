export type SearchHighlightNavigation = {
  activationId: string;
  messageId: string;
  query: string;
};

export function createSearchHighlightNavigation(
  messageId: string,
  query: string | undefined,
): SearchHighlightNavigation | undefined {
  const trimmedQuery = query?.trim();
  if (!trimmedQuery) {
    return undefined;
  }

  return {
    activationId: crypto.randomUUID(),
    messageId,
    query: trimmedQuery,
  };
}

export function parseSearchHighlightNavigation(
  value: unknown,
): SearchHighlightNavigation | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const candidate = value as Partial<SearchHighlightNavigation>;
  if (
    typeof candidate.activationId !== "string" ||
    candidate.activationId.length === 0 ||
    typeof candidate.messageId !== "string" ||
    candidate.messageId.length === 0 ||
    typeof candidate.query !== "string" ||
    candidate.query.length === 0
  ) {
    return null;
  }

  return {
    activationId: candidate.activationId,
    messageId: candidate.messageId,
    query: candidate.query,
  };
}
