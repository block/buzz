import * as React from "react";

export function useSearchHighlightProps(
  messageId: string | null | undefined,
  query: string | undefined,
) {
  const searchMatchingMessageIds = React.useMemo(
    () => (messageId ? new Set([messageId]) : undefined),
    [messageId],
  );
  return React.useMemo(
    () => ({
      thread: { searchMessageId: messageId, searchQuery: query },
      timeline: { searchMatchingMessageIds, searchQuery: query },
    }),
    [messageId, query, searchMatchingMessageIds],
  );
}
