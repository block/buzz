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

export function useSentMessageBody(
  messageLink: SentMessageLink | null,
  inlineContent: string | null,
): string | null {
  const enabled = shouldFetchSentMessage(messageLink, inlineContent);
  const { data } = useQuery({
    queryKey: ["sent-message-body", messageLink?.messageId],
    queryFn: () => getEventById(messageLink?.messageId ?? ""),
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
  });

  return resolveSentMessageBody(inlineContent, data?.content);
}
