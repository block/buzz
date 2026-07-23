import { useQuery } from "@tanstack/react-query";

import { getEventById } from "@/shared/api/tauri";
import type { SentMessageLink } from "./messageLinks";

export function shouldFetchSentMessage(
  messageLink: SentMessageLink | null,
  inlineContent: string | null,
): boolean {
  return messageLink !== null && inlineContent === null;
}

export function resolveSentMessageBody(
  inlineContent: string | null,
  fetchedContent: string | undefined | null,
): string | null {
  if (inlineContent) return inlineContent;
  return fetchedContent ?? null;
}

/**
 * Builds the exact options object `useSentMessageBody` passes to `useQuery`.
 * Extracted so tests can drive the real fetch through query-core's
 * `QueryObserver` (what `useQuery` constructs internally) without a DOM.
 */
export function sentMessageBodyQueryOptions(
  messageLink: SentMessageLink | null,
  inlineContent: string | null,
  fetchEventById: (eventId: string) => Promise<{ content: string }>,
) {
  return {
    queryKey: ["sent-message-body", messageLink?.messageId],
    queryFn: () => fetchEventById(messageLink?.messageId ?? ""),
    enabled: shouldFetchSentMessage(messageLink, inlineContent),
    staleTime: Number.POSITIVE_INFINITY,
  };
}

export function useSentMessageBody(
  messageLink: SentMessageLink | null,
  inlineContent: string | null,
  fetchEventById: (
    eventId: string,
  ) => Promise<{ content: string }> = getEventById,
): string | null {
  const { data } = useQuery(
    sentMessageBodyQueryOptions(messageLink, inlineContent, fetchEventById),
  );

  return resolveSentMessageBody(inlineContent, data?.content);
}
